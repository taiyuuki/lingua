//! 平台抽象层。
//!
//! 目标：业务代码只见 trait，不见平台 API。所有 macOS / Windows /
//! Linux 的特殊逻辑都收敛在本 crate 的平台实现模块中，
//! 通过 `cfg(target_os = ...)` 隔离，绝不出现在 src-tauri 与前端里。

pub mod os;
pub mod selection;
pub mod selection_service;
pub mod unsupported;

#[cfg(target_os = "macos")]
pub mod macos;

pub use os::{Os, current_os};
pub use selection::SelectionProvider;
pub use selection_service::SelectionService;
pub use unsupported::UnsupportedSelectionProvider;

#[cfg(target_os = "macos")]
pub use macos::{AccessibilitySelectionProvider, ClipboardSelectionProvider};

/// 组装默认划词服务：首选 Accessibility，剪贴板模拟 Cmd+C 兜底。
///
/// Windows / Linux 的真实实现在 Phase 10 接入前，
/// 非 macOS 平台只有 `unsupported-platform` 占位 Provider。
pub fn create_default_selection_service() -> SelectionService {
    #[cfg(target_os = "macos")]
    return SelectionService::new(vec![
        Box::new(AccessibilitySelectionProvider),
        Box::new(ClipboardSelectionProvider::default()),
    ]);

    #[cfg(not(target_os = "macos"))]
    SelectionService::new(vec![Box::new(UnsupportedSelectionProvider)])
}

/// 划词所需权限是否已授予（macOS：辅助功能；其它平台视为已授予）。
pub fn selection_permission_granted() -> bool {
    #[cfg(target_os = "macos")]
    return macos::ffi::is_accessibility_trusted();

    #[cfg(not(target_os = "macos"))]
    true
}

/// 当前光标全局坐标（逻辑点，左上角原点）；Popup 定位用。
/// 平台无实现时返回 `None`。
pub fn cursor_position() -> Option<(f64, f64)> {
    #[cfg(target_os = "macos")]
    return macos::ffi::cursor_position();

    #[cfg(not(target_os = "macos"))]
    None
}
