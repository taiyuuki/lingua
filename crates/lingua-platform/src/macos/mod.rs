//! macOS 平台实现（仅 `cfg(target_os = "macos")` 编译）。
//!
//! * `ffi`：唯一的 unsafe 区域 —— Accessibility 与 CoreGraphics 的
//!   最小 FFI 面（读选中文本、光标位置、模拟 Cmd+C、权限检测）。
//! * `providers`：两个 [`SelectionProvider`](crate::SelectionProvider)
//!   实现，按可靠性排序注册进 SelectionService。

pub mod ffi;
pub mod providers;

pub use providers::{AccessibilitySelectionProvider, ClipboardSelectionProvider};
