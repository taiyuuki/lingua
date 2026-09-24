//! Tauri 应用壳。
//!
//! 这一层很薄：只做「接线」—— 注册 commands、初始化 tracing、
//! 打开数据库并跑 migration、管理窗口。业务逻辑一律下沉到
//! crates/* 中的 application service（Phase 1 起逐步引入）。

mod commands;
mod state;

use lingua_core::AppError;
use lingua_database::Database;
use state::AppState;
use tauri::Manager;

/// Popup 窗口的固定 label，commands 与窗口事件里共用。
pub const POPUP_LABEL: &str = "popup";

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .setup(|app| {
            init_tracing();
            tracing::info!(
                os = std::env::consts::OS,
                arch = std::env::consts::ARCH,
                "lingua starting"
            );

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let database = Database::open(&data_dir.join("lingua.sqlite3"))?;
            let report = database.migrate()?;
            tracing::info!(
                applied = report.applied.len(),
                schema_version = report.current_version,
                "database ready at {}",
                data_dir.display()
            );

            app.manage(AppState::new(database));
            Ok(())
        })
        .on_window_event(|window, event| {
            // Popup 失焦自动隐藏 ——「点击外部隐藏」的第一版实现。
            // Phase 1 接入选区定位后补充 Esc 关闭与动画。
            if window.label() == POPUP_LABEL {
                if let tauri::WindowEvent::Focused(false) = event {
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::database_status,
            commands::open_popup,
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}

/// 初始化 `tracing`：日志级别由 `RUST_LOG` 控制，缺省 `info`。
///
/// 隐私约束：任何 Provider 密钥、用户划词文本、截图都禁止写日志；
/// debug 级别的文本日志只在显式设置 `LINGUA_DEBUG_TEXT=1` 时由调用点自行启用。
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

/// 把任意 `std::error::Error` 收敛为 [`AppError::Platform`]，
/// 供 setup / command 边界使用。
pub(crate) fn platform_error(error: impl std::fmt::Display) -> AppError {
    AppError::Platform(error.to_string())
}
