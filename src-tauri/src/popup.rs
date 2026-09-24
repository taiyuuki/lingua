//! Popup 窗口管理：创建、固定位置、状态快照与事件广播。

use crate::platform_error;
use crate::state::{AppState, PopupState};
use lingua_core::{AppError, AppResult};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

/// 与前端 popup 样式约定一致的窗口尺寸（逻辑像素）。
pub const POPUP_WIDTH: f64 = 420.0;
pub const POPUP_HEIGHT: f64 = 360.0;
/// Popup 事件通道（前端 `listen("popup://state")`）。
pub const POPUP_EVENT: &str = "popup://state";

/// 置顶状态（工具栏图钉可切换；窗口默认置顶）。
static POPUP_PINNED: AtomicBool = AtomicBool::new(true);

pub fn popup_pinned() -> bool {
    POPUP_PINNED.load(Ordering::Relaxed)
}

pub fn set_popup_pinned(pinned: bool) {
    POPUP_PINNED.store(pinned, Ordering::Relaxed);
}

/// 流式译文的最新累积文本。delta 事件之外再留一份权威快照：
/// 弹窗首次创建时 Vue 监听器尚未挂载，会漏掉早期 delta 事件，
/// 前端挂载完成后拉一次即可追平。
static POPUP_STREAMED: Mutex<String> = Mutex::new(String::new());

pub fn set_streamed(text: &str) {
    *POPUP_STREAMED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = text.to_string();
}

pub fn streamed() -> String {
    POPUP_STREAMED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

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
    .transparent(true) // 圆角：页面根节点 border-radius 裁剪（需 macOSPrivateApi）
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .visible(false) // 定位后再显示，避免闪现在默认位置
    .build()
    .map_err(platform_error)
}

/// 固定位置：主显示器正中（水平垂直居中）。
/// 不跟随光标——光标可能在屏幕边缘导致弹窗越界。
fn position_fixed(app: &AppHandle, window: &WebviewWindow) {
    let Ok(Some(monitor)) = app.primary_monitor() else {
        return;
    };
    let Ok(window_size) = window.outer_size() else {
        return;
    };
    let position = monitor.position();
    let size = monitor.size();
    let x = f64::from(position.x) + (f64::from(size.width) - f64::from(window_size.width)) / 2.0;
    let y = f64::from(position.y) + (f64::from(size.height) - f64::from(window_size.height)) / 2.0;
    if let Err(error) =
        window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
    {
        tracing::warn!(error = %error, "failed to position popup");
    }
}

/// 更新 popup 状态：写快照 → 广播事件 → 定位（仅隐藏→显示时）→ 显示并聚焦。
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
        if matches!(state, PopupState::Loading { .. }) {
            *POPUP_STREAMED
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = String::new();
        }
    }

    let window = ensure_window(app)?;
    // 只在「隐藏 → 显示」时定位；Loading → Result 的状态切换
    // 原地刷新，否则翻译完成时弹窗会再次跳位。
    let already_visible = window.is_visible().map_err(platform_error)?;
    if !already_visible {
        position_fixed(app, &window);
    }
    app.emit(POPUP_EVENT, &state).map_err(platform_error)?;
    window.show().map_err(platform_error)?;
    window.set_focus().map_err(platform_error)?;
    Ok(())
}
