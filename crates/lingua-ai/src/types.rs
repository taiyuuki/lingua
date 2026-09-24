//! 与具体协议无关的请求/响应模型。
//!
//! 这些类型是 AI Provider trait 的通用语言：OpenAI-compatible 实现负责
//! 把它们映射成 `/chat/completions` 请求，未来 Gemini / Claude 实现负责
//! 各自的映射。任何厂商字段都不允许泄漏到这些结构里。

use serde::{Deserialize, Serialize};

/// 消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// 一条对话消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

/// 一次对话补全请求。
///
/// `model` 为 `None` 时使用 Provider 配置里的默认模型；
/// Router 层据此实现「普通翻译用本地模型、复杂任务用云端模型」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

impl ChatRequest {
    /// 最常见的构造：一条 system + 一条 user。
    pub fn simple(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            messages: vec![ChatMessage::system(system), ChatMessage::user(user)],
            model: None,
            temperature: None,
            max_tokens: None,
        }
    }
}

/// 一次对话补全响应。
///
/// Phase 2 接入流式输出时会扩展为 `ContentPart` 列表，此处先保持简单。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResponse {
    pub content: String,
    pub model: Option<String>,
    pub finish_reason: Option<String>,
}

/// Vision 请求：一张图片 + 指令，用于 OCR 失败后的高级 fallback。
///
/// 结构化输出（blocks/bbox）由调用方通过 prompt 约定 JSON schema，
/// Vision 层不假设图片内容类型（游戏 UI / 漫画 / 截图均可）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionRequest {
    /// Base64 编码的图片数据（不带 data: 前缀）。
    pub image_base64: String,
    pub mime_type: String,
    pub prompt: String,
}

/// Provider 能力声明，Router 据此挑选 Provider（如 Vision 任务只派给
/// `vision == true` 的 Provider）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub chat: bool,
    pub translate: bool,
    pub vision: bool,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            chat: true,
            translate: false,
            vision: false,
        }
    }
}

impl ProviderCapabilities {
    pub const fn all() -> Self {
        Self {
            chat: true,
            translate: true,
            vision: true,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn simple_request_has_system_and_user() {
        let request = ChatRequest::simple("You are a translator.", "hello");
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, ChatRole::System);
        assert_eq!(request.model, None);
    }

    #[test]
    fn capabilities_default_is_chat_only() {
        let caps = ProviderCapabilities::default();
        assert!(caps.chat && !caps.vision && !caps.translate);
        assert!(ProviderCapabilities::all().vision);
    }

    #[test]
    fn message_serializes_snake_case_role() {
        let json = serde_json::to_value(ChatMessage::user("hi")).unwrap();
        assert_eq!(json["role"], "user");
    }
}
