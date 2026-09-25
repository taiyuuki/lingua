//! 生词本 DAO：`vocabulary` / `vocabulary_reviews`。
//!
//! 两条写入路径：
//! * [`Database::upsert_vocabulary`] —— 划词自动记录（`bump_lookup = true`
//!   时 `lookup_count` 自增）与 Popup 手动收藏共用；
//! * [`Database::record_review`] —— 复习打点（SRS 算法后续阶段）。
//!
//! 查询侧 [`Database::list_vocabulary`] 支持搜索 / 标签 / 语言 /
//! 掌握度筛选，按最近查看排序。

use crate::{Database, db_err};
use lingua_core::{AppResult, Language, Mastery, VocabularyItem};
use rusqlite::{OptionalExtension, params, params_from_iter};

/// 一次生词条目的写入（划词自动记录或手动收藏）。
#[derive(Debug, Clone)]
pub struct VocabularyUpsert {
    pub word: String,
    pub language: Language,
    pub translation: Option<String>,
    pub source: String,
    pub examples: Vec<String>,
    /// `true` = 划词自动记录，`lookup_count` 自增；`false` = 手动收藏。
    pub bump_lookup: bool,
}

/// 生词列表筛选条件。
#[derive(Debug, Clone, Default)]
pub struct VocabularyFilter {
    /// 模糊匹配 word / translation / notes。
    pub search: Option<String>,
    pub tag: Option<String>,
    pub language: Option<Language>,
    pub mastery: Option<Mastery>,
    /// 上限，默认 500，避免一次性拉全量。
    pub limit: u32,
    /// 分页偏移（与 limit 搭配）。
    pub offset: u32,
}

fn json_array(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

/// 筛选条件 → (where 子句, 绑定值)，list/count 共用。
fn vocabulary_conditions(filter: &VocabularyFilter) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(search) = filter
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let pattern = format!("%{search}%");
        conditions
            .push("(word LIKE ?1 OR translation LIKE ?1 OR notes LIKE ?1)".replace("?1", "?"));
        values.push(Box::new(pattern.clone()));
        values.push(Box::new(pattern.clone()));
        values.push(Box::new(pattern));
    }
    if let Some(tag) = filter
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        conditions.push(
            "EXISTS (SELECT 1 FROM json_each(vocabulary.tags) WHERE json_each.value = ?)"
                .to_string(),
        );
        values.push(Box::new(tag.to_string()));
    }
    if let Some(language) = &filter.language {
        conditions.push("language = ?".to_string());
        values.push(Box::new(language.code().to_string()));
    }
    if let Some(mastery) = filter.mastery {
        conditions.push("mastery = ?".to_string());
        values.push(Box::new(mastery.to_db()));
    }

    let where_clause = if conditions.is_empty() {
        "1=1".to_string()
    } else {
        conditions.join(" AND ")
    };
    (where_clause, values)
}

fn to_json_array(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string())
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<VocabularyItem> {
    Ok(VocabularyItem {
        id: row.get(0)?,
        word: row.get(1)?,
        language: Language::parse(&row.get::<_, String>(2)?),
        translation: row.get(3)?,
        notes: row.get(4)?,
        tags: json_array(&row.get::<_, String>(5)?),
        mastery: Mastery::from_db(row.get::<_, i64>(6)?),
        lookup_count: row.get::<_, i64>(7)? as u32,
        review_count: row.get::<_, i64>(8)? as u32,
        source: row.get(9)?,
        examples: json_array(&row.get::<_, String>(10)?),
        created_at: row.get(11)?,
        last_seen_at: row.get(12)?,
    })
}

const ITEM_COLUMNS: &str = "id, word, language, translation, notes, tags, mastery, \
     lookup_count, review_count, source, examples, created_at, last_seen_at";

impl Database {
    /// 插入或更新一条生词记录（按 `word_lower + language` 唯一）。
    ///
    /// 更新语义：`translation` 为 `None` 时保留旧值；`examples` / `source`
    /// 非空才覆盖；`lookup_count` 按需自增；`last_seen_at` 总是刷新；
    /// 笔记 / 标签 / 掌握度**永不被自动写入覆盖**（那是用户数据，
    /// 只能经 [`Database::update_vocabulary`] 修改）。
    pub fn upsert_vocabulary(&self, record: &VocabularyUpsert) -> AppResult<i64> {
        let word = record.word.trim();
        if word.is_empty() {
            return Err(lingua_core::AppError::InvalidInput(
                "vocabulary word is required".to_string(),
            ));
        }
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO vocabulary
                (word, word_lower, language, translation, source, examples,
                 lookup_count, last_seen_at, due_at, interval_days)
             VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 1)
             ON CONFLICT(word_lower, language) DO UPDATE SET
                translation = COALESCE(excluded.translation, vocabulary.translation),
                source = CASE WHEN excluded.source != '' THEN excluded.source
                              ELSE vocabulary.source END,
                examples = CASE WHEN excluded.examples != '[]' THEN excluded.examples
                                ELSE vocabulary.examples END,
                lookup_count = vocabulary.lookup_count + ?7,
                last_seen_at = excluded.last_seen_at",
            params![
                word,
                word.to_lowercase(),
                record.language.code(),
                record.translation,
                record.source.trim(),
                to_json_array(&record.examples),
                i64::from(record.bump_lookup),
            ],
        )
        .map_err(db_err)?;
        let id = conn
            .query_row(
                "SELECT id FROM vocabulary WHERE word_lower = ?1 AND language = ?2",
                params![word.to_lowercase(), record.language.code()],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| lingua_core::AppError::Database("upserted row not found".to_string()))?;
        Ok(id)
    }

    /// 按筛选条件列出生词，最近查看在前。
    pub fn list_vocabulary(&self, filter: &VocabularyFilter) -> AppResult<Vec<VocabularyItem>> {
        let (where_clause, values) = vocabulary_conditions(filter);
        let limit = if filter.limit == 0 { 500 } else { filter.limit };
        let sql = format!(
            "SELECT {ITEM_COLUMNS} FROM vocabulary WHERE {where_clause} \
             ORDER BY last_seen_at DESC, id DESC LIMIT {limit} OFFSET {}",
            filter.offset
        );

        let conn = self.lock_conn()?;
        let mut statement = conn.prepare(&sql).map_err(db_err)?;
        let rows = statement
            .query_map(
                params_from_iter(values.iter().map(|value| value.as_ref())),
                row_to_item,
            )
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// 与筛选条件匹配的词条总数（分页用）。
    pub fn count_vocabulary(&self, filter: &VocabularyFilter) -> AppResult<usize> {
        let (where_clause, values) = vocabulary_conditions(filter);
        let sql = format!("SELECT COUNT(*) FROM vocabulary WHERE {where_clause}");
        let conn = self.lock_conn()?;
        conn.query_row(
            &sql,
            params_from_iter(values.iter().map(|value| value.as_ref())),
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count as usize)
        .map_err(db_err)
    }

    /// 用户编辑：笔记 / 标签 / 掌握度。返回是否存在该条。
    pub fn update_vocabulary(
        &self,
        id: i64,
        notes: &str,
        tags: &[String],
        mastery: Mastery,
    ) -> AppResult<bool> {
        let conn = self.lock_conn()?;
        let changed = conn
            .execute(
                "UPDATE vocabulary SET notes = ?1, tags = ?2, mastery = ?3 WHERE id = ?4",
                params![notes, to_json_array(tags), mastery.to_db(), id],
            )
            .map_err(db_err)?;
        Ok(changed > 0)
    }

    /// 按「词 + 语言」查找生词记录（弹窗星标状态查询用）。
    pub fn find_vocabulary(
        &self,
        word: &str,
        language: Language,
    ) -> AppResult<Option<VocabularyItem>> {
        let conn = self.lock_conn()?;
        conn.query_row(
            &format!(
                "SELECT {ITEM_COLUMNS} FROM vocabulary WHERE word_lower = ?1 AND language = ?2"
            ),
            params![word.trim().to_lowercase(), language.code()],
            row_to_item,
        )
        .optional()
        .map_err(db_err)
    }

    /// 按「词 + 语言」删除生词（弹窗取消收藏用）。返回是否存在。
    pub fn delete_vocabulary_word(&self, word: &str, language: Language) -> AppResult<bool> {
        let conn = self.lock_conn()?;
        let changed = conn
            .execute(
                "DELETE FROM vocabulary WHERE word_lower = ?1 AND language = ?2",
                params![word.trim().to_lowercase(), language.code()],
            )
            .map_err(db_err)?;
        Ok(changed > 0)
    }

    pub fn delete_vocabulary(&self, id: i64) -> AppResult<bool> {
        let conn = self.lock_conn()?;
        let changed = conn
            .execute("DELETE FROM vocabulary WHERE id = ?1", params![id])
            .map_err(db_err)?;
        Ok(changed > 0)
    }

    /// 批量删除（单事务），返回实际删除条数。
    pub fn batch_delete_vocabulary(&self, ids: &[i64]) -> AppResult<usize> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(db_err)?;
        let mut deleted = 0usize;
        for id in ids {
            deleted += tx
                .execute("DELETE FROM vocabulary WHERE id = ?1", params![id])
                .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        Ok(deleted)
    }

    /// 批量更新（单事务）：设掌握度、追加标签（去重）。
    /// 返回实际更新的条数。
    pub fn batch_update_vocabulary(
        &self,
        ids: &[i64],
        mastery: Option<Mastery>,
        add_tags: &[String],
    ) -> AppResult<usize> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(db_err)?;
        let mut updated = 0usize;
        for id in ids {
            let tags: Vec<String> = tx
                .query_row(
                    "SELECT tags FROM vocabulary WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(db_err)
                .and_then(|raw| {
                    serde_json::from_str::<Vec<String>>(&raw).map_err(|error| {
                        crate::db_err(rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
                    })
                })
                .unwrap_or_default();
            let mut merged = tags;
            for tag in add_tags {
                if !merged.contains(tag) {
                    merged.push(tag.clone());
                }
            }
            let tags_json = serde_json::to_string(&merged)
                .map_err(|error| db_err(rusqlite::Error::ToSqlConversionFailure(Box::new(error))))?;
            let changed = match mastery {
                Some(mastery) => {
                    tx.execute(
                        "UPDATE vocabulary SET mastery = ?2, tags = ?3 WHERE id = ?1",
                        params![id, mastery.to_db(), tags_json],
                    )
                    .map_err(db_err)?
                }
                None if !add_tags.is_empty() => {
                    tx.execute(
                        "UPDATE vocabulary SET tags = ?2 WHERE id = ?1",
                        params![id, tags_json],
                    )
                    .map_err(db_err)?
                }
                None => 0,
            };
            updated += changed;
        }
        tx.commit().map_err(db_err)?;
        Ok(updated)
    }

    /// 复习评分（SRS-lite）：写 `vocabulary_reviews` 打点 +
    /// `review_count` 自增 + 重算到期时间/间隔/掌握度。
    ///
    /// `result`：0=again（忘记）1=good（记得）2=easy（简单）。
    /// 间隔阶梯 `[1,2,4,7,15,30,60]` 天：忘记回 1 天并回到「新词」；
    /// 记得升一级（学习中）；简单直接「已掌握」+ 30 天。
    pub fn grade_review(&self, id: i64, result: i64) -> AppResult<bool> {
        const LADDER: [i64; 7] = [1, 2, 4, 7, 15, 30, 60];
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(db_err)?;
        let (current_interval, current_mastery): (i64, i64) = tx
            .query_row(
                "SELECT interval_days, mastery FROM vocabulary WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| {
                lingua_core::AppError::Database("vocabulary item not found".to_string())
            })?;

        let (next_interval, next_mastery) = match result {
            0 => (1, 0),
            1 => {
                let interval = LADDER
                    .iter()
                    .find(|&&days| days > current_interval)
                    .copied()
                    .unwrap_or(60);
                // 已掌握的词温习保持已掌握，其余至少「学习中」。
                (interval, current_mastery.max(1))
            }
            _ => (30, 2),
        };

        tx.execute(
            "INSERT INTO vocabulary_reviews (vocab_id, result) VALUES (?1, ?2)",
            params![id, result],
        )
        .map_err(db_err)?;
        let changed = tx
            .execute(
                "UPDATE vocabulary
                 SET review_count = review_count + 1,
                     last_seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     interval_days = ?2,
                     mastery = ?3,
                     due_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+' || ?2 || ' days')
                 WHERE id = ?1",
                params![id, next_interval, next_mastery],
            )
            .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(changed > 0)
    }

    /// 到期待复习条数（due_at 非空且 <= 现在）。
    pub fn due_vocabulary_count(&self) -> AppResult<usize> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT COUNT(*) FROM vocabulary
             WHERE due_at != '' AND due_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count as usize)
        .map_err(db_err)
    }

    /// 到期复习队列（最早到期优先），每次最多取 `limit` 条。
    pub fn list_due_vocabulary(&self, limit: u32) -> AppResult<Vec<VocabularyItem>> {
        let sql = format!(
            "SELECT {ITEM_COLUMNS} FROM vocabulary
             WHERE due_at != '' AND due_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             ORDER BY due_at ASC, id ASC LIMIT {limit}"
        );
        let conn = self.lock_conn()?;
        let mut statement = conn.prepare(&sql).map_err(db_err)?;
        let rows = statement
            .query_map([], row_to_item)
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// 全部去重标签（筛选下拉用）。
    pub fn vocabulary_tags(&self) -> AppResult<Vec<String>> {
        let conn = self.lock_conn()?;
        let mut statement = conn
            .prepare(
                "SELECT DISTINCT json_each.value
                 FROM vocabulary, json_each(vocabulary.tags)
                 ORDER BY json_each.value",
            )
            .map_err(db_err)?;
        let rows = statement
            .query_map([], |row| row.get(0))
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use lingua_core::AppError;

    fn lookup(word: &str) -> VocabularyUpsert {
        VocabularyUpsert {
            word: word.into(),
            language: Language::En,
            translation: Some(format!("{word} 的译文")),
            source: "ai".into(),
            examples: vec![format!("{word} example")],
            bump_lookup: true,
        }
    }

    #[test]
    fn upsert_accumulates_and_preserves_user_data() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();

        let id = db.upsert_vocabulary(&lookup("ephemeral")).unwrap();
        db.upsert_vocabulary(&lookup("ephemeral")).unwrap();

        let items = db.list_vocabulary(&VocabularyFilter::default()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, id);
        assert_eq!(items[0].lookup_count, 2);
        assert_eq!(items[0].mastery, Mastery::New);

        // 用户数据写入后，再次自动记录不应覆盖。
        db.update_vocabulary(id, "我的笔记", &["gre".into()], Mastery::Mastered)
            .unwrap();
        db.upsert_vocabulary(&lookup("ephemeral")).unwrap();
        let items = db.list_vocabulary(&VocabularyFilter::default()).unwrap();
        assert_eq!(items[0].lookup_count, 3);
        assert_eq!(items[0].notes, "我的笔记");
        assert_eq!(items[0].tags, vec!["gre"]);
        assert_eq!(items[0].mastery, Mastery::Mastered);
    }

    #[test]
    fn manual_save_does_not_bump_lookup() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let manual = VocabularyUpsert {
            word: "serendipity".into(),
            language: Language::En,
            translation: None,
            source: "manual".into(),
            examples: vec![],
            bump_lookup: false,
        };
        db.upsert_vocabulary(&manual).unwrap();
        let items = db.list_vocabulary(&VocabularyFilter::default()).unwrap();
        assert_eq!(items[0].lookup_count, 0);
        assert_eq!(items[0].translation, None);
        assert_eq!(items[0].source, "manual");
    }

    #[test]
    fn same_word_different_languages_are_distinct() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let mut ja = lookup("猫");
        ja.language = Language::Ja;
        db.upsert_vocabulary(&lookup("猫")).unwrap();
        db.upsert_vocabulary(&ja).unwrap();
        assert_eq!(
            db.list_vocabulary(&VocabularyFilter::default())
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn filters_search_tag_mastery() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();

        db.upsert_vocabulary(&lookup("ephemeral")).unwrap();
        let id = db.upsert_vocabulary(&lookup("serendipity")).unwrap();
        db.update_vocabulary(
            id,
            "意外发现的运气",
            &["gre".into(), "noun".into()],
            Mastery::Learning,
        )
        .unwrap();
        db.upsert_vocabulary(&lookup("ubiquitous")).unwrap();

        let search = VocabularyFilter {
            search: Some("运气".into()),
            ..Default::default()
        };
        assert_eq!(db.list_vocabulary(&search).unwrap().len(), 1);

        let tag = VocabularyFilter {
            tag: Some("gre".into()),
            ..Default::default()
        };
        assert_eq!(db.list_vocabulary(&tag).unwrap().len(), 1);

        let mastered = VocabularyFilter {
            mastery: Some(Mastery::Mastered),
            ..Default::default()
        };
        assert!(db.list_vocabulary(&mastered).unwrap().is_empty());

        assert_eq!(db.vocabulary_tags().unwrap(), vec!["gre", "noun"]);
    }

    #[test]
    fn review_grading_reschedules_and_counts() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let id = db.upsert_vocabulary(&lookup("hello")).unwrap();
        // 新词入库即到期（interval=1）。
        assert_eq!(db.due_vocabulary_count().unwrap(), 1);

        // 记得：间隔 1→2，掌握度升为学习中，离开到期队列。
        assert!(db.grade_review(id, 1).unwrap());
        assert_eq!(db.due_vocabulary_count().unwrap(), 0);
        let item = db.find_vocabulary("hello", Language::En).unwrap().unwrap();
        assert_eq!(item.review_count, 1);
        assert_eq!(item.mastery, Mastery::Learning);

        // 直接把到期时间挪到现在，连续「记得」沿阶梯上升。
        let conn = db.lock_conn().unwrap();
        conn.execute(
            "UPDATE vocabulary SET due_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            params![id],
        )
        .unwrap();
        drop(conn);
        assert!(db.grade_review(id, 1).unwrap()); // 2→4
        assert!(db.grade_review(id, 1).unwrap()); // 4→7（已到期与否不影响调度）

        // 忘记：回 1 天、回到新词。
        assert!(db.grade_review(id, 0).unwrap());
        let item = db.find_vocabulary("hello", Language::En).unwrap().unwrap();
        assert_eq!(item.mastery, Mastery::New);
        // 简单：已掌握 + 30 天。
        assert!(db.grade_review(id, 2).unwrap());
        let item = db.find_vocabulary("hello", Language::En).unwrap().unwrap();
        assert_eq!(item.mastery, Mastery::Mastered);
        assert_eq!(item.review_count, 5);
    }

    #[test]
    fn due_queue_orders_oldest_first() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let a = db.upsert_vocabulary(&lookup("alpha")).unwrap();
        let b = db.upsert_vocabulary(&lookup("beta")).unwrap();
        // 手工设不同的到期时间：alpha 更早。
        let conn = db.lock_conn().unwrap();
        conn.execute(
            "UPDATE vocabulary SET due_at = '2020-01-01T00:00:00.000Z' WHERE id = ?1",
            params![a],
        )
        .unwrap();
        conn.execute(
            "UPDATE vocabulary SET due_at = '2021-01-01T00:00:00.000Z' WHERE id = ?1",
            params![b],
        )
        .unwrap();
        drop(conn);
        let due = db.list_due_vocabulary(10).unwrap();
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].word, "alpha");
        assert_eq!(due[1].word, "beta");
    }

    #[test]
    fn delete_and_missing_id() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        assert!(!db.delete_vocabulary(12345).unwrap());
        let id = db.upsert_vocabulary(&lookup("bye")).unwrap();
        assert!(db.delete_vocabulary(id).unwrap());
        assert!(
            db.list_vocabulary(&VocabularyFilter::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn find_and_delete_by_word() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        db.upsert_vocabulary(&lookup("Ephemeral")).unwrap();

        // 大小写不敏感命中
        let found = db.find_vocabulary("ephemeral", Language::En).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().lookup_count, 1);

        // 语言不同则查不到
        assert!(
            db.find_vocabulary("ephemeral", Language::Ja)
                .unwrap()
                .is_none()
        );

        assert!(
            db.delete_vocabulary_word("EPHEMERAL", Language::En)
                .unwrap()
        );
        assert!(
            db.find_vocabulary("ephemeral", Language::En)
                .unwrap()
                .is_none()
        );
        assert!(
            !db.delete_vocabulary_word("ephemeral", Language::En)
                .unwrap()
        );
    }

    #[test]
    fn blank_word_rejected() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let error = db.upsert_vocabulary(&lookup("   ")).unwrap_err();
        assert!(matches!(error, AppError::InvalidInput(_)));
    }
}
