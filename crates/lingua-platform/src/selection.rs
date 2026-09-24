//! 划词 Provider 抽象。
//!
//! # 平台实现规划（Phase 1）
//!
//! | 平台 | 首选实现 | Fallback |
//! |------|----------|----------|
//! | macOS | Accessibility API（AXUIElement） | 剪贴板（模拟 Cmd+C） |
//! | Windows | UI Automation | 剪贴板（模拟 Ctrl+C） |
//! | Linux | AT-SPI2 | 剪贴板；Wayland 下受协议限制，见下 |
//!
//! 注意事项（来自需求约束，不许假设）：
//!
//! * 不是所有应用都暴露系统 Selection API —— 必须 fallback。
//! * 剪贴板 fallback 会覆盖用户剪贴板内容，实现时必须先备份后恢复，
//!   且只在首选方案失败时使用。
//! * Wayland 没有全局坐标/全局热键的统一方案，Popup 定位与快捷键
//!   在 Wayland 下的限制要在 Phase 10（跨平台完善）里逐桌面环境处理。

use lingua_core::{AppResult, SelectedText};

/// 划词能力抽象。实现方必须是可并发共享的（会被注册进 Provider 注册表）。
///
/// `capture` 是一次「尽力而为」的快照：拿不到就返回
/// [`AppError::Permission`]（缺系统权限）或 [`AppError::Platform`]
/// （当前应用不支持），不允许返回空文本假装成功。
#[async_trait::async_trait]
pub trait SelectionProvider: Send + Sync {
    /// Provider 名称，用于日志与设置界面（如 `"macos-accessibility"`）。
    fn name(&self) -> &str;

    /// 获取当前选中的文本。
    async fn capture(&self) -> AppResult<SelectedText>;
}

/// 编译期保证 trait 可以做成 `Box<dyn SelectionProvider>` 注册表。
#[allow(dead_code)]
fn assert_object_safe(provider: Box<dyn SelectionProvider>) -> Box<dyn SelectionProvider> {
    provider
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use lingua_core::SelectionSource;

    /// 测试替身：模拟「系统 API 拿到了选中文本」。
    struct FakeAccessible;

    #[async_trait::async_trait]
    impl SelectionProvider for FakeAccessible {
        fn name(&self) -> &str {
            "fake-accessible"
        }

        async fn capture(&self) -> AppResult<SelectedText> {
            Ok(SelectedText {
                text: "ephemeral".into(),
                source: SelectionSource::Accessibility,
                language: None,
                rect: None,
            })
        }
    }

    #[tokio::test]
    async fn provider_can_be_dispatched_dynamically() {
        let provider: Box<dyn SelectionProvider> = Box::new(FakeAccessible);
        let selected = provider.capture().await.unwrap();
        assert_eq!(selected.text, "ephemeral");
        assert_eq!(provider.name(), "fake-accessible");
    }
}
