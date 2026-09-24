//! Tauri 应用壳。
//!
//! 这一层很薄：只做「接线」—— 注册 commands / 插件、初始化 tracing、
//! 打开数据库并跑 migration、注册全局快捷键、管理窗口。
//! 业务逻辑在 crates/*（platform / ai / translation / database / secrets）。

mod commands;
mod ocr;
mod pipeline;
mod popup;
mod state;

use lingua_core::AppError;
use lingua_database::Database;
use state::AppState;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

/// Popup 窗口的固定 label，commands 与窗口事件里共用。
pub const POPUP_LABEL: &str = "popup";

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
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

            app.manage(AppState::new(
                database.clone(),
                lingua_platform::create_default_selection_service(),
            ));

            // 全局快捷键：从 settings 读取（默认 alt+t），触发翻译 pipeline。
            register_shortcut_from_settings(app.handle(), &database)?;

            tracing::info!("lingua ready");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Popup 失焦自动隐藏 ——「点击外部隐藏」的实现。
            // Windows 上点击弹窗内容会把焦点转给 WebView2 子窗口并
            // 触发 Focused(false)，由 handle_focus_lost 裁决是否真失焦。
            if window.label() == POPUP_LABEL {
                if let tauri::WindowEvent::Focused(false) = event {
                    popup::handle_focus_lost(window);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::database_status,
            commands::open_popup,
            commands::hide_popup,
            commands::toggle_popup_pin,
            commands::latest_streamed,
            commands::latest_popup_state,
            commands::selection_permission_status,
            commands::list_providers,
            commands::save_provider,
            commands::delete_provider,
            commands::test_provider,
            commands::get_app_settings,
            commands::save_app_settings,
            commands::list_dictionaries,
            commands::import_dictionary_tsv,
            commands::delete_dictionary,
            commands::list_vocabulary,
            commands::update_vocabulary,
            commands::delete_vocabulary,
            commands::vocabulary_tags,
            commands::save_to_vocabulary,
            commands::is_word,
            commands::vocabulary_word_status,
            commands::remove_vocabulary_word,
            commands::start_ocr_capture,
            commands::get_ocr_background,
            commands::perform_ocr_translation,
            commands::cancel_ocr,
            commands::screen_recording_status,
            commands::ocr_retry_available,
            commands::retry_ocr_with_vision,
            commands::ocr_source_info,
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}

/// 重新注册全局快捷键（设置变更时也会调用）：
/// 划词翻译（默认 `alt+t`）与截图翻译（默认 `alt+s`，跟随光标所在
/// 显示器，多屏截图的主要入口）。两者相同（或截图快捷键非法）时
/// 截图快捷键让路，只告警不失败——划词是核心路径。
pub fn register_shortcut_from_settings(
    app: &tauri::AppHandle,
    database: &Database,
) -> lingua_core::AppResult<()> {
    let global_shortcut = app.global_shortcut();
    if let Err(error) = global_shortcut.unregister_all() {
        tracing::debug!(error = %error, "unregister_all reported an error (may be none registered)");
    }

    let translate_shortcut = pipeline::read_shortcut(database);
    register_shortcut(app, &translate_shortcut, translate_shortcut_handler)?;

    let ocr_shortcut = pipeline::read_ocr_shortcut(database);
    if ocr_shortcut == translate_shortcut {
        tracing::warn!(
            shortcut = %ocr_shortcut,
            "ocr shortcut conflicts with translate shortcut; ignoring"
        );
        return Ok(());
    }
    register_shortcut(app, &ocr_shortcut, ocr_shortcut_handler)
}

/// 解析并注册单个快捷键。
fn register_shortcut<F>(
    app: &tauri::AppHandle,
    shortcut_string: &str,
    handler: F,
) -> lingua_core::AppResult<()>
where
    F: Fn(&tauri::AppHandle, &Shortcut, ShortcutEvent) + Send + Sync + 'static,
{
    let shortcut: Shortcut = shortcut_string.parse().map_err(|error| {
        lingua_core::AppError::InvalidInput(format!(
            "invalid shortcut `{shortcut_string}`: {error}"
        ))
    })?;
    app.global_shortcut()
        .on_shortcut(shortcut, handler)
        .map_err(|error| lingua_core::AppError::Platform(error.to_string()))?;
    tracing::info!(shortcut = shortcut_string, "global shortcut registered");
    Ok(())
}

/// 划词快捷键：按下即触发一次完整翻译 pipeline。
fn translate_shortcut_handler(app: &tauri::AppHandle, _shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state == ShortcutState::Pressed {
        tauri::async_runtime::spawn(pipeline::run_translation_pipeline(app.clone()));
    }
}

/// 截图快捷键：按下即截光标所在显示器并显示选区 overlay。
fn ocr_shortcut_handler(app: &tauri::AppHandle, _shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state == ShortcutState::Pressed {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = ocr::start_capture(&app).await {
                tracing::warn!(error = %error, "ocr capture failed to start");
            }
        });
    }
}

/// 把任意 `std::error::Error` 收敛为 [`AppError::Platform`]，
/// 供 setup / command 边界使用。
pub(crate) fn platform_error(error: impl std::fmt::Display) -> AppError {
    AppError::Platform(error.to_string())
}

/// 初始化 `tracing`：日志级别由 `RUST_LOG` 控制，缺省 `info`。
///
/// 隐私约束：任何 Provider 密钥、用户划词文本、截图都禁止写日志；
/// 选区只允许记录字符数，debug 级别的文本日志由调用点显式开启。
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
