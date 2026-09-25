//! Popup 窗口管理：创建、固定位置、状态快照与事件广播。

use crate::platform_error;
use crate::state::{AppState, PopupState};
use lingua_core::{AppError, AppResult};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

/// 与前端 popup 样式约定一致的窗口尺寸（逻辑像素）。
pub const POPUP_WIDTH: f64 = 560.0;
pub const POPUP_HEIGHT: f64 = 480.0;
/// Popup 事件通道（前端 `listen("popup://state")`）。
pub const POPUP_EVENT: &str = "popup://state";

/// 图钉固定状态（工具栏图钉可切换）：固定时点击外部不隐藏，
/// Esc / 重新翻译仍然生效。窗口本身始终置顶，与图钉解耦。
static POPUP_PINNED: AtomicBool = AtomicBool::new(false);

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

// MARK: - 失焦隐藏裁决

/// 失焦监视中标记：保证同一时刻至多一个轮询 watcher。
static FOCUS_WATCHING: AtomicBool = AtomicBool::new(false);

/// 焦点是否仍在本窗口树内（Windows：点击弹窗内容会把真实焦点
/// 转给 WebView2 子窗口，触发主窗口的 Focused(false)，不是失焦）。
#[cfg(target_os = "windows")]
fn focus_inside_own_window_tree(window: &tauri::Window) -> bool {
    window
        .hwnd()
        .map(|hwnd| lingua_platform::window_in_foreground(hwnd.0 as isize))
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn focus_inside_own_window_tree(_window: &tauri::Window) -> bool {
    false
}

/// Popup 收到 `Focused(false)` 时的裁决：
/// * 前台根窗口已不是 popup → 真失焦（点击外部），立即隐藏；
/// * 焦点仍在 popup 窗口树内 → 不隐藏；但父窗口不会再次收到
///   失焦事件，改由 watcher 轮询盯到真正失焦再隐藏。
pub fn handle_focus_lost(window: &tauri::Window) {
    // 图钉固定：点击外部不隐藏（Esc / 重新翻译仍生效）。
    if popup_pinned() {
        return;
    }
    if focus_inside_own_window_tree(window) {
        arm_focus_watcher(window.clone());
    } else {
        let _ = window.hide();
    }
}

/// 每 150ms 检查前台是否已离开 popup 窗口树；popup 隐藏后自动退出。
/// 监视期间图钉被点亮则直接退出（不隐藏）。
fn arm_focus_watcher(window: tauri::Window) {
    if FOCUS_WATCHING.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tracing::debug!("popup focus watcher armed");
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            if !window.is_visible().unwrap_or(false) || popup_pinned() {
                break;
            }
            if !focus_inside_own_window_tree(&window) {
                let _ = window.hide();
                break;
            }
        }
        FOCUS_WATCHING.store(false, Ordering::Release);
    });
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
    .resizable(true) // 可临时调节；每次重新弹出时重置为默认尺寸
    .visible(false) // 定位后再显示，避免闪现在默认位置
    .build()
    .map_err(platform_error)
}

/// 光标所在的显示器；取不到光标或命中失败时回退主显示器。
pub(crate) fn cursor_monitor(app: &AppHandle) -> Option<tauri::Monitor> {
    if let Ok(cursor) = app.cursor_position() {
        if let Ok(Some(monitor)) = app.monitor_from_point(cursor.x, cursor.y) {
            return Some(monitor);
        }
    }
    app.primary_monitor().ok().flatten()
}

/// 在显示器内居中，并钳制不越出边界（弹出窗比显示器大时贴左上角）。
fn center_clamped(
    origin: (f64, f64),
    monitor_size: (f64, f64),
    popup_size: (f64, f64),
) -> (f64, f64) {
    let max_x = (origin.0 + monitor_size.0 - popup_size.0).max(origin.0);
    let max_y = (origin.1 + monitor_size.1 - popup_size.1).max(origin.1);
    (
        (origin.0 + (monitor_size.0 - popup_size.0) / 2.0).clamp(origin.0, max_x),
        (origin.1 + (monitor_size.1 - popup_size.1) / 2.0).clamp(origin.1, max_y),
    )
}

/// 定位：光标所在显示器内居中。多显示器下弹窗跟着光标走，
/// 不再固定出现在主显示器；尺寸做边界钳制，不会越出屏幕。
fn position_popup(app: &AppHandle, window: &WebviewWindow) {
    let Some(monitor) = cursor_monitor(app) else {
        return;
    };
    let Ok(window_size) = window.outer_size() else {
        return;
    };
    let (x, y) = center_clamped(
        (
            f64::from(monitor.position().x),
            f64::from(monitor.position().y),
        ),
        (
            f64::from(monitor.size().width),
            f64::from(monitor.size().height),
        ),
        (f64::from(window_size.width), f64::from(window_size.height)),
    );
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
    // 每次新的翻译触发（Loading 态）或从隐藏恢复时重置并定位：
    // 用户拖出的尺寸不持久化，恢复默认后按默认尺寸重新居中；
    // Loading → Result 的原地刷新不重定位，避免翻译完成时跳位。
    // 多显示器下重复触发会让弹窗跟随光标所在的屏幕。
    let already_visible = window.is_visible().map_err(platform_error)?;
    if !already_visible || matches!(state, PopupState::Loading { .. }) {
        if let Err(error) =
            window.set_size(tauri::LogicalSize::new(POPUP_WIDTH, POPUP_HEIGHT))
        {
            tracing::warn!(error = %error, "failed to reset popup size");
        }
        position_popup(app, &window);
    }
    app.emit(POPUP_EVENT, &state).map_err(platform_error)?;
    window.show().map_err(platform_error)?;
    window.set_focus().map_err(platform_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn centers_within_monitor() {
        let (x, y) = center_clamped((0.0, 0.0), (1920.0, 1080.0), (630.0, 540.0));
        assert_eq!((x, y), (645.0, 270.0));
    }

    #[test]
    fn clamps_to_top_left_when_popup_exceeds_monitor() {
        // 弹窗比显示器大：贴左上角，不出负坐标。
        let (x, y) = center_clamped((0.0, 0.0), (500.0, 400.0), (630.0, 540.0));
        assert_eq!((x, y), (0.0, 0.0));
    }

    #[test]
    fn supports_monitor_left_of_primary() {
        // 主显示器左侧的副屏：桌面坐标为负。
        let (x, y) = center_clamped((-1920.0, 0.0), (1920.0, 1080.0), (630.0, 540.0));
        assert_eq!((x, y), (-1275.0, 270.0));
    }
}
