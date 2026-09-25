//! Tauri 应用壳。
//!
//! 这一层很薄：只做「接线」—— 注册 commands / 插件、初始化 tracing、
//! 打开数据库并跑 migration、注册全局快捷键、管理窗口。
//! 业务逻辑在 crates/*（platform / ai / translation / database / secrets）。

mod commands;
mod ocr;
mod pipeline;
mod popup;
mod speex;
mod state;

use lingua_core::AppError;
use lingua_database::Database;
use state::AppState;
use tauri::Manager;
use tauri::menu::{CheckMenuItem, MenuBuilder, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

/// Popup 窗口的固定 label，commands 与窗口事件里共用。
pub const POPUP_LABEL: &str = "popup";
/// 主窗口的固定 label（与 tauri.conf.json 一致）。
pub const MAIN_LABEL: &str = "main";

/// 托盘状态：开机自启勾选项（设置页开关与托盘菜单双向同步用）。
pub struct TrayState {
    pub autostart_item: std::sync::Mutex<CheckMenuItem<tauri::Wry>>,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .register_uri_scheme_protocol("mdx-res", handle_mdx_resource)
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
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

            setup_tray(app)?;

            tracing::info!("lingua ready");
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭主窗口 = 隐藏到托盘（划词 / 截图 / Popup 继续可用）；
            // 退出走托盘菜单。
            if window.label() == MAIN_LABEL {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
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
            commands::import_dictionary_mdx,
            commands::cancel_dictionary_import,
            commands::dictionary_lookup,
            commands::lookup_word_dictionaries,
            commands::reorder_dictionaries,
            commands::delete_dictionary,
            commands::list_vocabulary,
            commands::vocabulary_count,
            commands::vocabulary_batch_delete,
            commands::vocabulary_batch_update,
            commands::grade_vocabulary_review,
            commands::due_vocabulary_count,
            commands::list_due_vocabulary,
            commands::vocabulary_language_counts,
            commands::update_vocabulary,
            commands::delete_vocabulary,
            commands::vocabulary_tags,
            commands::save_to_vocabulary,
            commands::is_word,
            commands::vocabulary_word_status,
            commands::remove_vocabulary_word,
            commands::list_history,
            commands::history_count,
            commands::delete_history,
            commands::clear_history,
            commands::start_ocr_capture,
            commands::get_ocr_background,
            commands::perform_ocr_translation,
            commands::cancel_ocr,
            commands::screen_recording_status,
            commands::ocr_retry_available,
            commands::retry_ocr_with_vision,
            commands::ocr_source_info,
            commands::autostart_enabled,
            commands::set_autostart,
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}

/// `mdx-res://localhost/<词典id>/<资源路径>` —— MDX 词典富文本的
/// 资源服务（MDD 解包产物，位于 app_data/mdx/<id>/）。释义 HTML 里
/// 的 CSS / 图片链接在导入净化时已改写到本协议。
fn handle_mdx_resource(
    context: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let app = context.app_handle();
    let not_found = || {
        tauri::http::Response::builder()
            .status(404)
            .body(Vec::new())
            .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
    };
    // host 固定 localhost；词典 id 与资源路径都在 path 里（已百分号编码）。
    let raw_path = request.uri().path().trim_start_matches('/');
    let Some((raw_id, raw_resource)) = raw_path.split_once('/') else {
        return not_found();
    };
    let dictionary_id = percent_decode(raw_id);
    let resource = percent_decode(raw_resource);
    let Some(resource) = lingua_dictionary::html_sanitize::normalize_resource_path(&resource)
    else {
        return not_found();
    };
    let Ok(data_dir) = app.path().app_data_dir() else {
        return not_found();
    };
    let full_path = data_dir.join("mdx").join(dictionary_id).join(&resource);
    match std::fs::read(&full_path) {
        Ok(bytes) => {
            // .spx（Ogg/Speex）：浏览器不支持，就地转码 WAV。
            let (bytes, content_type) = if resource
                .rsplit('.')
                .next()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("spx"))
            {
                match speex::speex_to_wav(&bytes) {
                    Some(wav) => (wav, "audio/wav"),
                    None => {
                        tracing::warn!(resource = %resource, "speex decode failed");
                        return not_found();
                    }
                }
            } else {
                (bytes, resource_content_type(&resource))
            };
            tauri::http::Response::builder()
                .header("Content-Type", content_type)
                .header("Cache-Control", "max-age=86400")
                .body(bytes)
                .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
        }
        Err(_) => not_found(),
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""),
                16,
            ) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn resource_content_type(path: &str) -> &'static str {
    let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "css" => "text/css",
        "js" => "text/javascript",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}
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

/// 建托盘：常驻入口（显示主窗口 / 截图翻译 / 开机自启 / 退出）。
/// 左键单击 = 显示主窗口；菜单里退出是唯一退出路径
/// （关闭主窗口只是隐藏到托盘）。
fn setup_tray(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        "开机自启",
        true,
        app.autolaunch().is_enabled().unwrap_or(false),
        None::<&str>,
    )?;
    let menu = MenuBuilder::new(app)
        .item(&MenuItem::with_id(
            app,
            "show",
            "显示主窗口",
            true,
            None::<&str>,
        )?)
        .item(&MenuItem::with_id(
            app,
            "capture",
            "截图翻译",
            true,
            None::<&str>,
        )?)
        .separator()
        .item(&autostart_item)
        .separator()
        .item(&MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?)
        .build()?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| AppError::Platform("no default window icon embedded".to_string()))?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Lingua")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "capture" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = ocr::start_capture(&app).await {
                        tracing::warn!(error = %error, "tray capture failed to start");
                    }
                });
            }
            // CheckMenuItem 被点击时自行翻转勾选；以翻转后的状态为准
            // 同步到 autostart 插件（设置页开关走 set_autostart 反向同步）。
            "autostart" => {
                let checked = app
                    .try_state::<TrayState>()
                    .and_then(|state| {
                        state
                            .autostart_item
                            .lock()
                            .ok()
                            .and_then(|item| item.is_checked().ok())
                    })
                    .unwrap_or(false);
                if let Err(error) = apply_autostart(app, checked) {
                    tracing::warn!(error = %error, "failed to toggle autostart from tray");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(TrayState {
        autostart_item: std::sync::Mutex::new(autostart_item),
    });
    Ok(())
}

/// 显示并聚焦主窗口（托盘左键 / 菜单共用）。
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// 把开机自启状态写入 autostart 插件（注册表 / LaunchAgent）。
fn apply_autostart(app: &tauri::AppHandle, enabled: bool) -> lingua_core::AppResult<()> {
    let autostart = app.autolaunch();
    let result = if enabled {
        autostart.enable()
    } else {
        autostart.disable()
    };
    result.map_err(|error| AppError::Platform(error.to_string()))
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
