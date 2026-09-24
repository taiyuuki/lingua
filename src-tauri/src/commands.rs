//! Tauri commands —— 前端唯一的后端入口。
//!
//! 约定：
//!
//! * 命令返回值序列化为 camelCase JSON；
//! * 可能失败的命令返回 `Result<T, AppError>`，前端拿到 `{kind, message}`；
//! * 涉及数据库的命令用 `spawn_blocking` 包裹，避免阻塞主线程。

use crate::{POPUP_LABEL, platform_error, state::AppState};
use lingua_core::AppError;
use serde::Serialize;
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

/// 打开（或聚焦已隐藏的）Popup 窗口。
///
/// Phase 0 用于验证多窗口链路；Phase 1 将由全局快捷键触发，
/// 并按选中文本的 `rect` 定位。
#[tauri::command]
pub async fn open_popup(app: tauri::AppHandle) -> Result<(), AppError> {
    if let Some(existing) = app.get_webview_window(POPUP_LABEL) {
        existing.show().map_err(platform_error)?;
        existing.set_focus().map_err(platform_error)?;
        return Ok(());
    }

    tauri::webview::WebviewWindowBuilder::new(
        &app,
        POPUP_LABEL,
        tauri::WebviewUrl::App("popup.html".into()),
    )
    .title("Lingua")
    .inner_size(420.0, 280.0)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .focused(true)
    .build()
    .map_err(platform_error)?;
    Ok(())
}
