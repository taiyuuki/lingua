//! 词典抽象层。
//!
//! 上层（Translation Engine、未来的生词本/学习系统）只面向
//! [`DictionaryProvider`] trait。任何词典来源（本地 SQLite、在线词典
//! API、Wiktionary）实现该 trait 后即可接入；原始数据格式由导入层
//! 转换为 [`DictionaryEntry`](lingua_core::DictionaryEntry) 统一模型。

pub mod mdx;
pub mod provider;
pub mod tsv;

pub use provider::{DictionaryProvider, SqliteDictionaryProvider};
pub use tsv::parse_tsv;
