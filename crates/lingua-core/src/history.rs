//! 翻译历史领域模型。
//!
//! 历史是纯追加日志：记录「哪段文本 → 译成了什么」，不承载用户
//! 整理数据（那是生词本的职责）。划词与截图翻译共用一张表，
//! 用 [`HistoryKind`] 区分入口。

use crate::language::Language;
use serde::{Deserialize, Serialize};

/// 历史条目来源。反序列化时未知值回退为 [`HistoryKind::Selection`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryKind {
    #[default]
    Selection,
    Ocr,
}

impl HistoryKind {
    /// 数据库存的是小写标签，未知值回退为划词。
    pub fn from_db(value: &str) -> Self {
        match value {
            "ocr" => HistoryKind::Ocr,
            _ => HistoryKind::Selection,
        }
    }

    pub fn to_db(self) -> &'static str {
        match self {
            HistoryKind::Selection => "selection",
            HistoryKind::Ocr => "ocr",
        }
    }
}

/// 一条翻译历史。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: i64,
    pub source_text: String,
    pub translated_text: String,
    /// 识别/检测出的源语言；词典路径可能为 `None`。
    pub detected_language: Option<Language>,
    pub target_language: Language,
    /// AI 模型 / Provider 标签；词典命中为 `None`。
    pub model: Option<String>,
    pub kind: HistoryKind,
    /// 是否命中本地词典。
    pub dictionary_hit: bool,
    /// ISO-8601 时间戳（SQLite 生成）。
    pub created_at: String,
}

/// 历史写入载荷（`id` / `created_at` 由数据库生成）。
#[derive(Debug, Clone)]
pub struct HistoryRecord {
    pub source_text: String,
    pub translated_text: String,
    pub detected_language: Option<Language>,
    pub target_language: Language,
    pub model: Option<String>,
    pub kind: HistoryKind,
    pub dictionary_hit: bool,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn kind_db_roundtrip_and_defaults() {
        assert_eq!(HistoryKind::from_db("ocr"), HistoryKind::Ocr);
        assert_eq!(HistoryKind::from_db("selection"), HistoryKind::Selection);
        assert_eq!(HistoryKind::from_db("whatever"), HistoryKind::Selection);
        assert_eq!(HistoryKind::default(), HistoryKind::Selection);
        assert_eq!(HistoryKind::Ocr.to_db(), "ocr");
        assert_eq!(HistoryKind::Selection.to_db(), "selection");
    }

    #[test]
    fn kind_serializes_snake_case() {
        assert_eq!(serde_json::to_value(HistoryKind::Ocr).unwrap(), "ocr");
        let back: HistoryKind = serde_json::from_value(serde_json::json!("selection")).unwrap();
        assert_eq!(back, HistoryKind::Selection);
    }
}
