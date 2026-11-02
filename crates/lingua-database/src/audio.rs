//! 发音库 DAO：`audio_libraries` / `audio_keys`。
//!
//! 发音库是「词头 → 音频」的查表：导入期把 .mdd 里按词头命名的
//! 音频 key 提取成索引行，查词发音时按词头精确命中（大小写不敏感）。
//! 音频本体留在 .mdd 文件里按需读取，数据库只存索引。

use crate::{Database, db_err};
use lingua_core::{AppError, AppResult};
use rusqlite::params;
use serde::Serialize;

/// 发音库元信息（列表页展示用）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioLibraryInfo {
    pub id: String,
    pub name: String,
    /// 实际写入的词头音频条数。
    pub entry_count: usize,
    /// 覆盖的口音（"uk" / "us"，按固定顺序）。
    pub accents: Vec<String>,
}

/// 一条词头音频索引行（导入入参）。
#[derive(Debug, Clone)]
pub struct AudioKeyRecord {
    pub word_lower: String,
    /// "uk" | "us"。
    pub accent: String,
    /// mdd 内规范化资源路径。
    pub resource_path: String,
}

/// 一次词头音频命中的定位信息。
#[derive(Debug, Clone, PartialEq)]
pub struct AudioHit {
    pub library_id: String,
    pub accent: String,
    pub resource_path: String,
}

impl Database {
    /// 整体替换导入一个发音库（同 id 重导覆盖），返回实际写入条数。
    /// 词头索引按字典序插入（对 word_lower 索引接近追加写入）。
    pub fn replace_audio_library(
        &self,
        id: &str,
        name: &str,
        file_path: &str,
        entries: &[AudioKeyRecord],
    ) -> AppResult<usize> {
        if id.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "audio library id is required".to_string(),
            ));
        }
        if entries.is_empty() {
            return Err(AppError::InvalidInput(
                "audio library has no word-indexed audio".to_string(),
            ));
        }
        let mut ordered: Vec<&AudioKeyRecord> = entries.iter().collect();
        ordered.sort_by(|a, b| a.word_lower.cmp(&b.word_lower));

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(db_err)?;
        tx.execute("DELETE FROM audio_libraries WHERE id = ?1", params![id])
            .map_err(db_err)?;
        tx.execute(
            "INSERT INTO audio_libraries (id, name, file_path, entry_count)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, name.trim(), file_path, ordered.len() as i64],
        )
        .map_err(db_err)?;
        let written = {
            let mut statement = tx
                .prepare(
                    "INSERT OR IGNORE INTO audio_keys
                        (library_id, word_lower, accent, resource_path)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(db_err)?;
            let mut written = 0usize;
            for record in &ordered {
                written +=
                    statement.execute(params![id, record.word_lower, record.accent, record.resource_path])
                        .map_err(db_err)?;
            }
            written
        };
        tx.execute(
            "UPDATE audio_libraries SET entry_count = ?2 WHERE id = ?1",
            params![id, written as i64],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(written)
    }

    /// 全部发音库（按导入顺序），含各自覆盖的口音。
    pub fn list_audio_libraries(&self) -> AppResult<Vec<AudioLibraryInfo>> {
        let conn = self.lock_conn()?;
        let mut statement = conn
            .prepare(
                "SELECT id, name, entry_count,
                        EXISTS(SELECT 1 FROM audio_keys k WHERE k.library_id = a.id AND k.accent = 'uk'),
                        EXISTS(SELECT 1 FROM audio_keys k WHERE k.library_id = a.id AND k.accent = 'us')
                 FROM audio_libraries a ORDER BY a.created_at, a.id",
            )
            .map_err(db_err)?;
        let rows = statement
            .query_map([], |row| {
                let mut accents = Vec::new();
                if row.get::<_, bool>(3)? {
                    accents.push("uk".to_string());
                }
                if row.get::<_, bool>(4)? {
                    accents.push("us".to_string());
                }
                Ok(AudioLibraryInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    entry_count: row.get::<_, i64>(2)? as usize,
                    accents,
                })
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// 发音库 .mdd 的收编路径（资源服务按 id 取路径用）。
    pub fn audio_library_path(&self, id: &str) -> AppResult<Option<String>> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT file_path FROM audio_libraries WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(db_err(other)),
        })
    }

    /// 删除发音库（audio_keys 随级联删除）。
    pub fn delete_audio_library(&self, id: &str) -> AppResult<bool> {
        let conn = self.lock_conn()?;
        let changed = conn
            .execute("DELETE FROM audio_libraries WHERE id = ?1", params![id])
            .map_err(db_err)?;
        Ok(changed > 0)
    }

    /// 按词头查全部发音库的命中（按库导入顺序），不聚合口音。
    pub fn lookup_audio(&self, word_lower: &str) -> AppResult<Vec<AudioHit>> {
        let word_lower = word_lower.trim().to_lowercase();
        if word_lower.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock_conn()?;
        let mut statement = conn
            .prepare(
                "SELECT k.library_id, k.accent, k.resource_path
                 FROM audio_keys k
                 JOIN audio_libraries l ON l.id = k.library_id
                 WHERE k.word_lower = ?1
                 ORDER BY l.created_at, l.id",
            )
            .map_err(db_err)?;
        let rows = statement
            .query_map(params![word_lower], |row| {
                Ok(AudioHit {
                    library_id: row.get(0)?,
                    accent: row.get(1)?,
                    resource_path: row.get(2)?,
                })
            })
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

    fn record(word: &str, accent: &str, path: &str) -> AudioKeyRecord {
        AudioKeyRecord {
            word_lower: word.to_string(),
            accent: accent.to_string(),
            resource_path: path.to_string(),
        }
    }

    #[test]
    fn replace_lookup_and_delete_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();

        let entries = vec![
            record("hello", "uk", "hello_Uk.spx"),
            record("hello", "us", "hello.spx"),
            record("world", "us", "world.spx"),
            // 同 (词头, 口音) 重复：去重。
            record("hello", "us", "hello_2.spx"),
        ];
        let written = db
            .replace_audio_library("al-lib", "英式美式发音库", r"C:\app\audiolibs\al-lib.mdd", &entries)
            .unwrap();
        assert_eq!(written, 3);

        let libraries = db.list_audio_libraries().unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].id, "al-lib");
        assert_eq!(libraries[0].entry_count, 3);
        assert_eq!(libraries[0].accents, vec!["uk", "us"]);

        // 查词命中两路口音；词头大小写不敏感。
        let hits = db.lookup_audio("  HeLLo ").unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].library_id, "al-lib");
        assert_eq!(hits[0].accent, "uk");
        assert_eq!(hits[0].resource_path, "hello_Uk.spx");
        assert!(db.lookup_audio("missing").unwrap().is_empty());

        assert_eq!(
            db.audio_library_path("al-lib").unwrap().as_deref(),
            Some(r"C:\app\audiolibs\al-lib.mdd")
        );

        // 重导覆盖：同 id 替换为新索引。
        db.replace_audio_library(
            "al-lib",
            "新发音库",
            r"C:\app\audiolibs\al-lib.mdd",
            &[record("cat", "us", "cat.spx")],
        )
        .unwrap();
        assert!(db.lookup_audio("hello").unwrap().is_empty());
        assert_eq!(db.lookup_audio("cat").unwrap().len(), 1);
        let libraries = db.list_audio_libraries().unwrap();
        assert_eq!(libraries[0].name, "新发音库");
        assert_eq!(libraries[0].entry_count, 1);

        assert!(db.delete_audio_library("al-lib").unwrap());
        assert!(db.list_audio_libraries().unwrap().is_empty());
        assert!(db.lookup_audio("cat").unwrap().is_empty());
        assert!(!db.delete_audio_library("al-lib").unwrap());
    }

    #[test]
    fn multiple_libraries_order_by_import() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        db.replace_audio_library("al-a", "A", "a.mdd", &[record("cat", "us", "a/cat.spx")])
            .unwrap();
        db.replace_audio_library("al-b", "B", "b.mdd", &[record("cat", "uk", "b/cat_Uk.spx")])
            .unwrap();

        let hits = db.lookup_audio("cat").unwrap();
        assert_eq!(
            hits.iter().map(|hit| hit.library_id.as_str()).collect::<Vec<_>>(),
            vec!["al-a", "al-b"]
        );
    }
}
