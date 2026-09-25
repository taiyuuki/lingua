//! 学习分析器（Phase 9）：单词深析 / 句子解构 → 结构化学习卡片。
//!
//! 与翻译引擎共享 Prompt 模板渲染器；要求模型输出严格 JSON，
//! 解析失败时降级——把整段响应当 `notes` 展示，绝不因格式偏差
//! 让整个分析报错（模型输出不可控是常态，用户仍能看到内容）。

use crate::template::{PromptTemplate, vars_from_pairs};
use lingua_ai::{AIProvider, types::ChatRequest};
use lingua_core::{AppResult, Language};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 学习分析 Prompt（编译期内嵌）。
mod prompts {
    pub const STUDY_SYSTEM: &str = include_str!("../prompts/study/system.md");
    pub const STUDY_USER: &str = include_str!("../prompts/study/user.md");
}

/// 一次学习分析请求。
#[derive(Debug, Clone)]
pub struct StudyRequest {
    pub text: String,
    /// 源语言提示（`None` = 自动检测）。
    pub source_language: Option<Language>,
    /// 释义/讲解使用的语言。
    pub target_language: Language,
    /// 已有的译文（Popup 结果态带上，帮助模型保持一致；可空）。
    pub translation: Option<String>,
}

/// 一条例句 / 释义例句。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyExample {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub translation: Option<String>,
}

/// 义项（单词路径）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudySense {
    #[serde(default)]
    pub pos: Option<String>,
    #[serde(default)]
    pub meaning: String,
    #[serde(default)]
    pub example: Option<String>,
    #[serde(default)]
    pub example_translation: Option<String>,
}

/// 词根词缀拆解（单词路径；非派生词可全空）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyMorphology {
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub affixes: Option<String>,
    #[serde(default)]
    pub cognates: Option<String>,
}

/// 近义词辨析。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudySynonym {
    #[serde(default)]
    pub word: String,
    #[serde(default)]
    pub nuance: Option<String>,
}

/// 关键短语（句子路径）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyKeyPhrase {
    #[serde(default)]
    pub phrase: String,
    #[serde(default)]
    pub meaning: Option<String>,
}

/// 学习分析结果。所有块都是可选——模型按文本类型填词块或句块，
/// 解析失败的降级场景下只有 `notes`（全文）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StudyAnalysis {
    /// `word` | `sentence`（模型自判；未知/缺失为 None）。
    pub kind: Option<String>,
    pub language: Option<String>,
    pub phonetic: Option<String>,
    #[serde(default)]
    pub parts_of_speech: Vec<String>,
    #[serde(default)]
    pub senses: Vec<StudySense>,
    pub morphology: Option<StudyMorphology>,
    #[serde(default)]
    pub collocations: Vec<String>,
    #[serde(default)]
    pub synonyms: Vec<StudySynonym>,
    pub structure: Option<String>,
    #[serde(default)]
    pub key_phrases: Vec<StudyKeyPhrase>,
    pub paraphrase: Option<String>,
    #[serde(default)]
    pub examples: Vec<StudyExample>,
    pub mnemonics: Option<String>,
    /// 降级时携带模型原始输出（strict JSON 失败的兜底展示）。
    pub notes: Option<String>,
}

impl StudyAnalysis {
    /// 是否为降级结果（没有任何结构化块，只有 notes）。
    pub fn is_degraded(&self) -> bool {
        self.senses.is_empty()
            && self.key_phrases.is_empty()
            && self.examples.is_empty()
            && self.morphology.is_none()
            && self.structure.is_none()
            && self.paraphrase.is_none()
            && self.mnemonics.is_none()
            && self.notes.is_some()
    }
}

/// 学习分析器：持有一个经 Router 选出的 Provider。
pub struct StudyAnalyzer {
    provider: Arc<dyn AIProvider>,
}

impl StudyAnalyzer {
    pub fn new(provider: Arc<dyn AIProvider>) -> Self {
        Self { provider }
    }

    /// 渲染 study prompt（system 的 `{{target_language}}` 与 user
    /// 模板共用一张变量表）。
    fn render_prompt(&self, request: &StudyRequest) -> (String, String) {
        let vars = vars_from_pairs(&[
            ("text", request.text.trim().to_string()),
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
            (
                "translation",
                request.translation.clone().unwrap_or_default(),
            ),
        ]);
        let template = PromptTemplate::from_parts(prompts::STUDY_SYSTEM, prompts::STUDY_USER);
        template.render(&vars)
    }

    /// 执行学习分析。网络/Provider 错误照常上抛；模型输出格式偏差
    /// 不报错，降级为 `notes` 全文。
    pub async fn analyze(&self, request: &StudyRequest) -> AppResult<StudyAnalysis> {
        let (system, user) = self.render_prompt(request);
        let chat_request = ChatRequest::simple(system, user);
        let response = self.provider.chat(&chat_request).await?;
        Ok(parse_study_analysis(
            response.content,
            response
                .model
                .or_else(|| Some(self.provider.id().to_string())),
        ))
    }
}

/// 解析模型响应；`_model` 保留参数位（未来结果标注来源用）。
fn parse_study_analysis(content: String, _model: Option<String>) -> StudyAnalysis {
    let Some(json_start) = content.find('{') else {
        return degraded(content);
    };
    let Some(json_end) = content.rfind('}') else {
        return degraded(content);
    };
    if json_end < json_start {
        return degraded(content);
    }
    serde_json::from_str::<StudyAnalysis>(&content[json_start..=json_end])
        .unwrap_or_else(|_| degraded(content))
}

/// 降级：结构化解析失败时保留原文，让用户至少看到模型说了什么。
fn degraded(content: String) -> StudyAnalysis {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return StudyAnalysis {
            notes: Some("（模型没有返回内容）".to_string()),
            ..StudyAnalysis::default()
        };
    }
    StudyAnalysis {
        notes: Some(trimmed.to_string()),
        ..StudyAnalysis::default()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use lingua_ai::AIProvider;
    use lingua_ai::types::{ChatResponse, ProviderCapabilities, VisionRequest};
    use lingua_core::AppError;

    struct ScriptedProvider {
        response: String,
    }

    #[async_trait::async_trait]
    impl AIProvider for ScriptedProvider {
        fn id(&self) -> &str {
            "scripted"
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::default()
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

    fn request(text: &str) -> StudyRequest {
        StudyRequest {
            text: text.into(),
            source_language: None,
            target_language: Language::Zh,
            translation: None,
        }
    }

    #[tokio::test]
    async fn parses_word_analysis() {
        let analyzer = StudyAnalyzer::new(Arc::new(ScriptedProvider {
            response: r#"```json
{"kind": "word", "language": "en", "phonetic": "/ɪˈfem(ə)rəl/",
 "parts_of_speech": ["adj"],
 "senses": [{"pos": "adj", "meaning": "短暂的", "example": "an ephemeral moment", "example_translation": "转瞬即逝的瞬间"}],
 "morphology": {"root": "Greek hemera (day)", "affixes": "epi- + hemera", "cognates": "ephemerality"},
 "collocations": ["ephemeral beauty", "ephemeral pleasure"],
 "synonyms": [{"word": "transient", "nuance": "更中性"}],
 "mnemonics": "希腊语 hemera = 一天，寿命只有一天"}
```"#
            .into(),
        }));
        let analysis = analyzer.analyze(&request("ephemeral")).await.unwrap();
        assert_eq!(analysis.kind.as_deref(), Some("word"));
        assert_eq!(analysis.phonetic.as_deref(), Some("/ɪˈfem(ə)rəl/"));
        assert_eq!(analysis.senses.len(), 1);
        assert_eq!(analysis.senses[0].meaning, "短暂的");
        assert!(analysis.morphology.is_some());
        assert!(!analysis.is_degraded());
    }

    #[tokio::test]
    async fn parses_sentence_analysis() {
        let analyzer = StudyAnalyzer::new(Arc::new(ScriptedProvider {
            response: r#"{"kind": "sentence", "structure": "主系表", "keyPhrases": [{"phrase": "on the fence", "meaning": "犹豫不决"}], "paraphrase": "I have not decided yet.", "examples": [{"text": "He is still on the fence.", "translation": "他还没决定。"}]}"#
                .into(),
        }));
        let analysis = analyzer
            .analyze(&request("I'm on the fence about it."))
            .await
            .unwrap();
        assert_eq!(analysis.kind.as_deref(), Some("sentence"));
        assert_eq!(analysis.key_phrases.len(), 1);
        assert_eq!(analysis.key_phrases[0].phrase, "on the fence");
        assert!(analysis.senses.is_empty());
    }

    #[tokio::test]
    async fn non_json_response_degrades_to_notes() {
        let analyzer = StudyAnalyzer::new(Arc::new(ScriptedProvider {
            response: "ephemeral 意为「短暂的」，来自希腊语。".into(),
        }));
        let analysis = analyzer.analyze(&request("ephemeral")).await.unwrap();
        assert!(analysis.is_degraded());
        assert!(analysis.notes.unwrap().contains("短暂的"));
    }

    #[tokio::test]
    async fn malformed_json_degrades_to_notes() {
        let analyzer = StudyAnalyzer::new(Arc::new(ScriptedProvider {
            response: "{\"kind\": \"word\", broken".into(),
        }));
        let analysis = analyzer.analyze(&request("ephemeral")).await.unwrap();
        assert!(analysis.is_degraded());
    }

    #[test]
    fn prompt_contains_text_and_target_language() {
        let analyzer = StudyAnalyzer::new(Arc::new(ScriptedProvider {
            response: "{}".into(),
        }));
        let (system, user) = analyzer.render_prompt(&request("ephemeral"));
        assert!(system.contains("zh"), "system 应含释义语言: {system}");
        assert!(user.contains("ephemeral"));
        assert!(user.contains("zh"));
    }

    #[test]
    fn empty_response_degrades_with_placeholder() {
        let analysis = degraded("  ".to_string());
        assert!(analysis.is_degraded());
        assert!(analysis.notes.unwrap().contains("没有返回内容"));
    }
}
