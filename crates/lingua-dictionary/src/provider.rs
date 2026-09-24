//! [`DictionaryProvider`] trait 与 SQLite 实现。

use lingua_core::{AppError, AppResult, DictionaryEntry, Language};
use lingua_database::Database;

/// 词典查询抽象。实现必须是可并发共享的。
///
/// `language` 是词头语言提示：`None` 表示不限语言、跨词典查询；
/// 实现可以忽略提示（比如在线词典 API 只有固定方向）。
#[async_trait::async_trait]
pub trait DictionaryProvider: Send + Sync {
    /// Provider 名称（日志与 UI 展示用）。
    fn name(&self) -> &str;

    /// 查询词条；无命中返回空 Vec（这不是错误）。
    async fn lookup(
        &self,
        word: &str,
        language: Option<Language>,
    ) -> AppResult<Vec<DictionaryEntry>>;
}

/// 基于 SQLite（`lingua-database`）的本地词典实现：查询用户导入的
/// 全部词典。rusqlite 是阻塞的，查询放进 `spawn_blocking`。
#[derive(Clone)]
pub struct SqliteDictionaryProvider {
    database: Database,
}

impl SqliteDictionaryProvider {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
}

#[async_trait::async_trait]
impl DictionaryProvider for SqliteDictionaryProvider {
    fn name(&self) -> &str {
        "sqlite"
    }

    async fn lookup(
        &self,
        word: &str,
        language: Option<Language>,
    ) -> AppResult<Vec<DictionaryEntry>> {
        let database = self.database.clone();
        let word = word.trim().to_string();
        if word.is_empty() {
            return Ok(Vec::new());
        }
        tokio::task::spawn_blocking(move || database.lookup_entries(&word, language))
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use lingua_core::DictionaryDefinition;
    use lingua_database::DictionaryMeta;

    fn meta(id: &str) -> DictionaryMeta {
        DictionaryMeta {
            id: id.into(),
            name: id.into(),
            source_language: Language::En,
            gloss_language: Language::Zh,
        }
    }

    #[tokio::test]
    async fn sqlite_provider_lookups_via_trait() {
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        database
            .import_dictionary(
                &meta("t"),
                &[DictionaryEntry {
                    lemma: "serendipity".into(),
                    language: Language::En,
                    gloss_language: Language::Zh,
                    pronunciations: vec![],
                    inflections: vec![],
                    source: "t".into(),
                    definitions: vec![DictionaryDefinition {
                        part_of_speech: Some("noun".into()),
                        definition: "意外发现珍宝的运气".into(),
                        html: None,
                        examples: vec![],
                        synonyms: vec![],
                        antonyms: vec![],
                    }],
                }],
            )
            .unwrap();

        let provider = SqliteDictionaryProvider::new(database);
        assert_eq!(provider.name(), "sqlite");

        let hits = provider
            .lookup("Serendipity", Some(Language::En))
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].definitions[0].definition, "意外发现珍宝的运气");

        let miss = provider
            .lookup("nonexistent", Some(Language::En))
            .await
            .unwrap();
        assert!(miss.is_empty());

        let blank = provider.lookup("   ", None).await.unwrap();
        assert!(blank.is_empty());
    }
}
