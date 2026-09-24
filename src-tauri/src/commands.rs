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
use lingua_ai::{AIProvider, ProviderKind};
use lingua_core::{AppError, AppResult, Language};
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

// ---------- Provider 管理（Phase 2） ----------

/// Provider 配置 DTO（前端表单 ↔ 后端）。
///
/// 出参时 `apiKey` 恒为 `None`、`hasApiKey` 指示密钥库中是否有密钥；
/// 入参时 `apiKey` 按 [`save_provider`] 注释中的语义解释。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDto {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: Option<String>,
    pub organization: Option<String>,
    pub timeout_secs: u64,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub has_api_key: bool,
}

impl ProviderDto {
    fn from_record(record: ProviderRecord, has_api_key: bool) -> Self {
        Self {
            id: record.id,
            name: record.name,
            kind: record.kind,
            base_url: record.base_url,
            model: record.model,
            organization: record.organization,
            timeout_secs: record.timeout_secs,
            api_key: None,
            has_api_key,
        }
    }

    fn to_record(&self) -> ProviderRecord {
        ProviderRecord {
            id: self.id.trim().to_string(),
            name: self.name.trim().to_string(),
            kind: self.kind,
            base_url: self.base_url.trim().to_string(),
            model: self
                .model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            organization: self.organization.clone(),
            timeout_secs: self.timeout_secs,
            can_chat: true,
            can_translate: true,
            can_vision: false,
            api_key_ref: Some(SecretRef::provider(self.id.trim()).account),
        }
    }
}

/// 列出全部 Provider（含每个的密钥存在状态），按创建顺序。
#[tauri::command]
pub async fn list_providers(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProviderDto>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let secrets = KeyringSecretStore::default();
        database
            .list_providers()?
            .into_iter()
            .map(|record| {
                let has_api_key = secrets
                    .get_secret(&SecretRef::provider(&record.id))?
                    .is_some();
                Ok(ProviderDto::from_record(record, has_api_key))
            })
            .collect()
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 插入或更新一个 Provider。
///
/// `api_key` 字段语义：`None` = 不修改；`Some("")` = 删除；非空字符串 = 覆盖。
/// 保存后若当前没有默认 Provider，则把该 Provider 设为默认。
#[tauri::command]
pub async fn save_provider(
    state: tauri::State<'_, AppState>,
    provider: ProviderDto,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let record = provider.to_record();
        if record.id.is_empty() || record.base_url.is_empty() {
            return Err(AppError::InvalidInput(
                "provider id and base_url are required".to_string(),
            ));
        }
        database.upsert_provider(&record)?;

        let no_usable_default = match pipeline::read_default_provider_id(&database) {
            Some(id) => database.get_provider(&id)?.is_none(),
            None => true,
        };
        if no_usable_default {
            database.set_setting(
                pipeline::SETTINGS_DEFAULT_PROVIDER,
                &serde_json::json!(record.id),
            )?;
        }

        let reference = SecretRef::provider(&record.id);
        let secrets = KeyringSecretStore::default();
        match provider.api_key.as_deref() {
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
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 删除 Provider（连同其密钥），并清理指向它的默认 / 路由设置。
#[tauri::command]
pub async fn delete_provider(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if database.delete_provider(&id)? {
            KeyringSecretStore::default().delete_secret(&SecretRef::provider(&id))?;
        }
        if pipeline::read_default_provider_id(&database).as_deref() == Some(id.as_str()) {
            database.remove_setting(pipeline::SETTINGS_DEFAULT_PROVIDER)?;
        }
        if pipeline::read_route_provider(&database, pipeline::SETTINGS_ROUTE_TRANSLATE).as_deref()
            == Some(id.as_str())
        {
            database.remove_setting(pipeline::SETTINGS_ROUTE_TRANSLATE)?;
        }
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 连接测试结果。测试失败不算命令失败，结构化返回给前端展示。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestProviderResult {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub reply: Option<String>,
    pub error: Option<String>,
}

/// 连接测试：向该 Provider 发一个极小的对话请求。
#[tauri::command]
pub async fn test_provider(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<TestProviderResult, AppError> {
    let database = state.database.clone();
    // 阻塞部分（DB + 钥匙串）先在阻塞线程里完成。
    let provider = tauri::async_runtime::spawn_blocking(move || -> AppResult<_> {
        let record = database
            .get_provider(&id)?
            .ok_or_else(|| AppError::InvalidInput(format!("provider `{id}` does not exist")))?;
        let mut config = record.to_config();
        config.api_key = KeyringSecretStore::default().get_secret(&SecretRef::provider(&id))?;
        lingua_ai::OpenAiCompatibleProvider::new(config)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))??;

    let request = lingua_ai::ChatRequest {
        messages: vec![
            lingua_ai::ChatMessage::system("You are a connection health check."),
            lingua_ai::ChatMessage::user("Reply with the single word: pong"),
        ],
        model: None,
        temperature: Some(0.0),
        max_tokens: Some(16),
    };

    let start = std::time::Instant::now();
    match provider.chat(&request).await {
        Ok(response) => Ok(TestProviderResult {
            ok: true,
            latency_ms: Some(start.elapsed().as_millis() as u64),
            reply: Some(response.content.chars().take(60).collect()),
            error: None,
        }),
        Err(error) => Ok(TestProviderResult {
            ok: false,
            latency_ms: Some(start.elapsed().as_millis() as u64),
            reply: None,
            error: Some(error.to_string()),
        }),
    }
}

// ---------- 应用设置（路由 + 快捷键） ----------

/// 应用设置：AI Router 路由 + 全局快捷键。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsDto {
    /// 默认 Provider（`None` = 未设置，路由未命中时报「未配置」）。
    pub default_provider_id: Option<String>,
    /// 翻译任务路由（`None` = 跟随默认 Provider）。
    pub translate_provider_id: Option<String>,
    /// 划词翻译的目标语言（BCP-47 基础码）。
    pub target_language: Language,
    pub shortcut: String,
}

#[tauri::command]
pub async fn get_app_settings(
    state: tauri::State<'_, AppState>,
) -> Result<AppSettingsDto, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(AppSettingsDto {
            default_provider_id: pipeline::read_default_provider_id(&database),
            translate_provider_id: pipeline::read_route_provider(
                &database,
                pipeline::SETTINGS_ROUTE_TRANSLATE,
            ),
            target_language: Language::parse(&pipeline::read_target_language_code(&database)),
            shortcut: pipeline::read_shortcut(&database),
        })
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 保存应用设置；快捷键变更会立即重新注册。
#[tauri::command]
pub async fn save_app_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    settings: AppSettingsDto,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        match settings.default_provider_id.as_deref() {
            None => {
                database.remove_setting(pipeline::SETTINGS_DEFAULT_PROVIDER)?;
            }
            Some(id) => {
                database.set_setting(pipeline::SETTINGS_DEFAULT_PROVIDER, &serde_json::json!(id))?
            }
        }
        match settings.translate_provider_id.as_deref() {
            None => {
                database.remove_setting(pipeline::SETTINGS_ROUTE_TRANSLATE)?;
            }
            Some(id) => {
                database.set_setting(pipeline::SETTINGS_ROUTE_TRANSLATE, &serde_json::json!(id))?
            }
        }
        database.set_setting(
            pipeline::SETTINGS_TARGET_LANGUAGE,
            &serde_json::json!(settings.target_language.code()),
        )?;
        database.set_setting(
            pipeline::SETTINGS_SHORTCUT,
            &serde_json::json!(settings.shortcut),
        )?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))??;

    crate::register_shortcut_from_settings(&app, &state.database)?;
    Ok(())
}

// ---------- Popup / 权限 ----------

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
