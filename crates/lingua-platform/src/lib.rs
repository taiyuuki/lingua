//! 平台抽象层。
//!
//! 目标：业务代码只见 trait，不见平台 API。所有 macOS / Windows /
//! Linux 的特殊逻辑都收敛在本 crate 的平台实现模块中，
//! 通过 `cfg(target_os = ...)` 隔离，绝不出现在 src-tauri 与前端里。

pub mod capture;
pub mod os;
pub mod selection;
pub mod selection_service;
pub mod unsupported;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

pub use capture::{
    capture_region, crop_image, request_screen_recording_access, screen_recording_granted,
};
pub use os::{Os, current_os, open_accessibility_settings};
pub use selection::SelectionProvider;
pub use selection_service::SelectionService;
pub use unsupported::UnsupportedSelectionProvider;

#[cfg(target_os = "macos")]
pub use macos::{AccessibilitySelectionProvider, ClipboardSelectionProvider};

#[cfg(target_os = "windows")]
pub use windows::{ClipboardSelectionProvider, UiaSelectionProvider};

/// 组装默认划词服务：首选系统选区 API（macOS Accessibility /
/// Windows UI Automation），剪贴板模拟复制快捷键兜底。
///
/// Linux 的真实实现（AT-SPI2）在接入前只有
/// `unsupported-platform` 占位 Provider。
pub fn create_default_selection_service() -> SelectionService {
    #[cfg(target_os = "macos")]
    return SelectionService::new(vec![
        Box::new(AccessibilitySelectionProvider),
        Box::new(ClipboardSelectionProvider::default()),
    ]);

    #[cfg(target_os = "windows")]
    return SelectionService::new(vec![
        Box::new(UiaSelectionProvider),
        Box::new(ClipboardSelectionProvider::default()),
    ]);

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    SelectionService::new(vec![Box::new(UnsupportedSelectionProvider)])
}

/// 划词所需权限是否已授予（macOS：辅助功能；其它平台无需授权）。
pub fn selection_permission_granted() -> bool {
    #[cfg(target_os = "macos")]
    return macos::ffi::is_accessibility_trusted();

    #[cfg(not(target_os = "macos"))]
    true
}

/// 请求划词权限（macOS：prompt=true 注册并引导；其它平台无操作）。
/// 返回请求后的授权状态。
pub fn request_selection_permission() -> bool {
    #[cfg(target_os = "macos")]
    return macos::ffi::request_accessibility_access();

    #[cfg(not(target_os = "macos"))]
    true
}

/// 当前光标全局坐标（逻辑点，左上角原点）；Popup 定位用。
/// 平台无实现时返回 `None`。
pub fn cursor_position() -> Option<(f64, f64)> {
    #[cfg(target_os = "macos")]
    return macos::ffi::cursor_position();

    #[cfg(target_os = "windows")]
    return windows::ffi::cursor_position();

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    None
}

/// 窗口（含 WebView2 等子窗口树）是否仍持有前台焦点。
/// Windows 的 `Focused(false)` 可能只是焦点转移给了子窗口，
/// 调用方据此区分「点击弹窗内部」与「点击外部」（仅 Windows 有）。
#[cfg(target_os = "windows")]
pub fn window_in_foreground(hwnd: isize) -> bool {
    windows::ffi::window_in_foreground(hwnd)
}
