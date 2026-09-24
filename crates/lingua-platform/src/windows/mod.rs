//! Windows 平台实现（仅 `cfg(target_os = "windows")` 编译）。
//!
//! * `ffi`：唯一的 unsafe 区域 —— UI Automation、SendInput、
//!   GetCursorPos 与 GDI 截图的最小封装。
//! * `providers`：两个 [`SelectionProvider`](crate::SelectionProvider)
//!   实现，按可靠性排序注册进 SelectionService。
//!
//! 命名注意：本模块与 `windows` crate 同名，模块内引用外部 crate
//! 时一律写 `::windows::...`（前导冒号）避免歧义。

pub mod ffi;
pub mod providers;

pub use providers::{ClipboardSelectionProvider, UiaSelectionProvider};
