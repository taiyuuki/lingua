//! 划词翻译 pipeline：
//!
//! 选中文本 → Translation Engine（AI Provider）→ Popup 展示。
//!
//! 顺序约束：**必须先抓选区再弹 popup**——popup 一旦聚焦，
//! 「当前聚焦元素」就变成我们自己，AX / Cmd+C 都会失效。

use crate::popup;
use crate::state::{AppState, PopupState};
use lingua_ai::OpenAiCompatibleProvider;
use lingua_core::translation::TranslationRequest;
use lingua_core::{AppError, AppResult, Language};
use lingua_database::Database;
use lingua_secrets::{KeyringSecretStore, SecretRef, SecretStore};
use lingua_translation::TranslationEngine;
use tauri::{AppHandle, Manager};

/// settings 表中的键名（集中定义，避免散落字符串）。
pub const SETTINGS_DEFAULT_PROVIDER: &str = "ai.default_provider_id";
pub const SETTINGS_TARGET_LANGUAGE: &str = "translation.target_language";
pub const SETTINGS_SHORTCUT: &str = "shortcut.translate";
pub const DEFAULT_PROVIDER_ID: &str = "default";
pub const DEFAULT_SHORTCUT: &str = "alt+t";
pub const DEFAULT_TARGET_LANGUAGE: &str = "zh";

/// 完整流程；任何失败都以 popup 错误态呈现（用户可感知）。
pub async fn run_translation_pipeline(app: AppHandle) {
    if let Err(error) = run(&app).await {
        tracing::warn!(error = %error, "translation pipeline failed");
        let popup_state = PopupState::Error {
            kind: error.kind().to_string(),
            message: error.to_string(),
        };
        if let Err(show_error) = popup::show(&app, popup_state).await {
            tracing::error!(error = %show_error, "failed to show error popup");
        }
    }
}

async fn run(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();

    // 1. 抓选区（此时 popup 还没弹，聚焦元素仍是目标应用）。
    let selected = state.selection.capture_first().await?;
    tracing::debug!(
        chars = selected.text.chars().count(),
        source = ?selected.source,
        "selection captured"
    );

    // 2. 立即弹 popup 展示加载态（用户有反馈，翻译可能耗时数秒）。
    popup::show(app, PopupState::Loading).await?;

    // 3. 组装引擎并翻译。
    let engine = build_engine(&state.database)?;
    let request = TranslationRequest::new(selected.text, target_language(&state.database)?);
    let result = engine.translate(&request).await?;

    // 4. 切到结果态。
    popup::show(app, PopupState::Result(result)).await?;
    Ok(())
}

/// 从数据库 + OS 密钥库组装默认 Translation Engine。
///
/// Phase 2 会把这一步换成 AI Router：按任务在多个 Provider 间挑选。
pub fn build_engine(database: &Database) -> AppResult<TranslationEngine> {
    let provider_id = read_default_provider_id(database);
    let record = database.get_provider(&provider_id)?.ok_or_else(|| {
        AppError::InvalidInput(format!(
            "AI provider `{provider_id}` is not configured; open Settings to set base_url / api key"
        ))
    })?;

    let mut config = record.to_config();
    let secrets = KeyringSecretStore::default();
    config.api_key = secrets.get_secret(&SecretRef::provider(&provider_id))?;

    let provider = OpenAiCompatibleProvider::new(config)?;
    Ok(TranslationEngine::new(Box::new(provider)))
}

pub fn read_default_provider_id(database: &Database) -> String {
    database
        .get_setting(SETTINGS_DEFAULT_PROVIDER)
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_PROVIDER_ID.to_string())
}

pub fn read_target_language_code(database: &Database) -> String {
    database
        .get_setting(SETTINGS_TARGET_LANGUAGE)
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_TARGET_LANGUAGE.to_string())
}

pub fn read_shortcut(database: &Database) -> String {
    database
        .get_setting(SETTINGS_SHORTCUT)
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_SHORTCUT.to_string())
}

fn target_language(database: &Database) -> AppResult<Language> {
    Ok(Language::parse(&read_target_language_code(database)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn settings_defaults_are_sane() {
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        assert_eq!(read_default_provider_id(&database), "default");
        assert_eq!(read_shortcut(&database), "alt+t");
        assert_eq!(target_language(&database).unwrap(), Language::Zh);
        let Err(error) = build_engine(&database) else {
            unreachable!("expected missing-provider error");
        };
        assert_eq!(error.kind(), "invalid_input");
    }

    #[test]
    fn target_language_reads_overrides() {
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        database
            .set_setting(SETTINGS_TARGET_LANGUAGE, &serde_json::json!("ja"))
            .unwrap();
        assert_eq!(target_language(&database).unwrap(), Language::Ja);
    }
}
