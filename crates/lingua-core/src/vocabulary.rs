//! 生词本领域模型。
//!
//! 与词典分离（需求约束）：词典回答「这个词是什么意思」，
//! 生词本回答「这个词**对你**意味着什么」——查询次数、笔记、
//! 标签、掌握程度都是用户维度，不属于任何词典。

use crate::language::Language;
use serde::{Deserialize, Serialize};

/// 掌握程度。SRS（间隔重复）在后续阶段实现，这里只保留数据字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mastery {
    #[default]
    New,
    Learning,
    Mastered,
}

impl Mastery {
    /// 数据库存的是整数（0/1/2），未知值回退为 [`Mastery::New`]。
    pub fn from_db(value: i64) -> Self {
        match value {
            1 => Mastery::Learning,
            2 => Mastery::Mastered,
            _ => Mastery::New,
        }
    }

    pub fn to_db(self) -> i64 {
        match self {
            Mastery::New => 0,
            Mastery::Learning => 1,
            Mastery::Mastered => 2,
        }
    }
}

/// 一条生词记录。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyItem {
    pub id: i64,
    pub word: String,
    pub language: Language,
    /// 最近一次译文（自动记录或手动收藏时带上）。
    pub translation: Option<String>,
    pub notes: String,
    pub tags: Vec<String>,
    pub mastery: Mastery,
    /// 划词查询次数（自动路径累计；手动收藏不计）。
    pub lookup_count: u32,
    pub review_count: u32,
    /// 来源说明（词典 id / `"ai"` / `"manual"`）。
    pub source: String,
    /// 例句（来自词典命中时的释义例句）。
    pub examples: Vec<String>,
    /// ISO-8601 时间戳（SQLite 生成）。
    pub created_at: String,
    pub last_seen_at: String,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn mastery_db_roundtrip_and_defaults() {
        assert_eq!(Mastery::from_db(0), Mastery::New);
        assert_eq!(Mastery::from_db(1), Mastery::Learning);
        assert_eq!(Mastery::from_db(2), Mastery::Mastered);
        assert_eq!(Mastery::from_db(99), Mastery::New, "未知值回退 New");
        assert_eq!(Mastery::default(), Mastery::New);
        assert_eq!(Mastery::Mastered.to_db(), 2);
    }

    #[test]
    fn mastery_serializes_snake_case() {
        assert_eq!(serde_json::to_value(Mastery::Learning).unwrap(), "learning");
        let back: Mastery = serde_json::from_value(serde_json::json!("mastered")).unwrap();
        assert_eq!(back, Mastery::Mastered);
    }
}
