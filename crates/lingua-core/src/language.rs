//! 语言标识。
//!
//! Phase 0 只保留「基础语言」粒度（`zh-CN` 与 `zh-TW` 暂时都归一为
//! [`Language::Zh`]）。未来需要区分地区变体时，在这里扩展，
//! 不允许散落到业务代码里手写字符串比较。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 支持的语言。未知代码统一进 [`Language::Other`]，避免解析失败。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Language {
    Zh,
    En,
    Ja,
    Ko,
    Fr,
    De,
    Es,
    Ru,
    Other(String),
}

impl Language {
    /// BCP-47 基础码（小写），如 `"zh"`、`"en"`；未知语言保留原始小写码。
    pub fn code(&self) -> &str {
        match self {
            Language::Zh => "zh",
            Language::En => "en",
            Language::Ja => "ja",
            Language::Ko => "ko",
            Language::Fr => "fr",
            Language::De => "de",
            Language::Es => "es",
            Language::Ru => "ru",
            Language::Other(code) => code.as_str(),
        }
    }

    /// 从任意 BCP-47 / locale 字符串解析：忽略地区与书写系统子标签
    /// （`zh-CN`、`en_US`、`ja-JP` 都取基础码）。空串归一为 `und`。
    pub fn parse(code: &str) -> Language {
        let base = code
            .trim()
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_lowercase();
        match base.as_str() {
            "zh" => Language::Zh,
            "en" => Language::En,
            "ja" => Language::Ja,
            "ko" => Language::Ko,
            "fr" => Language::Fr,
            "de" => Language::De,
            "es" => Language::Es,
            "ru" => Language::Ru,
            "" => Language::Other("und".to_string()),
            _ => Language::Other(base),
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl From<String> for Language {
    fn from(code: String) -> Self {
        Language::parse(&code)
    }
}

impl From<Language> for String {
    fn from(language: Language) -> Self {
        language.code().to_string()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parse_strips_region_and_case() {
        assert_eq!(Language::parse("zh-CN"), Language::Zh);
        assert_eq!(Language::parse("en_US"), Language::En);
        assert_eq!(Language::parse(" JA-JP "), Language::Ja);
    }

    #[test]
    fn unknown_and_empty_codes_do_not_fail() {
        assert_eq!(Language::parse("tok"), Language::Other("tok".into()));
        assert_eq!(Language::parse(""), Language::Other("und".into()));
    }

    #[test]
    fn code_roundtrip() {
        assert_eq!(Language::Zh.code(), "zh");
        assert_eq!(Language::parse("de-AT").code(), "de");
        assert_eq!(Language::parse("tok").to_string(), "tok");
    }

    #[test]
    fn serde_uses_bcp47_code() {
        let json = serde_json::to_value(Language::Ja).unwrap();
        assert_eq!(json, serde_json::json!("ja"));
        let parsed: Language = serde_json::from_value(serde_json::json!("fr-CA")).unwrap();
        assert_eq!(parsed, Language::Fr);
    }
}
