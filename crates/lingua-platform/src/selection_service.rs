//! 划词服务：按可靠性顺序尝试多个 SelectionProvider。
//!
//! fallback 约定：
//! * [`AppError::NoSelection`] / 空白文本 → 换下一个 Provider；
//! * [`AppError::Permission`] → 直接终止（fallback 同样需要权限，
//!   继续尝试只会浪费时间并污染剪贴板）；
//! * 其它错误 → 记录日志后继续尝试，最终返回最后一个错误。

use crate::selection::SelectionProvider;
use lingua_core::{AppError, AppResult, SelectedText};

pub struct SelectionService {
    providers: Vec<Box<dyn SelectionProvider>>,
}

impl SelectionService {
    pub fn new(providers: Vec<Box<dyn SelectionProvider>>) -> Self {
        Self { providers }
    }

    /// 依次尝试所有 Provider，返回第一个成功结果。
    pub async fn capture_first(&self) -> AppResult<SelectedText> {
        let mut last_error = AppError::NoSelection;
        for provider in &self.providers {
            match provider.capture().await {
                Ok(selected) if !selected.is_blank() => return Ok(selected),
                Ok(_) => {
                    tracing::debug!(provider = provider.name(), "blank selection, trying next");
                }
                Err(AppError::NoSelection) => {
                    tracing::debug!(provider = provider.name(), "no selection, trying next");
                }
                // 权限缺失时不再尝试 fallback（fallback 同样需要权限），
                // 原样透传，让前端给出权限引导。
                Err(error @ AppError::Permission(_)) => return Err(error),
                Err(error) => {
                    tracing::warn!(provider = provider.name(), error = %error, "selection provider failed");
                    last_error = error;
                }
            }
        }
        Err(last_error)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use lingua_core::SelectionSource;

    struct StaticProvider(&'static str, &'static str);

    #[async_trait::async_trait]
    impl SelectionProvider for StaticProvider {
        fn name(&self) -> &str {
            self.0
        }

        async fn capture(&self) -> AppResult<SelectedText> {
            match self.1 {
                "no_selection" => Err(AppError::NoSelection),
                "permission" => Err(AppError::Permission("denied".into())),
                "" => Ok(SelectedText::new("   ")),
                text => Ok(SelectedText {
                    text: text.to_string(),
                    source: SelectionSource::Unknown,
                    language: None,
                    rect: None,
                }),
            }
        }
    }

    fn service(first: &'static str, second: &'static str) -> SelectionService {
        SelectionService::new(vec![
            Box::new(StaticProvider("first", first)),
            Box::new(StaticProvider("second", second)),
        ])
    }

    #[tokio::test]
    async fn first_success_wins() {
        let selected = service("hello", "world").capture_first().await.unwrap();
        assert_eq!(selected.text, "hello");
    }

    #[tokio::test]
    async fn blank_and_no_selection_fall_through() {
        let selected = service("", "world").capture_first().await.unwrap();
        assert_eq!(selected.text, "world");

        let selected = service("no_selection", "world")
            .capture_first()
            .await
            .unwrap();
        assert_eq!(selected.text, "world");
    }

    #[tokio::test]
    async fn permission_short_circuits() {
        let error = service("permission", "world")
            .capture_first()
            .await
            .unwrap_err();
        // 权限缺失时不再尝试 fallback（避免无意义地污染剪贴板），并保留原始错误。
        assert!(matches!(error, AppError::Permission(_)));
    }

    #[tokio::test]
    async fn all_fail_reports_no_selection_or_last_error() {
        let error = service("no_selection", "no_selection")
            .capture_first()
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::NoSelection));
    }
}
