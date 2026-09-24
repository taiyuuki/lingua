//! 翻译引擎。
//!
//! Phase 1 路由策略：一切请求走 AI Provider（单词/短语/句子统一处理，
//! 词典路由在 Phase 3 加入 `Dictionary` 优先分支）。
//!
//! 输出契约：要求模型返回严格 JSON（见 `prompts/translate/system.md`），
//! 解析失败时优雅降级——把整段响应当作 `translated_text`，
//! 绝不因模型格式偏差让整次翻译失败。

use crate::template::{PromptTemplate, vars_from_pairs};
use lingua_ai::{AIProvider, types::ChatRequest};
use lingua_core::{
    AppError, AppResult, Language,
    translation::{TranslationMode, TranslationRequest, TranslationResult},
};
use std::sync::Arc;

/// 翻译 Prompt（编译期内嵌，修改后重新编译即生效）。
mod prompts {
    pub const TRANSLATE_SYSTEM: &str = include_str!("../prompts/translate/system.md");
    pub const TRANSLATE_USER: &str = include_str!("../prompts/translate/user.md");
}

pub struct TranslationEngine {
    provider: Arc<dyn AIProvider>,
}

impl TranslationEngine {
    /// 用 Router 选出的 Provider 构造引擎（共享所有权，多个引擎可复用）。
    pub fn new(provider: Arc<dyn AIProvider>) -> Self {
        Self { provider }
    }

    pub async fn translate(&self, request: &TranslationRequest) -> AppResult<TranslationResult> {
        let (system, user) = self.render_prompt(request);
        let chat_request = ChatRequest::simple(system, user);
        let response = self.provider.chat(&chat_request).await?;
        Ok(parse_ai_translation(request, response.content))
    }

    fn render_prompt(&self, request: &TranslationRequest) -> (String, String) {
        let template =
            PromptTemplate::from_parts(prompts::TRANSLATE_SYSTEM, prompts::TRANSLATE_USER);
        let vars = vars_from_pairs(&[
            ("text", request.text.clone()),
            (
                "source_language",
                request.source_language.as_ref().map_or_else(
                    || "auto".to_string(),
                    |language| language.code().to_string(),
                ),
            ),
            (
                "target_language",
                request.target_language.code().to_string(),
            ),
            ("context", request.context.clone().unwrap_or_default()),
            (
                "mode",
                match request.mode {
                    TranslationMode::Default => "default".to_string(),
                    TranslationMode::Dictionary => "dictionary".to_string(),
                    TranslationMode::Fluent => "fluent".to_string(),
                    TranslationMode::Literal => "literal".to_string(),
                },
            ),
        ]);
        template.render(&vars)
    }
}

/// 从模型响应解析结构化翻译结果；非 JSON 输出整段作为译文。
pub(crate) fn parse_ai_translation(
    request: &TranslationRequest,
    content: String,
) -> TranslationResult {
    let fallback = || TranslationResult {
        source_text: request.text.clone(),
        translated_text: content.trim().to_string(),
        detected_language: None,
        alternatives: Vec::new(),
        explanation: None,
    };

    let Some(json_start) = content.find('{') else {
        return fallback();
    };
    let Some(json_end) = content.rfind('}') else {
        return fallback();
    };
    if json_end < json_start {
        return fallback();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content[json_start..=json_end])
    else {
        return fallback();
    };

    let translated_text = value
        .get("translated_text")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| content.trim().to_string());
    let detected_language = value
        .get("detected_language")
        .and_then(serde_json::Value::as_str)
        .map(Language::parse);
    let alternatives: Vec<String> = value
        .get("alternatives")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let explanation = value
        .get("explanation")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    tracing::debug!(
        alternatives = alternatives.len(),
        has_explanation = explanation.is_some(),
        "parsed AI translation"
    );

    TranslationResult {
        source_text: request.text.clone(),
        translated_text,
        detected_language,
        alternatives,
        explanation,
    }
}

/// 无 Provider 可用时的统一错误（pipeline 在配置缺失时提前拦截，
/// 这个构造器主要供测试与未来 Router 使用）。
pub fn no_provider_error() -> AppError {
    AppError::InvalidInput("no AI provider is configured".to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use lingua_ai::AIProvider;
    use lingua_ai::types::{ChatResponse, VisionRequest};

    struct ScriptedProvider {
        response: String,
    }

    #[async_trait::async_trait]
    impl AIProvider for ScriptedProvider {
        fn id(&self) -> &str {
            "scripted"
        }

        async fn chat(&self, _request: &ChatRequest) -> AppResult<ChatResponse> {
            Ok(ChatResponse {
                content: self.response.clone(),
                model: Some("scripted-1".into()),
                finish_reason: Some("stop".into()),
            })
        }

        async fn vision(&self, _request: &VisionRequest) -> AppResult<ChatResponse> {
            Err(AppError::Provider {
                provider: "scripted".into(),
                message: "no vision".into(),
            })
        }
    }

    fn request(text: &str) -> TranslationRequest {
        TranslationRequest::new(text, Language::Zh)
    }

    #[tokio::test]
    async fn parses_structured_json_response() {
        let engine = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: r#"```json
{"detected_language": "en", "translated_text": "短暂的", "alternatives": ["转瞬即逝的"], "explanation": "形容存在时间极短"}
```"#
            .into(),
        }));
        let result = engine.translate(&request("ephemeral")).await.unwrap();
        assert_eq!(result.translated_text, "短暂的");
        assert_eq!(result.detected_language, Some(Language::En));
        assert_eq!(result.alternatives, vec!["转瞬即逝的".to_string()]);
        assert_eq!(result.explanation.as_deref(), Some("形容存在时间极短"));
        assert_eq!(result.source_text, "ephemeral");
    }

    #[tokio::test]
    async fn plain_text_response_falls_back_to_translation() {
        let engine = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "短暂的".into(),
        }));
        let result = engine.translate(&request("ephemeral")).await.unwrap();
        assert_eq!(result.translated_text, "短暂的");
        assert!(result.alternatives.is_empty());
        assert_eq!(result.detected_language, None);
    }

    #[tokio::test]
    async fn malformed_json_falls_back_gracefully() {
        let engine = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "{oops not json".into(),
        }));
        let result = engine.translate(&request("hello")).await.unwrap();
        assert_eq!(result.translated_text, "{oops not json");
    }

    #[test]
    fn prompt_contains_text_and_target() {
        let engine = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "{}".into(),
        }));
        let (system, user) = engine.render_prompt(&request("hello"));
        assert!(system.contains("JSON"));
        assert!(user.contains("hello"));
        assert!(user.contains("zh"));
    }
}
