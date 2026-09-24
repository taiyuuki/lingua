//! settings 表访问（JSON 键值对）。
//!
//! 仅用于应用级偏好（UI 主题、窗口尺寸、AI Router 默认策略等）。
//! 结构化业务数据（providers、vocabulary、history…）必须用明确的表，
//! 不允许塞进 settings 打包成 JSON。

use crate::{Database, db_err};
use lingua_core::{AppError, AppResult};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

impl Database {
    /// 读取一个设置项；不存在返回 `None`。
    pub fn get_setting(&self, key: &str) -> AppResult<Option<Value>> {
        let conn = self.lock_conn()?;
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_err)?;

        match raw {
            Some(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|error| AppError::Database(format!("corrupt setting `{key}`: {error}"))),
            None => Ok(None),
        }
    }

    /// 写入（或覆盖）一个设置项。
    pub fn set_setting(&self, key: &str, value: &Value) -> AppResult<()> {
        let text = serde_json::to_string(value).map_err(|error| {
            AppError::InvalidInput(format!("setting `{key}` is not JSON: {error}"))
        })?;
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO settings (key, value, updated_at)
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at",
            params![key, text],
        )
        .map_err(db_err)?;
        Ok(())
    }

    /// 删除一个设置项；返回是否真的删除了。
    pub fn remove_setting(&self, key: &str) -> AppResult<bool> {
        let conn = self.lock_conn()?;
        let changed = conn
            .execute("DELETE FROM settings WHERE key = ?1", params![key])
            .map_err(db_err)?;
        Ok(changed > 0)
    }
}
