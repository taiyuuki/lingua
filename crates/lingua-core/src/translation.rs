//! 翻译请求/结果的数据契约。
//!
//! 这里只定义「Translation Engine 的输入输出」；引擎本身（路由到
//! Dictionary / Translation Provider / AI）在 crate `lingua-translation`
//! 中实现。

use crate::dictionary::DictionaryEntry;
use crate::language::Language;
use serde::{Deserialize, Serialize};

/// 文本形态分类，用于引擎路由决策（词典只对单词生效）。
///
/// 这是启发式分类，不追求语言学精确：CJK 按字符数、其它按空格分词。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    Word,
    Phrase,
    Sentence,
}

/// 判断文本是否是 CJK 表意字符。
fn is_cjk(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}')
}

/// 粗略分类一段待翻译文本。
pub fn classify_text(text: &str) -> TextKind {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return TextKind::Word;
    }

    let has_terminal_punctuation = trimmed.chars().any(|c| {
        matches!(
            c,
            '.' | '!' | '?' | ';' | ':' | '。' | '！' | '？' | '；' | '：' | '…'
        )
    });
    if has_terminal_punctuation {
        return TextKind::Sentence;
    }

    let cjk_chars = trimmed.chars().filter(|c| is_cjk(*c)).count();
    let space_tokens = trimmed.split_whitespace().count();
    if cjk_chars > 0 && space_tokens > 1 {
        // 中英混排或多段 CJK：按短语处理。
        return TextKind::Phrase;
    }
    if cjk_chars > 0 {
        return if cjk_chars <= 4 {
            TextKind::Word
        } else {
            TextKind::Phrase
        };
    }
    if space_tokens <= 1 {
        TextKind::Word
    } else {
        TextKind::Phrase
    }
}

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
    /// 命中的词典词条（每本词典一条，按导入顺序，首页为首选词典）。
    /// Popup 据此渲染词典视图并支持多词典切换；AI 结果为空。
    #[serde(default)]
    pub dictionaries: Vec<DictionaryEntry>,
    /// 产生译文的模型名（仅 AI 路径；词典路径为 `None`）。
    /// Popup 用它作结果来源标签；模型未上报时前端回退到 Provider id。
    #[serde(default)]
    pub model: Option<String>,
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
        dictionaries: Vec::new(),
        model: Some("test-model".into()),
    };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("translatedText").is_some());
        assert!(json.get("detectedLanguage").is_some());
    }

    #[test]
    fn classify_recognizes_words_phrases_and_sentences() {
        assert_eq!(classify_text("ephemeral"), TextKind::Word);
        assert_eq!(classify_text("  ephemeral "), TextKind::Word);
        assert_eq!(classify_text("an ephemeral moment"), TextKind::Phrase);
        assert_eq!(classify_text("It is ephemeral."), TextKind::Sentence);
        assert_eq!(classify_text("你好"), TextKind::Word);
        assert_eq!(classify_text("人工智能"), TextKind::Word);
        assert_eq!(classify_text("今天天气真不错啊"), TextKind::Phrase);
        assert_eq!(classify_text("今天天气真不错。"), TextKind::Sentence);
        assert_eq!(classify_text("machine 学习"), TextKind::Phrase);
    }
}
