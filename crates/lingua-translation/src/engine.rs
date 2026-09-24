//! 翻译引擎。
//!
//! 路由策略（按需求「Translation Engine 根据请求决定使用什么」）：
//!
//! * **单词 + 配置了词典 + 词典命中（释义语言 == 目标语言）** →
//!   直接返回词典结果，不发 AI 请求（快、离线、零成本）；
//! * 单词未命中 / 短语 / 句子 → AI Provider。
//!
//! 输出契约：要求模型返回严格 JSON（见 `prompts/translate/system.md`），
//! 解析失败时优雅降级——把整段响应当作 `translated_text`，
//! 绝不因模型格式偏差让整次翻译失败。

use crate::template::{PromptTemplate, vars_from_pairs};
use lingua_ai::{AIProvider, types::ChatRequest};
use lingua_core::{
    AppError, AppResult, DictionaryEntry, Language,
    translation::{
        TextKind, TranslationMode, TranslationRequest, TranslationResult, classify_text,
    },
};
use lingua_dictionary::DictionaryProvider;
use std::collections::HashMap;
use std::sync::Arc;

/// 翻译 Prompt（编译期内嵌，修改后重新编译即生效）。
mod prompts {
    pub const TRANSLATE_SYSTEM: &str = include_str!("../prompts/translate/system.md");
    pub const TRANSLATE_USER: &str = include_str!("../prompts/translate/user.md");
    /// 流式路径专用：纯文本输出（无 JSON 信封），保证增量可读。
    pub const STREAM_SYSTEM: &str = include_str!("../prompts/translate/stream_system.md");
}

pub struct TranslationEngine {
    provider: Arc<dyn AIProvider>,
    dictionary: Option<Arc<dyn DictionaryProvider>>,
}

impl TranslationEngine {
    /// 用 Router 选出的 Provider 构造引擎（共享所有权，多个引擎可复用）。
    pub fn new(provider: Arc<dyn AIProvider>) -> Self {
        Self {
            provider,
            dictionary: None,
        }
    }

    /// 挂载词典（单词查询优先走词典）。可传 `None` 保持纯 AI 行为。
    pub fn with_dictionary(mut self, dictionary: Option<Arc<dyn DictionaryProvider>>) -> Self {
        self.dictionary = dictionary;
        self
    }

    pub async fn translate(&self, request: &TranslationRequest) -> AppResult<TranslationResult> {
        if let Some(result) = self.try_dictionary(request).await {
            return Ok(result);
        }

        let (system, user) = self.render_prompt(request);
        let chat_request = ChatRequest::simple(system, user);
        let response = self.provider.chat(&chat_request).await?;
        Ok(parse_ai_translation(request, response.content))
    }

    /// 流式翻译：词典命中时一次性回调译文；AI 路径使用**纯文本输出**
    /// 的流式 prompt——模型每产出一个增量就实时回调，保证首字延迟后
    /// 译文连续流动（不做 JSON 信封：那会让增量提取依赖模型字段顺序，
    /// 模型不配合时整个流期间发不出任何增量，表现为"等完了一波出"）。
    /// 结束时做轻量清理（去围栏/首尾空白），并把清理后译文补发一次。
    pub async fn translate_stream(
        &self,
        request: &TranslationRequest,
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> AppResult<TranslationResult> {
        if let Some(result) = self.try_dictionary(request).await {
            on_delta(result.translated_text.clone());
            return Ok(result);
        }

        let chat_request = ChatRequest::simple(
            prompts::STREAM_SYSTEM.to_string(),
            self.user_prompt(request),
        );
        let mut accumulated = String::new();
        self.provider
            .chat_stream(&chat_request, &mut |delta: String| {
                // Provider 契约：回调参数是到目前为止的完整文本。
                accumulated = delta.clone();
                on_delta(delta);
            })
            .await?;

        let translated_text = Self::sanitize_streamed(&accumulated);
        if translated_text != accumulated {
            on_delta(translated_text.clone());
        }
        Ok(TranslationResult {
            source_text: request.text.clone(),
            translated_text,
            detected_language: request.source_language.clone(),
            alternatives: Vec::new(),
            explanation: None,
            dictionary: None,
        })
    }

    /// 收尾清理：去掉流式输出偶发的代码围栏与首尾空白。
    fn sanitize_streamed(text: &str) -> String {
        let trimmed = text.trim();
        let stripped = trimmed
            .strip_prefix("```")
            .and_then(|rest| rest.strip_suffix("```"))
            .map_or(trimmed, str::trim);
        stripped.to_string()
    }

    /// 词典路由：对「单词」与「词组」生效（词典里也有多词词头，
    /// 如 "15 minutes of fame"；索引查询便宜，未命中静默回退 AI），
    /// 整句仍直接走 AI；任何失败（未命中、释义语言不匹配、查询出错）
    /// 都静默回退 AI，不阻断翻译。
    async fn try_dictionary(&self, request: &TranslationRequest) -> Option<TranslationResult> {
        let dictionary = self.dictionary.as_ref()?;
        if classify_text(&request.text) == TextKind::Sentence {
            return None;
        }
        let word = request.text.trim();
        let entries = match dictionary
            .lookup(word, request.source_language.clone())
            .await
        {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(error = %error, "dictionary lookup failed; falling back to AI");
                return None;
            }
        };
        let usable: Vec<DictionaryEntry> = entries
            .into_iter()
            .filter(|entry| {
                entry.has_definitions() && entry.gloss_language == request.target_language
            })
            .collect();
        let primary = usable.first().cloned()?;
        tracing::debug!(
            source = %primary.source,
            definitions = primary.definitions.len(),
            "dictionary hit"
        );

        let translated_text = primary
            .definitions
            .iter()
            .find(|definition| !definition.definition.trim().is_empty())
            .map(|definition| definition.definition.clone())
            .unwrap_or_else(|| primary.lemma.clone());
        let alternatives: Vec<String> = usable
            .iter()
            .flat_map(|entry| entry.definitions.iter())
            .filter(|definition| definition.definition != translated_text)
            .map(|definition| definition.definition.clone())
            .take(6)
            .collect();

        Some(TranslationResult {
            source_text: request.text.clone(),
            translated_text,
            detected_language: request
                .source_language
                .clone()
                .or(Some(primary.language.clone())),
            alternatives,
            explanation: None,
            dictionary: Some(primary),
        })
    }

    fn render_prompt(&self, request: &TranslationRequest) -> (String, String) {
        let template =
            PromptTemplate::from_parts(prompts::TRANSLATE_SYSTEM, prompts::TRANSLATE_USER);
        let vars = self.prompt_vars(request);
        template.render(&vars)
    }

    fn user_prompt(&self, request: &TranslationRequest) -> String {
        let template = PromptTemplate::from_parts("", prompts::TRANSLATE_USER);
        // render 返回 (system, user)——这里只要 user（含原文与目标语言）。
        let (_, prompt) = template.render(&self.prompt_vars(request));
        prompt
    }

    fn prompt_vars(&self, request: &TranslationRequest) -> HashMap<String, String> {
        vars_from_pairs(&[
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
        ])
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
        dictionary: None,
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
        dictionary: None,
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

    // ---- 词典路由 ----

    use lingua_core::{DictionaryDefinition, DictionaryEntry};
    use lingua_database::DictionaryMeta;
    use lingua_dictionary::SqliteDictionaryProvider;

    fn dict_entry(lemma: &str, definition: &str) -> DictionaryEntry {
        DictionaryEntry {
            lemma: lemma.into(),
            language: Language::En,
            gloss_language: Language::Zh,
            pronunciations: vec!["/test/".into()],
            inflections: vec![],
            source: "test-dict".into(),
            definitions: vec![DictionaryDefinition {
                part_of_speech: Some("noun".into()),
                definition: definition.into(),
                html: None,
                examples: vec![],
                synonyms: vec![],
                antonyms: vec![],
            }],
        }
    }

    async fn engine_with_dict(
        provider_response: &str,
        entries: Vec<DictionaryEntry>,
    ) -> TranslationEngine {
        let database = lingua_database::Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        database
            .import_dictionary(
                &DictionaryMeta {
                    id: "test-dict".into(),
                    name: "Test".into(),
                    source_language: Language::En,
                    gloss_language: Language::Zh,
                },
                &entries,
            )
            .unwrap();
        TranslationEngine::new(Arc::new(ScriptedProvider {
            response: provider_response.into(),
        }))
        .with_dictionary(Some(Arc::new(SqliteDictionaryProvider::new(database))))
    }

    #[tokio::test]
    async fn word_uses_dictionary_without_ai() {
        // Provider 配了会报错的响应：如果引擎走了 AI，测试会拿到错误。
        let engine = engine_with_dict(
            "should-not-be-called",
            vec![dict_entry("ephemeral", "短暂的")],
        )
        .await;
        let result = engine.translate(&request("Ephemeral")).await.unwrap();
        assert_eq!(result.translated_text, "短暂的");
        assert!(result.dictionary.is_some());
        assert_eq!(result.dictionary.as_ref().unwrap().lemma, "ephemeral");
        assert_eq!(result.detected_language, Some(Language::En));
    }

    #[tokio::test]
    async fn word_miss_falls_back_to_ai() {
        let engine = engine_with_dict("从 AI 来的", vec![dict_entry("other", "别的词")]).await;
        let result = engine.translate(&request("ephemeral")).await.unwrap();
        assert_eq!(result.translated_text, "从 AI 来的");
        assert!(result.dictionary.is_none());
    }

    #[tokio::test]
    async fn gloss_language_mismatch_falls_back_to_ai() {
        // 词典是 en→en（释义语言 != 目标语言 zh），不应被采用。
        let database = lingua_database::Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        database
            .import_dictionary(
                &DictionaryMeta {
                    id: "en-en".into(),
                    name: "En-En".into(),
                    source_language: Language::En,
                    gloss_language: Language::En,
                },
                &[dict_entry("ephemeral", "brief")],
            )
            .unwrap();
        let engine = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "从 AI 来的".into(),
        }))
        .with_dictionary(Some(Arc::new(SqliteDictionaryProvider::new(database))));
        let result = engine.translate(&request("ephemeral")).await.unwrap();
        assert_eq!(result.translated_text, "从 AI 来的");
        assert!(result.dictionary.is_none());
    }

    #[tokio::test]
    async fn phrase_uses_dictionary_without_ai() {
        // 多词词组也查词典（词库里有 "15 minutes of fame" 这类词头）。
        let engine = engine_with_dict(
            "should-not-be-called",
            vec![dict_entry("an ephemeral moment", "朝露之瞬")],
        )
        .await;
        let result = engine.translate(&request("an ephemeral moment")).await.unwrap();
        assert_eq!(result.translated_text, "朝露之瞬");
        assert_eq!(result.dictionary.as_ref().unwrap().lemma, "an ephemeral moment");
    }

    #[tokio::test]
    async fn long_cjk_phrase_uses_dictionary() {
        // CJK 超 4 字判为词组，同样走词典（此前只有 ≤4 字的「单词」走）。
        let engine = engine_with_dict("should-not-be-called", vec![dict_entry(
            "人工智能大模型",
            "大规格 AI 模型",
        )])
        .await;
        let result = engine.translate(&request("人工智能大模型")).await.unwrap();
        assert_eq!(result.translated_text, "大规格 AI 模型");
    }

    #[tokio::test]
    async fn sentence_skips_dictionary_and_uses_ai() {
        // 句末标点判为整句：直接走 AI，不查词典。
        let engine = engine_with_dict("从 AI 来的", vec![dict_entry("hello", "你好")]).await;
        let request = TranslationRequest::new("hello world.", Language::Zh);
        let result = engine.translate(&request).await.unwrap();
        assert_eq!(result.translated_text, "从 AI 来的");
        assert!(result.dictionary.is_none());
    }

    #[tokio::test]
    async fn streaming_translation_emits_translated_text() {
        // 默认 chat_stream 退化为非流式：一次回调完整文本
        let engine = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "  ```\n短暂的\n```  ".into(),
        }));
        let deltas = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let collected = deltas.clone();
        let result = engine
            .translate_stream(&request("ephemeral"), &mut |text: String| {
                collected.lock().unwrap().push(text);
            })
            .await
            .unwrap();
        assert_eq!(result.translated_text, "短暂的");
        assert_eq!(result.alternatives.len(), 0, "流式路径无 JSON 信封");
        let deltas = deltas.lock().unwrap();
        assert_eq!(deltas.as_slice(), ["  ```\n短暂的\n```  ", "短暂的"]);
    }

    #[test]
    fn sanitize_strips_fences_and_whitespace() {
        assert_eq!(TranslationEngine::sanitize_streamed("  短暂的  "), "短暂的");
        assert_eq!(
            TranslationEngine::sanitize_streamed("```\n短暂的\n```"),
            "短暂的"
        );
    }

    #[tokio::test]
    async fn dictionary_path_streams_once() {
        let engine = engine_with_dict("unused", vec![dict_entry("cat", "猫")]).await;
        let deltas = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let collected = deltas.clone();
        let result = engine
            .translate_stream(&request("cat"), &mut |text: String| {
                collected.lock().unwrap().push(text);
            })
            .await
            .unwrap();
        assert_eq!(result.translated_text, "猫");
        assert_eq!(deltas.lock().unwrap().as_slice(), ["猫"]);
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

    #[test]
    fn stream_user_prompt_must_contain_source_text() {
        // 回归：曾误取 render 返回的 system 位（空串），导致模型收到空输入。
        let engine = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "{}".into(),
        }));
        let user = engine.user_prompt(&request("rules against source files"));
        assert!(
            user.contains("rules against source files"),
            "user prompt 丢了原文: {user}"
        );
        assert!(user.contains("zh"));
        assert!(!engine.render_prompt(&request("x")).0.is_empty());
    }
}
