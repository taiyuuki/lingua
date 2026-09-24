//! [`AIProvider`] trait —— 所有 AI 服务的统一入口。
//!
//! * 实现方负责认证、超时、重试与错误映射，失败一律映射为
//!   [`AppError`]（网络类 → `Network`，密钥类 → `Authentication`，
//!   服务端业务错 → `Provider`），不允许泄漏 HTTP 状态码给上层。
//! * `vision` 给了默认实现：不支持视觉能力的 Provider 直接返回
//!   `Provider` 错误，Router 捕获后可以改派其他 Provider。

use crate::types::{ChatRequest, ChatResponse, VisionRequest};
use lingua_core::{AppError, AppResult};

#[async_trait::async_trait]
pub trait AIProvider: Send + Sync {
    /// 稳定标识，对应 [`crate::config::ProviderConfig::id`]。
    fn id(&self) -> &str;

    /// 能力声明，Router 据此做任务分派。
    fn capabilities(&self) -> crate::types::ProviderCapabilities {
        crate::types::ProviderCapabilities::default()
    }

    /// 对话补全（翻译、解释、学习分析等都走这里）。
    async fn chat(&self, request: &ChatRequest) -> AppResult<ChatResponse>;

    /// 流式对话补全：内容增量到达时调用 `on_delta`，回调参数是
    /// **到目前为止的完整文本**（前端直接替换渲染，天然抗丢事件）。
    /// 用 `String` 而非 `&str` 避免高阶生命周期与 async-trait 的兼容问题。
    /// 默认实现退化为非流式：拿到完整结果后回调一次。
    async fn chat_stream(
        &self,
        request: &ChatRequest,
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> AppResult<ChatResponse> {
        let response = self.chat(request).await?;
        on_delta(response.content.clone());
        Ok(response)
    }

    /// 视觉理解，作为 OCR 的高级 fallback。
    async fn vision(&self, _request: &VisionRequest) -> AppResult<ChatResponse> {
        Err(AppError::Provider {
            provider: self.id().to_string(),
            message: "this provider does not support vision".to_string(),
        })
    }
}

/// 编译期保证 trait 对象安全（Provider 注册表需要 `Box<dyn AIProvider>`）。
#[allow(dead_code)]
fn assert_object_safe(provider: Box<dyn AIProvider>) -> Box<dyn AIProvider> {
    provider
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::types::{ChatMessage, ChatRole};

    /// 测试替身：固定回显，模拟一个可用的 Provider。
    struct EchoProvider;

    #[async_trait::async_trait]
    impl AIProvider for EchoProvider {
        fn id(&self) -> &str {
            "echo"
        }

        async fn chat(&self, request: &ChatRequest) -> AppResult<ChatResponse> {
            let last = request
                .messages
                .last()
                .ok_or_else(|| AppError::InvalidInput("empty message list".to_string()))?;
            Ok(ChatResponse {
                content: format!("echo: {}", last.content),
                model: Some("echo-1".into()),
                finish_reason: Some("stop".into()),
            })
        }
    }

    #[tokio::test]
    async fn chat_round_trips_through_trait_object() {
        let provider: Box<dyn AIProvider> = Box::new(EchoProvider);
        let request = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hello".into(),
            }],
            model: None,
            temperature: None,
            max_tokens: None,
        };
        let response = provider.chat(&request).await.unwrap();
        assert_eq!(response.content, "echo: hello");
    }

    #[tokio::test]
    async fn vision_defaults_to_unsupported() {
        let provider: Box<dyn AIProvider> = Box::new(EchoProvider);
        let error = provider
            .vision(&VisionRequest {
                image_base64: "aGk=".into(),
                mime_type: "image/png".into(),
                prompt: "read the text".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "provider");
    }
}
