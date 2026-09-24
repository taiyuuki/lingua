//! 词典领域模型 —— 所有词典数据的统一契约。
//!
//! UI 与业务层只认 [`DictionaryEntry`]，不感知原始格式
//! （TSV / CC-CEDICT / JMdict / Wiktionary 均由导入层转换为该模型）。
//! 一个词条包含按「释义」粒度展开的列表，每条释义自带词性、
//! 例句与同/反义词。

use crate::language::Language;
use serde::{Deserialize, Serialize};

/// 一条释义（含该释义专属的例句与同/反义词）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryDefinition {
    /// 词性，如 `"noun"` / `"verb"` / `"n."`；原始数据缺失时为 `None`。
    pub part_of_speech: Option<String>,
    pub definition: String,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub synonyms: Vec<String>,
    #[serde(default)]
    pub antonyms: Vec<String>,
}

/// 一个词条。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryEntry {
    /// 词头（原样保留大小写）。
    pub lemma: String,
    /// 词头语言。
    pub language: Language,
    /// 释义语言（如 en→zh 词典为 `Zh`）。引擎据此决定该词条
    /// 是否适用于当前翻译方向。
    pub gloss_language: Language,
    /// 发音标注（IPA、拼音等），可多条。
    #[serde(default)]
    pub pronunciations: Vec<String>,
    /// 词形变化（复数、过去式……）。
    #[serde(default)]
    pub inflections: Vec<String>,
    /// 来源词典标识。
    pub source: String,
    pub definitions: Vec<DictionaryDefinition>,
}

impl DictionaryEntry {
    /// 词条是否至少有一条可用释义。
    pub fn has_definitions(&self) -> bool {
        self.definitions
            .iter()
            .any(|d| !d.definition.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn sample() -> DictionaryEntry {
        DictionaryEntry {
            lemma: "ephemeral".into(),
            language: Language::En,
            gloss_language: Language::Zh,
            pronunciations: vec!["/ɪˈfem.ər.əl/".into()],
            inflections: vec![],
            source: "test-dict".into(),
            definitions: vec![DictionaryDefinition {
                part_of_speech: Some("adj".into()),
                definition: "短暂的；转瞬即逝的".into(),
                examples: vec!["an ephemeral moment".into()],
                synonyms: vec!["transient".into()],
                antonyms: vec!["permanent".into()],
            }],
        }
    }

    #[test]
    fn serializes_camel_case() {
        let json = serde_json::to_value(sample()).unwrap();
        assert_eq!(json["glossLanguage"], "zh");
        assert_eq!(json["definitions"][0]["partOfSpeech"], "adj");
        assert_eq!(json["definitions"][0]["examples"][0], "an ephemeral moment");
    }

    #[test]
    fn roundtrip_via_json() {
        let entry = sample();
        let back: DictionaryEntry =
            serde_json::from_str(&serde_json::to_string(&entry).unwrap()).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn has_definitions_ignores_blank() {
        let mut entry = sample();
        assert!(entry.has_definitions());
        entry.definitions[0].definition = "  ".into();
        assert!(!entry.has_definitions());
    }
}
