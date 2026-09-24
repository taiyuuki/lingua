//! 划词翻译 pipeline：
//!
//! 选中文本 → AI Router（按任务选 Provider）→ Translation Engine → Popup。
//!
//! 顺序约束：**必须先抓选区再弹 popup**——popup 一旦聚焦，
//! 「当前聚焦元素」就变成我们自己，AX / Cmd+C 都会失效。

use crate::popup;
use crate::state::{AppState, PopupState};
use lingua_ai::{AIRouter, AiTask, OpenAiCompatibleProvider};
use lingua_core::translation::{TextKind, TranslationRequest, classify_text};
use lingua_core::{AppResult, Language};
use lingua_database::Database;
use lingua_database::VocabularyUpsert;
use lingua_dictionary::SqliteDictionaryProvider;
use lingua_secrets::{KeyringSecretStore, SecretRef, SecretStore};
use lingua_translation::TranslationEngine;
use std::collections::HashSet;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// settings 表中的键名（集中定义，避免散落字符串）。
pub const SETTINGS_DEFAULT_PROVIDER: &str = "ai.default_provider_id";
pub const SETTINGS_ROUTE_TRANSLATE: &str = "router.task.translate";
pub const SETTINGS_TARGET_LANGUAGE: &str = "translation.target_language";
pub const SETTINGS_SHORTCUT: &str = "shortcut.translate";
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

    // 3. 经 Router 选出 Translate 任务的 Provider，组装引擎并翻译。
    //    引擎内部先查词典（单词 + 命中时直接返回），否则走 AI。
    let router = build_router(&state.database)?;
    let provider = router.provider_for(AiTask::Translate)?;
    let dictionary: Arc<dyn lingua_dictionary::DictionaryProvider> =
        Arc::new(SqliteDictionaryProvider::new(state.database.clone()));
    let engine = TranslationEngine::new(provider).with_dictionary(Some(dictionary));
    let request = TranslationRequest::new(selected.text, target_language(&state.database)?);
    let result = engine.translate(&request).await?;

    // 4. 单词查询自动记入生词本（短语/句子不记）；
    //    在 result 移入 popup 前先取出需要的数据，写库放在 popup 展示之后。
    let vocabulary_record = if classify_text(&request.text) == TextKind::Word {
        Some(VocabularyUpsert {
            word: request.text.clone(),
            language: result
                .detected_language
                .clone()
                .or_else(|| request.source_language.clone())
                .unwrap_or_else(|| Language::parse("und")),
            translation: Some(result.translated_text.clone()),
            source: result
                .dictionary
                .as_ref()
                .map_or_else(|| "ai".to_string(), |entry| entry.source.clone()),
            examples: result
                .dictionary
                .as_ref()
                .map(|entry| {
                    entry
                        .definitions
                        .iter()
                        .flat_map(|definition| definition.examples.iter())
                        .take(4)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
            bump_lookup: true,
        })
    } else {
        None
    };

    // 5. 切到结果态。
    popup::show(app, PopupState::Result(Box::new(result))).await?;

    if let Some(record) = vocabulary_record {
        let database = state.database.clone();
        let recorded =
            tauri::async_runtime::spawn_blocking(move || database.upsert_vocabulary(&record)).await;
        match recorded {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => tracing::warn!(error = %error, "failed to record vocabulary lookup"),
            Err(error) => tracing::warn!(error = %error, "vocabulary record task panicked"),
        }
    }
    Ok(())
}

/// 从数据库 + OS 密钥库组装 AI Router：注册全部 Provider，
/// 应用默认 Provider 与任务路由（指向已删除 Provider 的路由被忽略）。
pub fn build_router(database: &Database) -> AppResult<AIRouter> {
    let secrets = KeyringSecretStore::default();
    let configured_default = read_default_provider_id(database);
    let mut registered: HashSet<String> = HashSet::new();

    let mut builder = AIRouter::builder();
    for record in database.list_providers()? {
        let mut config = record.to_config();
        config.api_key = secrets.get_secret(&SecretRef::provider(&record.id))?;
        if configured_default.as_deref() == Some(record.id.as_str()) {
            builder = builder.with_default(&record.id);
        }
        registered.insert(record.id.clone());
        builder = builder.register(OpenAiCompatibleProvider::new(config)?);
    }

    if let Some(translate_id) = read_route_provider(database, SETTINGS_ROUTE_TRANSLATE) {
        if registered.contains(&translate_id) {
            builder = builder.route(AiTask::Translate, translate_id);
        } else {
            tracing::warn!(provider = %translate_id, "translate route points to a missing provider; ignoring");
        }
    }

    Ok(builder.build())
}

/// 默认 Provider id；仅当设置值存在时返回（不存在由 build_router 回退）。
pub fn read_default_provider_id(database: &Database) -> Option<String> {
    database
        .get_setting(SETTINGS_DEFAULT_PROVIDER)
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_string))
}

pub fn read_route_provider(database: &Database, key: &str) -> Option<String> {
    database
        .get_setting(key)
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_string))
}

/// 通用 settings 读取（字符串值；不存在返回 None）。
pub fn read_setting(database: &Database, key: &str) -> Option<String> {
    database
        .get_setting(key)
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_string))
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

pub fn target_language(database: &Database) -> AppResult<Language> {
    Ok(Language::parse(&read_target_language_code(database)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use lingua_ai::ProviderKind;
    use lingua_database::providers::ProviderRecord;

    fn empty_database() -> Database {
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        database
    }

    fn record(id: &str) -> ProviderRecord {
        ProviderRecord {
            id: id.to_string(),
            name: id.to_string(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: format!("https://example.com/{id}/v1"),
            model: None,
            organization: None,
            timeout_secs: 30,
            can_chat: true,
            can_translate: true,
            can_vision: false,
            extra_body: None,
            api_key_ref: Some(format!("provider/{id}")),
        }
    }

    #[test]
    fn empty_database_yields_unconfigured_router() {
        let database = empty_database();
        assert_eq!(read_default_provider_id(&database), None);
        assert_eq!(read_shortcut(&database), "alt+t");
        assert_eq!(target_language(&database).unwrap(), Language::Zh);

        let router = build_router(&database).unwrap();
        let Err(error) = router.provider_for(AiTask::Translate) else {
            unreachable!("expected unconfigured error");
        };
        assert_eq!(error.kind(), "invalid_input");
    }

    #[test]
    fn router_uses_default_and_ignores_dangling_routes() {
        let database = empty_database();
        database.upsert_provider(&record("local")).unwrap();
        database.upsert_provider(&record("cloud")).unwrap();
        database
            .set_setting(SETTINGS_DEFAULT_PROVIDER, &serde_json::json!("cloud"))
            .unwrap();
        // 指向已删除 Provider 的路由应被忽略。
        database
            .set_setting(SETTINGS_ROUTE_TRANSLATE, &serde_json::json!("ghost"))
            .unwrap();

        let router = build_router(&database).unwrap();
        assert_eq!(
            router.provider_for(AiTask::Translate).unwrap().id(),
            "cloud"
        );
        assert_eq!(router.provider_ids(), vec!["cloud", "local"]);
    }

    #[test]
    fn translate_route_overrides_default() {
        let database = empty_database();
        database.upsert_provider(&record("local")).unwrap();
        database.upsert_provider(&record("cloud")).unwrap();
        database
            .set_setting(SETTINGS_DEFAULT_PROVIDER, &serde_json::json!("cloud"))
            .unwrap();
        database
            .set_setting(SETTINGS_ROUTE_TRANSLATE, &serde_json::json!("local"))
            .unwrap();

        let router = build_router(&database).unwrap();
        assert_eq!(
            router.provider_for(AiTask::Translate).unwrap().id(),
            "local"
        );
    }

    #[test]
    fn target_language_reads_overrides() {
        let database = empty_database();
        database
            .set_setting(SETTINGS_TARGET_LANGUAGE, &serde_json::json!("ja"))
            .unwrap();
        assert_eq!(target_language(&database).unwrap(), Language::Ja);
    }
}
