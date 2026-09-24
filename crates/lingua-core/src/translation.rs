//! 翻译请求/结果的数据契约。
//!
//! 这里只定义「Translation Engine 的输入输出」；引擎本身（路由到
//! Dictionary / Translation Provider / AI）在后续阶段实现，
//! 属于 application/domain service 层。

use crate::language::Language;
use serde::{Deserialize, Serialize};

/// 翻译风格模式，会透传给 Prompt 与路由决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationMode {
    #[default]
    Default,
    Dictionary,
    Fluent,
    Literal,
}

/// 一次翻译请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRequest {
    pub text: String,
    /// `None` 表示由引擎自动检测源语言。
    pub source_language: Option<Language>,
    pub target_language: Language,
    /// 额外上下文（如选中文本所在句子），AI 路由时会用到。
    pub context: Option<String>,
    pub mode: TranslationMode,
}

impl TranslationRequest {
    pub fn new(text: impl Into<String>, target_language: Language) -> Self {
        Self {
            text: text.into(),
            source_language: None,
            target_language,
            context: None,
            mode: TranslationMode::Default,
        }
    }
}

/// 一次翻译结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationResult {
    pub source_text: String,
    pub translated_text: String,
    pub detected_language: Option<Language>,
    pub alternatives: Vec<String>,
    pub explanation: Option<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn request_defaults_to_auto_detect_and_default_mode() {
        let request = TranslationRequest::new("hello", Language::Zh);
        assert_eq!(request.source_language, None);
        assert_eq!(request.mode, TranslationMode::Default);
    }

    #[test]
    fn serde_roundtrip_preserves_fields() {
        let request = TranslationRequest {
            text: "ephemeral".into(),
            source_language: Some(Language::En),
            target_language: Language::Zh,
            context: Some("an ephemeral moment".into()),
            mode: TranslationMode::Dictionary,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"targetLanguage\":\"zh\""));
        assert!(json.contains("\"mode\":\"dictionary\""));
        let back: TranslationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, request);
    }

    #[test]
    fn result_serializes_camel_case() {
        let result = TranslationResult {
            source_text: "hello".into(),
            translated_text: "你好".into(),
            detected_language: Some(Language::En),
            alternatives: vec!["您好".into()],
            explanation: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("translatedText").is_some());
        assert!(json.get("detectedLanguage").is_some());
    }
}
