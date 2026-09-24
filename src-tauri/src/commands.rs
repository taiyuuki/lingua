//! Tauri commands —— 前端唯一的后端入口。
//!
//! 约定：
//!
//! * 命令返回值序列化为 camelCase JSON；
//! * 可能失败的命令返回 `Result<T, AppError>`，前端拿到 `{kind, message}`；
//! * 涉及数据库 / 密钥库的命令用 `spawn_blocking` 包裹，避免阻塞主线程。

use crate::{
    POPUP_LABEL, pipeline, platform_error, popup,
    state::{AppState, PopupState},
};
use lingua_ai::ProviderKind;
use lingua_core::{AppError, Language};
use lingua_database::providers::ProviderRecord;
use lingua_secrets::{KeyringSecretStore, SecretRef, SecretStore};
use serde::{Deserialize, Serialize};
use tauri::Manager;

/// 应用基本信息（Phase 0 用于验证 IPC 链路）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

/// 数据库状态（Phase 0 用于验证「打开 → migration」链路）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStatus {
    pub schema_version: u32,
    pub migrations_total: usize,
}

#[tauri::command]
pub async fn database_status(
    state: tauri::State<'_, AppState>,
) -> Result<DatabaseStatus, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 顺带确保 schema 已是最新（正常流程 setup 阶段已完成，这里幂等）。
        let report = database.migrate()?;
        Ok(DatabaseStatus {
            schema_version: report.current_version,
            migrations_total: lingua_database::MIGRATIONS.len(),
        })
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 读取默认 Provider 配置（Phase 1 单 Provider；多 Provider 在 Phase 2）。
///
/// `api_key` 永远不出后端：只返回 `hasApiKey` 布尔值。
#[tauri::command]
pub async fn get_provider_config(
    state: tauri::State<'_, AppState>,
) -> Result<Option<ProviderConfigDto>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let provider_id = pipeline::read_default_provider_id(&database);
        let Some(record) = database.get_provider(&provider_id)? else {
            return Ok(None);
        };
        let has_api_key = KeyringSecretStore::default()
            .get_secret(&SecretRef::provider(&provider_id))?
            .is_some();
        Ok(Some(ProviderConfigDto {
            id: record.id.clone(),
            name: record.name.clone(),
            kind: record.kind,
            base_url: record.base_url.clone(),
            model: record.model.clone(),
            organization: record.organization.clone(),
            timeout_secs: record.timeout_secs,
            target_language: Language::parse(&pipeline::read_target_language_code(&database)),
            shortcut: pipeline::read_shortcut(&database),
            api_key: None,
            has_api_key,
        }))
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 保存默认 Provider 配置。
///
/// `api_key` 字段语义：`None` = 不修改；`Some("")` = 删除；非空字符串 = 覆盖。
#[tauri::command]
pub async fn save_provider_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: ProviderConfigDto,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let record = ProviderRecord {
            id: config.id.clone(),
            name: config.name,
            kind: config.kind,
            base_url: config.base_url,
            model: config.model,
            organization: config.organization,
            timeout_secs: config.timeout_secs,
            can_chat: true,
            can_translate: true,
            can_vision: false,
            api_key_ref: Some(SecretRef::provider(&config.id).account),
        };
        database.upsert_provider(&record)?;
        database.set_setting(
            pipeline::SETTINGS_DEFAULT_PROVIDER,
            &serde_json::json!(config.id),
        )?;
        database.set_setting(
            pipeline::SETTINGS_TARGET_LANGUAGE,
            &serde_json::json!(config.target_language.code()),
        )?;
        database.set_setting(
            pipeline::SETTINGS_SHORTCUT,
            &serde_json::json!(config.shortcut),
        )?;

        let reference = SecretRef::provider(&config.id);
        let secrets = KeyringSecretStore::default();
        match config.api_key.as_deref() {
            None => {}
            Some("") => {
                secrets.delete_secret(&reference)?;
            }
            Some(key) => {
                secrets.set_secret(&reference, key)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))??;

    // 快捷键可能变了，重新注册。
    crate::register_shortcut_from_settings(&app, &state.database)?;
    Ok(())
}

/// Provider 配置 DTO（前端表单 ↔ 后端）。
///
/// 出参时 `apiKey` 恒为 `None`、`hasApiKey` 指示密钥库中是否有密钥；
/// 入参时 `apiKey` 按 `save_provider_config` 注释中的语义解释。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfigDto {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: Option<String>,
    pub organization: Option<String>,
    pub timeout_secs: u64,
    pub target_language: Language,
    pub shortcut: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub has_api_key: bool,
}

/// 手动打开（或聚焦已隐藏的）Popup 窗口。
/// 主要用于开发与设置页验证；正常入口是全局快捷键触发的 pipeline。
#[tauri::command]
pub async fn open_popup(app: tauri::AppHandle) -> Result<(), AppError> {
    let window = popup::ensure_window(&app)?;
    window.show().map_err(platform_error)?;
    window.set_focus().map_err(platform_error)?;
    Ok(())
}

/// 隐藏 Popup（前端按 Esc 时调用）。
#[tauri::command]
pub async fn hide_popup(app: tauri::AppHandle) -> Result<(), AppError> {
    if let Some(window) = app.get_webview_window(POPUP_LABEL) {
        window.hide().map_err(platform_error)?;
    }
    Ok(())
}

/// Popup 最新状态快照（窗口刚创建时补拉，避免错过事件）。
#[tauri::command]
pub async fn latest_popup_state(app: tauri::AppHandle) -> Result<Option<PopupState>, AppError> {
    let state = app.state::<AppState>();
    let guard = state
        .popup_state
        .lock()
        .map_err(|_| AppError::Internal("popup state lock poisoned".into()))?;
    Ok(guard.clone())
}

/// 划词权限状态（macOS：辅助功能授权）。
#[tauri::command]
pub async fn selection_permission_status() -> Result<bool, AppError> {
    Ok(lingua_platform::selection_permission_granted())
}
