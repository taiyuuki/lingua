//! 翻译历史 DAO：`translation_history`。
//!
//! 纯追加日志：[`Database::record_history`] 写入并把总量裁剪到
//! [`HISTORY_RETENTION`] 条以内；查询侧 [`Database::list_history`]
//! 支持搜索（原文/译文 LIKE）与入口筛选，最近在前。

use crate::{Database, db_err};
use lingua_core::{AppError, AppResult, HistoryItem, HistoryKind, HistoryRecord, Language};
use rusqlite::{params, params_from_iter};

/// 历史保留上限（条）：超出时从最旧开始裁剪。
pub const HISTORY_RETENTION: usize = 5000;

/// 历史列表筛选条件。
#[derive(Debug, Clone, Default)]
pub struct HistoryFilter {
    /// 模糊匹配原文 / 译文。
    pub search: Option<String>,
    pub kind: Option<HistoryKind>,
    /// 上限，默认 200。
    pub limit: u32,
    /// 分页偏移（与 limit 搭配）。
    pub offset: u32,
}

/// 筛选条件 → (where 子句, 绑定值)，list/count 共用。
fn history_conditions(filter: &HistoryFilter) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(search) = filter
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let pattern = format!("%{search}%");
        conditions.push("(source_text LIKE ? OR translated_text LIKE ?)".to_string());
        values.push(Box::new(pattern.clone()));
        values.push(Box::new(pattern));
    }
    if let Some(kind) = filter.kind {
        conditions.push("entry_kind = ?".to_string());
        values.push(Box::new(kind.to_db().to_string()));
    }

    let where_clause = if conditions.is_empty() {
        "1=1".to_string()
    } else {
        conditions.join(" AND ")
    };
    (where_clause, values)
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryItem> {
    Ok(HistoryItem {
        id: row.get(0)?,
        source_text: row.get(1)?,
        translated_text: row.get(2)?,
        detected_language: row
            .get::<_, Option<String>>(3)?
            .map(|ref code| Language::parse(code)),
        target_language: Language::parse(&row.get::<_, String>(4)?),
        model: row.get(5)?,
        kind: HistoryKind::from_db(&row.get::<_, String>(6)?),
        dictionary_hit: row.get::<_, i64>(7)? != 0,
        created_at: row.get(8)?,
    })
}

const ITEM_COLUMNS: &str = "id, source_text, translated_text, detected_language, \
     target_language, model, entry_kind, dictionary_hit, created_at";

impl Database {
    /// 追加一条历史并裁剪总量。空原文/译文直接忽略（不报错——
    /// 历史记录是旁路写入，不该让主流程失败）。
    pub fn record_history(&self, record: &HistoryRecord) -> AppResult<i64> {
        let source = record.source_text.trim();
        let translated = record.translated_text.trim();
        if source.is_empty() || translated.is_empty() {
            return Err(AppError::InvalidInput(
                "history requires non-empty source and translated text".to_string(),
            ));
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(db_err)?;
        tx.execute(
            "INSERT INTO translation_history
                (source_text, translated_text, detected_language, target_language,
                 model, entry_kind, dictionary_hit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                source,
                translated,
                record
                    .detected_language
                    .as_ref()
                    .map(|language| language.code().to_string()),
                record.target_language.code(),
                record
                    .model
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty()),
                record.kind.to_db(),
                i64::from(record.dictionary_hit),
            ],
        )
        .map_err(db_err)?;
        let id = tx.last_insert_rowid();
        // 裁剪：只留最近 HISTORY_RETENTION 条。
        tx.execute(
            "DELETE FROM translation_history WHERE id NOT IN
             (SELECT id FROM translation_history ORDER BY id DESC LIMIT ?1)",
            params![HISTORY_RETENTION as i64],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(id)
    }

    /// 按筛选条件列出历史，最近在前。
    pub fn list_history(&self, filter: &HistoryFilter) -> AppResult<Vec<HistoryItem>> {
        let (where_clause, values) = history_conditions(filter);
        let limit = if filter.limit == 0 { 200 } else { filter.limit };
        let sql = format!(
            "SELECT {ITEM_COLUMNS} FROM translation_history WHERE {where_clause} \
             ORDER BY id DESC LIMIT {limit} OFFSET {}",
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

    /// 与筛选条件匹配的条数（分页用）。
    pub fn count_history(&self, filter: &HistoryFilter) -> AppResult<usize> {
        let (where_clause, values) = history_conditions(filter);
        let sql = format!("SELECT COUNT(*) FROM translation_history WHERE {where_clause}");
        let conn = self.lock_conn()?;
        conn.query_row(
            &sql,
            params_from_iter(values.iter().map(|value| value.as_ref())),
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count as usize)
        .map_err(db_err)
    }

    /// 删除单条。返回是否存在。
    pub fn delete_history(&self, id: i64) -> AppResult<bool> {
        let conn = self.lock_conn()?;
        let changed = conn
            .execute("DELETE FROM translation_history WHERE id = ?1", params![id])
            .map_err(db_err)?;
        Ok(changed > 0)
    }

    /// 清空全部历史。返回删除条数。
    pub fn clear_history(&self) -> AppResult<usize> {
        let conn = self.lock_conn()?;
        conn.execute("DELETE FROM translation_history", [])
            .map_err(db_err)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn record(source: &str, translated: &str) -> HistoryRecord {
        HistoryRecord {
            source_text: source.into(),
            translated_text: translated.into(),
            detected_language: Some(Language::En),
            target_language: Language::Zh,
            model: Some("test-model".into()),
            kind: HistoryKind::Selection,
            dictionary_hit: false,
        }
    }

    #[test]
    fn record_list_search_and_filters() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();

        let first = db
            .record_history(&record("hello world.", "你好，世界。"))
            .unwrap();
        let mut ocr = record("ephemeral", "短暂的");
        ocr.kind = HistoryKind::Ocr;
        ocr.dictionary_hit = true;
        ocr.model = None;
        db.record_history(&ocr).unwrap();

        // 最近在前。
        let all = db.list_history(&HistoryFilter::default()).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].source_text, "ephemeral");
        assert_eq!(all[0].kind, HistoryKind::Ocr);
        assert!(all[0].dictionary_hit);
        assert_eq!(all[0].model, None);
        assert_eq!(all[1].id, first);
        assert_eq!(all[1].detected_language, Some(Language::En));
        assert_eq!(all[1].target_language, Language::Zh);

        let search = HistoryFilter {
            search: Some("世界".into()),
            ..Default::default()
        };
        assert_eq!(db.list_history(&search).unwrap().len(), 1);
        assert_eq!(db.count_history(&search).unwrap(), 1);

        let by_kind = HistoryFilter {
            kind: Some(HistoryKind::Ocr),
            ..Default::default()
        };
        assert_eq!(db.count_history(&by_kind).unwrap(), 1);

        // 译文也能搜到。
        let by_translation = HistoryFilter {
            search: Some("短暂的".into()),
            ..Default::default()
        };
        assert_eq!(db.count_history(&by_translation).unwrap(), 1);
    }

    #[test]
    fn blank_text_rejected() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        assert_eq!(
            db.record_history(&record("   ", "译文"))
                .unwrap_err()
                .kind(),
            "invalid_input"
        );
        assert_eq!(
            db.record_history(&record("原文", "  ")).unwrap_err().kind(),
            "invalid_input"
        );
    }

    #[test]
    fn retention_prunes_oldest() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        // 写入 HISTORY_RETENTION + 5 条，最旧的 5 条应被裁掉。
        for index in 0..(HISTORY_RETENTION as i64 + 5) {
            db.record_history(&record(&format!("t{index}"), "译文"))
                .unwrap();
        }
        assert_eq!(
            db.count_history(&HistoryFilter::default()).unwrap(),
            HISTORY_RETENTION
        );
        // 全量拉取（limit=0 默认 200 不够，这里显式给满）。
        let all = db
            .list_history(&HistoryFilter {
                limit: HISTORY_RETENTION as u32,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(all.len(), HISTORY_RETENTION);
        assert_eq!(
            all[0].source_text,
            format!("t{}", HISTORY_RETENTION as i64 + 4)
        );
        assert_eq!(all.last().unwrap().source_text, "t5");
    }

    #[test]
    fn delete_and_clear() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let a = db.record_history(&record("a", "甲")).unwrap();
        db.record_history(&record("b", "乙")).unwrap();
        assert!(db.delete_history(a).unwrap());
        assert!(!db.delete_history(a).unwrap());
        assert_eq!(db.count_history(&HistoryFilter::default()).unwrap(), 1);
        assert_eq!(db.clear_history().unwrap(), 1);
        assert_eq!(db.count_history(&HistoryFilter::default()).unwrap(), 0);
    }

    #[test]
    fn pagination_offset_works() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        for index in 0..5 {
            db.record_history(&record(&format!("t{index}"), "译文"))
                .unwrap();
        }
        let page2 = HistoryFilter {
            limit: 2,
            offset: 2,
            ..Default::default()
        };
        let rows = db.list_history(&page2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].source_text, "t2");
        assert_eq!(rows[1].source_text, "t1");
    }
}
