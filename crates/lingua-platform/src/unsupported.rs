//! 非 macOS 平台的占位实现。
//!
//! Phase 10（跨平台完善）替换为真实实现：
//! * Windows：UI Automation（`TextPattern`）+ 剪贴板 fallback；
//! * Linux：AT-SPI2 + 剪贴板 fallback（Wayland 限制另见架构文档）。
//!
//! 刻意返回明确的 `Platform` 错误而不是假实现。

use crate::selection::SelectionProvider;
use lingua_core::{AppError, AppResult, SelectedText};

pub struct UnsupportedSelectionProvider;

#[async_trait::async_trait]
impl SelectionProvider for UnsupportedSelectionProvider {
    fn name(&self) -> &str {
        "unsupported-platform"
    }

    async fn capture(&self) -> AppResult<SelectedText> {
        Err(AppError::Platform(
            "selection capture is not implemented on this platform yet (planned for Phase 10)"
                .to_string(),
        ))
    }
}
