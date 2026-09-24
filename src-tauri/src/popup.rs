//! Popup 窗口管理：创建、按光标定位、状态快照与事件广播。

use crate::platform_error;
use crate::state::{AppState, PopupState};
use lingua_core::{AppError, AppResult};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

/// 与前端 popup 样式约定一致的窗口尺寸（逻辑像素）。
pub const POPUP_WIDTH: f64 = 420.0;
pub const POPUP_HEIGHT: f64 = 280.0;
/// Popup 事件通道（前端 `listen("popup://state")`）。
pub const POPUP_EVENT: &str = "popup://state";

/// 获取或创建（隐藏的）popup 窗口。
pub fn ensure_window(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(existing) = app.get_webview_window(crate::POPUP_LABEL) {
        return Ok(existing);
    }
    tauri::webview::WebviewWindowBuilder::new(
        app,
        crate::POPUP_LABEL,
        tauri::WebviewUrl::App("popup.html".into()),
    )
    .title("Lingua")
    .inner_size(POPUP_WIDTH, POPUP_HEIGHT)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .visible(false) // 定位后再显示，避免闪现在默认位置
    .build()
    .map_err(platform_error)
}

/// 把 popup 定位到光标附近：水平居中、垂直略下方（简单防遮挡）。
/// 多显示器越界钳制留到 Phase 10。
fn position_near_cursor(window: &WebviewWindow) {
    let Some((x, y)) = lingua_platform::cursor_position() else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let px = ((x * scale) - (POPUP_WIDTH * scale) / 2.0).round() as i32;
    let py = ((y * scale) + 16.0 * scale).round() as i32;
    if let Err(error) = window.set_position(PhysicalPosition::new(px, py)) {
        tracing::warn!(error = %error, "failed to position popup");
    }
}

/// 更新 popup 状态：写快照 → 广播事件 → 定位 → 显示并聚焦。
///
/// 聚焦 popup 是有意为之：Esc 关闭与失焦隐藏都依赖焦点；
/// 代价是暂时抢走目标应用焦点（与 Bob 等同类工具一致）。
pub async fn show(app: &AppHandle, state: PopupState) -> AppResult<()> {
    {
        let app_state = app.state::<AppState>();
        let mut guard = app_state
            .popup_state
            .lock()
            .map_err(|_| AppError::Internal("popup state lock poisoned".into()))?;
        *guard = Some(state.clone());
    }

    let window = ensure_window(app)?;
    position_near_cursor(&window);
    app.emit(POPUP_EVENT, &state).map_err(platform_error)?;
    window.show().map_err(platform_error)?;
    window.set_focus().map_err(platform_error)?;
    Ok(())
}
