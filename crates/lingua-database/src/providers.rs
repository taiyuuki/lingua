//! `providers` 表 DAO。
//!
//! 这张表只存配置，**不存密钥**：`api_key_ref` 是密钥在 OS 密钥库中
//! 的引用（`provider/{id}`），由 `lingua-secrets` 负责读写明文。

use crate::{Database, db_err};
use lingua_ai::{ProviderCapabilities, ProviderConfig, ProviderKind};
use lingua_core::{AppError, AppResult};
use rusqlite::{OptionalExtension, Row, params};

/// 与 [`ProviderConfig`] 对应的持久化记录（不含明文密钥）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRecord {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: Option<String>,
    pub organization: Option<String>,
    pub timeout_secs: u64,
    pub can_chat: bool,
    pub can_translate: bool,
    pub can_vision: bool,
    /// 请求体附带 `{"thinking": {"type": "disabled"}}`（GLM 等）。
    pub disable_thinking: bool,
    pub api_key_ref: Option<String>,
}

impl ProviderRecord {
    /// 转成运行时配置；`api_key` 由调用方从密钥库填充。
    pub fn to_config(&self) -> ProviderConfig {
        ProviderConfig {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind,
            base_url: Some(self.base_url.clone()),
            model: self.model.clone(),
            organization: self.organization.clone(),
            timeout_secs: self.timeout_secs,
            capabilities: ProviderCapabilities {
                chat: self.can_chat,
                translate: self.can_translate,
                vision: self.can_vision,
            },
            disable_thinking: self.disable_thinking,
            api_key: None,
        }
    }
}

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<ProviderRecord> {
    let kind: String = row.get("kind")?;
    Ok(ProviderRecord {
        id: row.get("id")?,
        name: row.get("name")?,
        kind: match kind.as_str() {
            "openai" => ProviderKind::OpenAi,
            _ => ProviderKind::OpenAiCompatible,
        },
        base_url: row.get("base_url")?,
        model: row.get("model")?,
        organization: row.get("organization")?,
        timeout_secs: row.get::<_, i64>("timeout_secs")? as u64,
        can_chat: row.get::<_, i64>("can_chat")? != 0,
        can_translate: row.get::<_, i64>("can_translate")? != 0,
        can_vision: row.get::<_, i64>("can_vision")? != 0,
        disable_thinking: row.get::<_, i64>("disable_thinking")? != 0,
        api_key_ref: row.get("api_key_ref")?,
    })
}

impl Database {
    /// 插入或覆盖一个 Provider 配置。
    pub fn upsert_provider(&self, record: &ProviderRecord) -> AppResult<()> {
        if record.id.trim().is_empty() || record.base_url.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "provider id and base_url are required".to_string(),
            ));
        }
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO providers (
                id, name, kind, base_url, model, organization, timeout_secs,
                can_chat, can_translate, can_vision, disable_thinking, api_key_ref, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            )
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                kind = excluded.kind,
                base_url = excluded.base_url,
                model = excluded.model,
                organization = excluded.organization,
                timeout_secs = excluded.timeout_secs,
                can_chat = excluded.can_chat,
                can_translate = excluded.can_translate,
                can_vision = excluded.can_vision,
                disable_thinking = excluded.disable_thinking,
                api_key_ref = excluded.api_key_ref,
                updated_at = excluded.updated_at",
            params![
                record.id,
                record.name,
                match record.kind {
                    ProviderKind::OpenAiCompatible => "openai_compatible",
                    ProviderKind::OpenAi => "openai",
                },
                record.base_url,
                record.model,
                record.organization,
                record.timeout_secs as i64,
                record.can_chat,
                record.can_translate,
                record.can_vision,
                record.disable_thinking,
                record.api_key_ref,
            ],
        )
        .map_err(db_err)?;
        Ok(())
    }

    pub fn get_provider(&self, id: &str) -> AppResult<Option<ProviderRecord>> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT * FROM providers WHERE id = ?1",
            params![id],
            row_to_record,
        )
        .optional()
        .map_err(db_err)
    }

    /// 按 created_at 顺序列出全部 Provider。
    pub fn list_providers(&self) -> AppResult<Vec<ProviderRecord>> {
        let conn = self.lock_conn()?;
        let mut statement = conn
            .prepare("SELECT * FROM providers ORDER BY created_at, id")
            .map_err(db_err)?;
        let rows = statement
            .query_map([], row_to_record)
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    pub fn delete_provider(&self, id: &str) -> AppResult<bool> {
        let conn = self.lock_conn()?;
        let changed = conn
            .execute("DELETE FROM providers WHERE id = ?1", params![id])
            .map_err(db_err)?;
        Ok(changed > 0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn record(id: &str) -> ProviderRecord {
        ProviderRecord {
            id: id.to_string(),
            name: "DeepSeek".to_string(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: Some("deepseek-chat".to_string()),
            organization: None,
            timeout_secs: 30,
            can_chat: true,
            can_translate: true,
            can_vision: false,
            disable_thinking: false,
            api_key_ref: Some(format!("provider/{id}")),
        }
    }

    #[test]
    fn upsert_get_and_update() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();

        assert_eq!(db.get_provider("deepseek").unwrap(), None);

        db.upsert_provider(&record("deepseek")).unwrap();
        let fetched = db.get_provider("deepseek").unwrap().unwrap();
        assert_eq!(fetched.base_url, "https://api.deepseek.com/v1");
        assert_eq!(fetched.api_key_ref.as_deref(), Some("provider/deepseek"));

        let mut updated = record("deepseek");
        updated.timeout_secs = 60;
        db.upsert_provider(&updated).unwrap();
        assert_eq!(
            db.get_provider("deepseek").unwrap().unwrap().timeout_secs,
            60
        );
        assert_eq!(db.list_providers().unwrap().len(), 1);
    }

    #[test]
    fn to_config_has_no_api_key() {
        let config = record("deepseek").to_config();
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://api.deepseek.com/v1")
        );
        assert_eq!(config.api_key, None, "明文密钥不允许从 DAO 带出");
        assert!(config.capabilities.translate);
    }

    #[test]
    fn rejects_blank_id_and_base_url() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let mut invalid = record("x");
        invalid.base_url = " ".to_string();
        assert_eq!(
            db.upsert_provider(&invalid).unwrap_err().kind(),
            "invalid_input"
        );
    }

    #[test]
    fn delete_provider() {
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        db.upsert_provider(&record("a")).unwrap();
        assert!(db.delete_provider("a").unwrap());
        assert!(!db.delete_provider("a").unwrap());
    }
}
