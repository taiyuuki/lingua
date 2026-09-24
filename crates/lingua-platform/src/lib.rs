//! 平台抽象层。
//!
//! 目标：业务代码只见 trait，不见平台 API。所有 macOS / Windows /
//! Linux 的特殊逻辑都收敛在本 crate 的平台实现模块中，
//! 通过 `cfg(target_os = ...)` 隔离，绝不出现在 src-tauri 与前端里。

pub mod os;
pub mod selection;

pub use os::{Os, current_os};
pub use selection::SelectionProvider;
