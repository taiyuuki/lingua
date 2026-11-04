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
    /// AI Provider；`None` = 词典-only 模式（无可用服务时词典仍可查，
    /// 词典未命中返回 Config 错误给出配置指引）。
    provider: Option<Arc<dyn AIProvider>>,
    dictionary: Option<Arc<dyn DictionaryProvider>>,
    /// 同语言换向：源文本已经（几乎全部）是目标语言时，改译成该语言。
    /// `None` 或与目标语言相同 = 不换向（保持旧行为）。
    swap_language: Option<Language>,
    /// 用户自定义 system prompt 覆盖（JSON 路径 / 流式路径）；
    /// `None` = 用内置模板。user 模板不支持覆盖（原文位置在内，覆盖
    /// 丢失原文会让翻译直接失败）。
    prompt_override: PromptOverride,
}

/// 自定义 Prompt 覆盖（空串字段视为未覆盖）。
#[derive(Debug, Clone, Default)]
pub struct PromptOverride {
    pub translate_system: Option<String>,
    pub translate_stream_system: Option<String>,
}

impl PromptOverride {
    /// 空白串归一为 `None`（设置存储以空串表示「用内置」）。
    pub fn normalized(translate_system: Option<&str>, stream_system: Option<&str>) -> Self {
        fn clean(value: Option<&str>) -> Option<String> {
            value
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        }
        Self {
            translate_system: clean(translate_system),
            translate_stream_system: clean(stream_system),
        }
    }
}

impl TranslationEngine {
    /// 用 Router 选出的 Provider 构造引擎（共享所有权，多个引擎可复用）。
    pub fn new(provider: Arc<dyn AIProvider>) -> Self {
        Self {
            provider: Some(provider),
            dictionary: None,
            swap_language: None,
            prompt_override: PromptOverride::default(),
        }
    }

    /// 词典查询的公开入口（单词/词组命中返回结果，句子或未命中返回
    /// `None`）。管线用它实现「先词典、后 Keychain」的两阶段：词典
    /// 命中秒回，不被 API Key 的授权等待拖慢。
    pub async fn lookup_dictionary(&self, request: &TranslationRequest) -> Option<TranslationResult> {
        self.try_dictionary(request).await
    }

    /// 挂载词典（单词查询优先走词典）。可传 `None` 保持纯 AI 行为。
    pub fn with_dictionary(mut self, dictionary: Option<Arc<dyn DictionaryProvider>>) -> Self {
        self.dictionary = dictionary;
        self
    }

    /// 词典-only 模式：没有可用的 AI Provider（未配置服务，或 API Key
    /// 读取被系统授权拒绝）时仍然可以查词典；词典未命中才报
    /// `Config` 错误，给用户明确的配置指引而不是整条管线失效。
    pub fn without_provider() -> Self {
        Self {
            provider: None,
            dictionary: None,
            swap_language: None,
            prompt_override: PromptOverride::default(),
        }
    }

    /// AI 路径必需的 Provider；词典-only 模式下词典未命中时返回指引。
    fn require_provider(&self) -> AppResult<&Arc<dyn AIProvider>> {
        self.provider.as_ref().ok_or_else(|| {
            AppError::Config(
                "词典未命中，且没有可用的 AI 服务（未配置 Provider 或 API Key 读取失败）。\
                 请在「设置 → 服务」确认配置；若系统弹出钥匙串授权，请选择允许。"
                    .to_string(),
            )
        })
    }

    /// 配置自定义 system prompt（`None` = 用内置模板）。
    pub fn with_prompt_override(mut self, prompt_override: PromptOverride) -> Self {
        self.prompt_override = prompt_override;
        self
    }

    /// 配置同语言换向目标语言（如目标语言 zh 时设 en：划到中文文本
    /// 自动改译英文）。换向是 Prompt 指令层行为：是否真的同语言由
    /// 模型在阅读源文本时判断，两种 prompt 路径都会注入该规则。
    pub fn with_swap_language(mut self, swap_language: Option<Language>) -> Self {
        self.swap_language = swap_language;
        self
    }

    /// 「同语言规则」prompt 片段：替换两条 system 模板里的
    /// `{{same_language_rule}}` 占位符（JSON 路径与流式路径措辞不同）。
    fn same_language_rule(&self, request: &TranslationRequest, streaming: bool) -> String {
        // 与目标语言相同的换向语言视为未配置。
        let swap = self
            .swap_language
            .as_ref()
            .filter(|swap| **swap != request.target_language);
        match (streaming, swap) {
            (false, Some(swap)) => format!(
                "If the source text is already (almost entirely) in the target language, \
                 DO NOT restate it: translate it into `{}` instead and say in \
                 \"explanation\" that you swapped the direction.",
                swap.code()
            ),
            (false, None) => "If the source text is already in the target language, \
                 still translate it faithfully and say so in \"explanation\"."
                .to_string(),
            (true, Some(swap)) => format!(
                "If the source text is already (almost entirely) in the target language, \
                 DO NOT restate it: translate it into `{}` instead and output only that.",
                swap.code()
            ),
            (true, None) => "If the source text is already in the target language, \
                 output it unchanged."
                .to_string(),
        }
    }

    pub async fn translate(&self, request: &TranslationRequest) -> AppResult<TranslationResult> {
        if let Some(result) = self.try_dictionary(request).await {
            return Ok(result);
        }

        let (system, user) = self.render_prompt(request);
        let chat_request = ChatRequest::simple(system, user);
        let provider = self.require_provider()?;
        let response = provider.chat(&chat_request).await?;
        Ok(parse_ai_translation(
            request,
            response.content,
            response
                .model
                .or_else(|| self.provider.as_ref().map(|p| p.id().to_string())),
        ))
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

        let system = self
            .prompt_override
            .translate_stream_system
            .clone()
            .unwrap_or_else(|| prompts::STREAM_SYSTEM.to_string());
        let chat_request = ChatRequest::simple(system, self.user_prompt(request));
        let provider = self.require_provider()?;
        let mut accumulated = String::new();
        let response = provider
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
            dictionaries: Vec::new(),
            model: response
                .model
                .or_else(|| self.provider.as_ref().map(|p| p.id().to_string())),
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
        // 两级过滤：优先「释义语言 == 目标语言」的词典；一本都不匹配时
        // 回退到任意有释义的词典（词典命中比硬走 AI 有用；例如目标语言
        // 切到英语时 en→zh 词典不该整体消失）。
        let all: Vec<DictionaryEntry> = entries
            .into_iter()
            .filter(DictionaryEntry::has_definitions)
            .collect();
        let usable: Vec<DictionaryEntry> = all
            .iter()
            .filter(|entry| entry.gloss_language == request.target_language)
            .cloned()
            .collect();
        let usable = if usable.is_empty() { all } else { usable };
        let primary = usable.first().cloned()?;
        tracing::debug!(
            dictionaries = usable.len(),
            primary = %primary.source,
            definitions = primary.definitions.len(),
            "dictionary hit"
        );

        let translated_text = primary
            .definitions
            .iter()
            .find(|definition| !definition.definition.trim().is_empty())
            .map(|definition| definition.definition.clone())
            .unwrap_or_else(|| primary.lemma.clone());
        // 候选释义只取首选词典的其余义项；其它词典的词条整个随
        // `dictionaries` 下发，由弹窗切换查看。
        let alternatives: Vec<String> = primary
            .definitions
            .iter()
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
            dictionaries: usable,
            model: None,
        })
    }

    fn render_prompt(&self, request: &TranslationRequest) -> (String, String) {
        let template = PromptTemplate::from_parts(
            self.prompt_override
                .translate_system
                .as_deref()
                .unwrap_or(prompts::TRANSLATE_SYSTEM),
            prompts::TRANSLATE_USER,
        );
        let mut vars = self.prompt_vars(request);
        vars.insert(
            "same_language_rule".to_string(),
            self.same_language_rule(request, false),
        );
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
    model: Option<String>,
) -> TranslationResult {
    let fallback = || TranslationResult {
        source_text: request.text.clone(),
        translated_text: content.trim().to_string(),
        detected_language: None,
        alternatives: Vec::new(),
        explanation: None,
        dictionaries: Vec::new(),
        model: model.clone(),
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
        dictionaries: Vec::new(),
        model,
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
        assert!(!result.dictionaries.is_empty());
        assert_eq!(result.dictionaries[0].lemma, "ephemeral");
        assert_eq!(result.detected_language, Some(Language::En));
    }

    // ---- 词典-only 模式（无可用 AI Provider）----

    async fn dictionary_only_engine(entries: Vec<DictionaryEntry>) -> TranslationEngine {
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
        TranslationEngine::without_provider()
            .with_dictionary(Some(Arc::new(SqliteDictionaryProvider::new(database))))
    }

    #[tokio::test]
    async fn dictionary_only_mode_still_serves_hits() {
        // 场景：用户拒绝钥匙串授权 / 未配置 Provider —— 词典命中必须
        // 正常返回，不能让整条划词管线失效。
        let engine = dictionary_only_engine(vec![dict_entry("ephemeral", "短暂的")]).await;
        let result = engine.translate(&request("Ephemeral")).await.unwrap();
        assert_eq!(result.translated_text, "短暂的");
        assert!(!result.dictionaries.is_empty());
    }

    #[tokio::test]
    async fn dictionary_only_mode_miss_returns_config_error() {
        // 词典未命中时给明确的配置指引（Config），而不是 panic 或静默。
        let engine = dictionary_only_engine(vec![dict_entry("ephemeral", "短暂的")]).await;
        let error = engine.translate(&request("serendipity")).await.unwrap_err();
        assert!(matches!(error, AppError::Config(_)), "{error}");
    }

    #[tokio::test]
    async fn word_miss_falls_back_to_ai() {
        let engine = engine_with_dict("从 AI 来的", vec![dict_entry("other", "别的词")]).await;
        let result = engine.translate(&request("ephemeral")).await.unwrap();
        assert_eq!(result.translated_text, "从 AI 来的");
        assert!(result.dictionaries.is_empty());
    }

    #[tokio::test]
    async fn gloss_language_mismatch_still_uses_dictionary() {
        // 唯一的词典是 en→en（释义语言 != 目标语言 zh）：
        // 两级过滤下仍采用词典结果，而不是硬走 AI。
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
        assert_eq!(result.translated_text, "brief");
        assert_eq!(result.dictionaries.len(), 1);
    }

    #[tokio::test]
    async fn multiple_dictionaries_are_all_carried() {
        // 两本词典都命中：dictionaries 按导入顺序携带两条词条，
        // 首选词典（先导入的）出主结果。
        let database = lingua_database::Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        database
            .import_dictionary(
                &DictionaryMeta {
                    id: "dict-a".into(),
                    name: "Dict A".into(),
                    source_language: Language::En,
                    gloss_language: Language::Zh,
                },
                &[dict_entry("ephemeral", "短暂的")],
            )
            .unwrap();
        database
            .import_dictionary(
                &DictionaryMeta {
                    id: "dict-b".into(),
                    name: "Dict B".into(),
                    source_language: Language::En,
                    gloss_language: Language::Zh,
                },
                &[dict_entry("ephemeral", "转瞬即逝的")],
            )
            .unwrap();
        let engine = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "should-not-be-called".into(),
        }))
        .with_dictionary(Some(Arc::new(SqliteDictionaryProvider::new(database))));
        let result = engine.translate(&request("ephemeral")).await.unwrap();
        assert_eq!(result.dictionaries.len(), 2);
        assert_eq!(result.dictionaries[0].source, "Dict A");
        assert_eq!(result.dictionaries[1].source, "Dict B");
        assert_eq!(result.translated_text, "短暂的");
    }

    #[tokio::test]
    async fn phrase_uses_dictionary_without_ai() {
        // 多词词组也查词典（词库里有 "15 minutes of fame" 这类词头）。
        let engine = engine_with_dict(
            "should-not-be-called",
            vec![dict_entry("an ephemeral moment", "朝露之瞬")],
        )
        .await;
        let result = engine
            .translate(&request("an ephemeral moment"))
            .await
            .unwrap();
        assert_eq!(result.translated_text, "朝露之瞬");
        assert_eq!(result.dictionaries[0].lemma, "an ephemeral moment");
    }

    #[tokio::test]
    async fn long_cjk_phrase_uses_dictionary() {
        // CJK 超 4 字判为词组，同样走词典（此前只有 ≤4 字的「单词」走）。
        let engine = engine_with_dict(
            "should-not-be-called",
            vec![dict_entry("人工智能大模型", "大规格 AI 模型")],
        )
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
        assert!(result.dictionaries.is_empty());
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

    #[test]
    fn same_language_rule_defaults_and_swap() {
        // 默认（未配置换向）：保持旧行为措辞。
        let plain = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "{}".into(),
        }));
        let (system, _) = plain.render_prompt(&request("hello"));
        assert!(
            system.contains("still translate it faithfully"),
            "默认 JSON 规则缺失: {system}"
        );

        // 配置换向（目标 zh → 换 en）：注入换向指令；换向语言与目标
        // 语言相同（en→en）等于不换向，仍是默认措辞。
        let swapping = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "{}".into(),
        }))
        .with_swap_language(Some(Language::En));
        let (system, _) = swapping.render_prompt(&request("hello"));
        assert!(system.contains("translate it into `en` instead"));

        let self_swap = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "{}".into(),
        }))
        .with_swap_language(Some(Language::Zh));
        let (system, _) = self_swap.render_prompt(&request("hello"));
        assert!(
            system.contains("still translate it faithfully"),
            "换向语言 == 目标语言时应视为不换向: {system}"
        );

        // 流式路径的换向措辞（无 explanation 字段）。
        let stream_rule = swapping.same_language_rule(&request("hello"), true);
        assert!(stream_rule.contains("translate it into `en` instead and output only that"));
    }

    #[test]
    fn prompt_override_replaces_system_only() {
        // 自定义 system 生效（JSON 与流式两条路径），user 模板不受影响。
        let engine = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "{}".into(),
        }))
        .with_prompt_override(PromptOverride {
            translate_system: Some("CUSTOM JSON RULES {{target_language}}".into()),
            translate_stream_system: Some("CUSTOM STREAM RULES".into()),
        });

        let (system, user) = engine.render_prompt(&request("hello"));
        assert!(system.contains("CUSTOM JSON RULES zh"), "{system}");
        assert!(user.contains("hello"), "user 模板仍带原文");

        let stream_system = engine
            .prompt_override
            .translate_stream_system
            .clone()
            .unwrap_or_default();
        assert_eq!(stream_system, "CUSTOM STREAM RULES");
    }

    #[test]
    fn prompt_override_blank_falls_back_to_builtin() {
        let engine = TranslationEngine::new(Arc::new(ScriptedProvider {
            response: "{}".into(),
        }))
        .with_prompt_override(PromptOverride::normalized(Some("   "), Some("")));
        let (system, _) = engine.render_prompt(&request("hello"));
        assert!(
            system.contains("STRICT JSON only"),
            "空白覆盖应回退内置模板: {system}"
        );
        assert!(engine.prompt_override.translate_system.is_none());
        assert!(engine.prompt_override.translate_stream_system.is_none());
    }
}
