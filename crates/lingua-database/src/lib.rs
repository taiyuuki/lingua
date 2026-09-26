//! SQLite 持久层。
//!
//! 设计约束（来自需求）：
//!
//! * schema 全部通过 [`MIGRATIONS`] 管理，用 `PRAGMA user_version` 追踪版本；
//!   禁止启动时动态改表、禁止手工 ALTER、禁止把 schema 藏进业务代码。
//! * UI 不直接触达本 crate —— 只有 application service 层可以调用。
//! * 业务数据不允许散落在 JSON 文件里；settings 表是唯一的 KV 例外。

pub mod audio;
pub mod dictionaries;
pub mod history;
pub mod migrations;
pub mod providers;
pub mod settings;
pub mod vocabulary;

pub use audio::{AudioHit, AudioKeyRecord, AudioLibraryInfo};
pub use dictionaries::{DictionaryInfo, DictionaryMeta};
pub use history::{HISTORY_RETENTION, HistoryFilter};
pub use migrations::{MIGRATIONS, Migration, latest_version};
pub use vocabulary::{VocabularyFilter, VocabularyUpsert};

use lingua_core::{AppError, AppResult};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// 一次 `migrate()` 的结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    /// 本次新应用的 migration 版本（幂等重跑时为空）。
    pub applied: Vec<u32>,
    /// 当前 schema 版本。
    pub current_version: u32,
}

/// SQLite 数据库句柄。内部 `Arc<Mutex<Connection>>`，可廉价 Clone 后
/// 交给 `spawn_blocking` 执行阻塞查询；单个连接 + 互斥锁对桌面应用的
/// 读写压力完全足够（WAL 模式下读不阻塞写，Phase 8 历史搜索量级也够）。
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// 打开（或创建）一个文件数据库，并设置基础 PRAGMA。
    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open(path).map_err(db_err)?;
        Self::init(conn)
    }

    /// 内存数据库，用于测试。
    pub fn open_in_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory().map_err(db_err)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> AppResult<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(db_err)?;
        // WAL 配 NORMAL：提交不再逐次 fsync。词典缓存可重导，
        // 断电最多丢最后几笔写入，换大批量导入的提交速度。
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(db_err)?;
        // 页缓存 64MB（默认 ~2MB）：百万级词条导入时 lemma 唯一索引
        // 按近似乱序写入，小缓存会持续换页，这是导入耗时的主因。
        conn.pragma_update(None, "cache_size", "-65536")
            .map_err(db_err)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(db_err)?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(db_err)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 应用所有未执行的 migration。幂等：重复调用是无操作。
    pub fn migrate(&self) -> AppResult<MigrationReport> {
        let mut conn = self.lock_conn()?;

        let mut current = read_schema_version(&conn)?;
        let max = latest_version();
        if current > max {
            return Err(AppError::Database(format!(
                "database schema v{current} is newer than supported v{max}; \
                 downgrading is not supported"
            )));
        }

        let mut applied = Vec::new();
        for migration in MIGRATIONS.iter() {
            if migration.version <= current {
                continue;
            }
            let tx = conn.transaction().map_err(db_err)?;
            tx.execute_batch(migration.sql).map_err(db_err)?;
            tx.pragma_update(None, "user_version", migration.version)
                .map_err(db_err)?;
            tx.commit().map_err(db_err)?;
            tracing::info!(
                version = migration.version,
                name = migration.name,
                "migration applied"
            );
            applied.push(migration.version);
            current = migration.version;
        }

        Ok(MigrationReport {
            applied,
            current_version: current,
        })
    }

    /// 当前 schema 版本（`PRAGMA user_version`）。
    pub fn schema_version(&self) -> AppResult<u32> {
        let conn = self.lock_conn()?;
        read_schema_version(&conn)
    }

    pub(crate) fn lock_conn(&self) -> AppResult<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| AppError::Database("database connection lock poisoned".to_string()))
    }
}

fn read_schema_version(conn: &Connection) -> AppResult<u32> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(db_err)
}

/// 统一把 rusqlite 错误折叠进 [`AppError::Database`]。
pub(crate) fn db_err(error: rusqlite::Error) -> AppError {
    AppError::Database(error.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use serde_json::json;

    #[test]
    fn migrations_are_versioned_from_one() {
        assert!(!MIGRATIONS.is_empty(), "at least one migration is required");
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                migration.version,
                (index + 1) as u32,
                "versions must be 1..N in order"
            );
        }
    }

    #[test]
    fn migrate_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let first = db.migrate().unwrap();
        assert_eq!(first.current_version, latest_version());
        assert_eq!(first.applied.len(), MIGRATIONS.len());

        let second = db.migrate().unwrap();
        assert!(second.applied.is_empty());
        assert_eq!(second.current_version, latest_version());
    }

    #[test]
    fn settings_roundtrip_and_overwrite() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();

        assert_eq!(db.get_setting("ui.theme").unwrap(), None);

        db.set_setting("ui.theme", &json!("light")).unwrap();
        db.set_setting("ui.theme", &json!("dark")).unwrap();
        assert_eq!(db.get_setting("ui.theme").unwrap(), Some(json!("dark")));

        let complex = json!({ "providers": [1, 2], "nested": { "a": true } });
        db.set_setting("misc", &complex).unwrap();
        assert_eq!(db.get_setting("misc").unwrap(), Some(complex));
    }

    #[test]
    fn remove_setting() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        db.set_setting("tmp", &json!(1)).unwrap();
        assert!(db.remove_setting("tmp").unwrap());
        assert!(!db.remove_setting("tmp").unwrap());
        assert_eq!(db.get_setting("tmp").unwrap(), None);
    }

    #[test]
    fn file_database_persists_schema_version() {
        let dir = std::env::temp_dir().join(format!("lingua-db-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lingua-test.sqlite3");

        {
            let db = Database::open(&path).unwrap();
            db.migrate().unwrap();
        }
        {
            let db = Database::open(&path).unwrap();
            assert_eq!(db.schema_version().unwrap(), latest_version());
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
