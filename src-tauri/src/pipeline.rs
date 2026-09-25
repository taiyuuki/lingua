//! 划词翻译 pipeline：
//!
//! 选中文本 → AI Router（按任务选 Provider）→ Translation Engine → Popup。
//!
//! 顺序约束：**必须先抓选区再弹 popup**——popup 一旦聚焦，
//! 「当前聚焦元素」就变成我们自己，AX / Cmd+C 都会失效。

use crate::popup;
use crate::state::{AppState, PopupState};
use lingua_ai::{AIRouter, AiTask, OpenAiCompatibleProvider};
use lingua_core::translation::{TextKind, TranslationRequest, TranslationResult, classify_text};
use lingua_core::{AppResult, HistoryKind, HistoryRecord, Language};
use lingua_database::Database;
use lingua_database::VocabularyUpsert;
use lingua_dictionary::SqliteDictionaryProvider;
use lingua_secrets::{KeyringSecretStore, SecretRef, SecretStore};
use lingua_translation::TranslationEngine;
use std::collections::HashSet;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// settings 表中的键名（集中定义，避免散落字符串）。
pub const SETTINGS_DEFAULT_PROVIDER: &str = "ai.default_provider_id";
pub const SETTINGS_ROUTE_TRANSLATE: &str = "router.task.translate";
pub const SETTINGS_ROUTE_VISION: &str = "router.task.vision";
/// 学习分析任务路由（Phase 9；未设置 = 跟随默认 Provider）。
pub const SETTINGS_ROUTE_STUDY: &str = "router.task.study";
pub const SETTINGS_TARGET_LANGUAGE: &str = "translation.target_language";
/// 同语言换向目标（划到的文本已是目标语言时改译成该语言；
/// 空串 = 关闭）。默认 en：目标 zh 的用户划到中文文本改译英文。
pub const SETTINGS_SWAP_LANGUAGE: &str = "translation.swap_language";
/// 自定义翻译 system prompt（JSON 路径 / 流式路径）；空串 = 用内置。
pub const SETTINGS_PROMPT_TRANSLATE_SYSTEM: &str = "prompt.translate_system";
pub const SETTINGS_PROMPT_TRANSLATE_STREAM: &str = "prompt.translate_stream";
pub const SETTINGS_SHORTCUT: &str = "shortcut.translate";
pub const SETTINGS_SHORTCUT_OCR: &str = "shortcut.ocr";
/// 查词自动记入生词本（默认关；Popup 的 ★ 手动收藏不受影响）。
pub const SETTINGS_AUTO_VOCABULARY: &str = "vocabulary.auto_record";
pub const SETTINGS_UI_THEME: &str = "ui.theme";
pub const DEFAULT_UI_THEME: &str = "dark";
pub const DEFAULT_SHORTCUT: &str = "alt+t";
pub const DEFAULT_OCR_SHORTCUT: &str = "alt+s";
pub const DEFAULT_TARGET_LANGUAGE: &str = "zh";
pub const DEFAULT_SWAP_LANGUAGE: &str = "en";

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

    // 划词翻译不是 OCR 来源：清掉「用 Vision 重新识别」的保留截图
    // 与识别来源标签。
    if let Ok(mut retry) = state.ocr_retry.lock() {
        *retry = None;
    }
    if let Ok(mut source) = state.ocr_source.lock() {
        *source = None;
    }

    // 1. 抓选区（此时 popup 还没弹，聚焦元素仍是目标应用）。
    let selected = state.selection.capture_first().await?;
    tracing::debug!(
        chars = selected.text.chars().count(),
        source = ?selected.source,
        "selection captured"
    );

    // 2. 立即弹 popup 展示原文（流式：译文随后增量到达）。
    popup::show(
        app,
        PopupState::Loading {
            source_text: Some(selected.text.clone()),
        },
    )
    .await?;

    // 3. 经 Router 选出 Translate 任务的 Provider，组装引擎并翻译。
    //    引擎内部先查词典（单词 + 命中时直接返回），否则走 AI；
    //    同语言换向由 Prompt 指令层处理（设置里可关）。
    let router = build_router(&state.database)?;
    let provider = router.provider_for(AiTask::Translate)?;
    let dictionary: Arc<dyn lingua_dictionary::DictionaryProvider> =
        Arc::new(SqliteDictionaryProvider::new(state.database.clone()));
    let engine = TranslationEngine::new(provider)
        .with_dictionary(Some(dictionary))
        .with_swap_language(swap_language(&state.database))
        .with_prompt_override(prompt_override(&state.database));
    let request = TranslationRequest::new(selected.text, target_language(&state.database)?);
    let app_for_delta = app.clone();
    let result = engine
        .translate_stream(&request, &mut |text: String| {
            crate::popup::set_streamed(&text);
            if let Err(error) = app_for_delta.emit("popup://delta", text) {
                tracing::warn!(error = %error, "failed to emit popup delta");
            }
        })
        .await?;

    // 4. 单词查询自动记入生词本（短语/句子不记；可在设置里关，默认关）；
    //    在 result 移入 popup 前先取出需要的数据，写库放在 popup 展示之后。
    //    来源/例句取首选词典（dictionaries[0]）。
    let auto_vocabulary =
        read_setting(&state.database, SETTINGS_AUTO_VOCABULARY).as_deref() == Some("true");
    let vocabulary_record = if auto_vocabulary && classify_text(&request.text) == TextKind::Word {
        Some(VocabularyUpsert {
            word: request.text.clone(),
            language: result
                .detected_language
                .clone()
                .or_else(|| request.source_language.clone())
                .unwrap_or_else(|| Language::parse("und")),
            translation: Some(result.translated_text.clone()),
            source: result
                .dictionaries
                .first()
                .map_or_else(|| "ai".to_string(), |entry| entry.source.clone()),
            examples: result
                .dictionaries
                .first()
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

    // 5. 切到结果态，随后旁路写历史与生词记录（都不阻断主流程）。
    let history_record = history_record_from_result(
        &result,
        request.target_language.clone(),
        HistoryKind::Selection,
    );
    popup::show(app, PopupState::Result(Box::new(result))).await?;

    record_history_task(state.database.clone(), history_record).await;

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

/// 从翻译结果组装历史写入载荷（译入语由调用方补充——结果里没有）。
pub fn history_record_from_result(
    result: &TranslationResult,
    target_language: Language,
    kind: HistoryKind,
) -> HistoryRecord {
    HistoryRecord {
        source_text: result.source_text.clone(),
        translated_text: result.translated_text.clone(),
        detected_language: result.detected_language.clone(),
        target_language,
        model: result.model.clone(),
        kind,
        dictionary_hit: !result.dictionaries.is_empty(),
    }
}

/// 旁路写历史：失败只告警，绝不影响翻译主流程。
pub async fn record_history_task(database: Database, record: HistoryRecord) {
    let recorded =
        tauri::async_runtime::spawn_blocking(move || database.record_history(&record)).await;
    match recorded {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::warn!(error = %error, "failed to record history"),
        Err(error) => tracing::warn!(error = %error, "history record task panicked"),
    }
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

    // Vision 路由同样只在 Provider 存在时生效；能力校验由 Router 分派时做。
    if let Some(vision_id) = read_route_provider(database, SETTINGS_ROUTE_VISION) {
        if registered.contains(&vision_id) {
            builder = builder.route(AiTask::Vision, vision_id);
        } else {
            tracing::warn!(provider = %vision_id, "vision route points to a missing provider; ignoring");
        }
    }

    // 学习分析路由：普通文本任务，跟随默认 Provider 即可工作。
    if let Some(study_id) = read_route_provider(database, SETTINGS_ROUTE_STUDY) {
        if registered.contains(&study_id) {
            builder = builder.route(AiTask::Study, study_id);
        } else {
            tracing::warn!(provider = %study_id, "study route points to a missing provider; ignoring");
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

/// 界面主题（light | dark）；非法值回退 dark。
pub fn read_ui_theme(database: &Database) -> String {
    match read_setting(database, SETTINGS_UI_THEME).as_deref() {
        Some("light") => "light".to_string(),
        _ => DEFAULT_UI_THEME.to_string(),
    }
}

pub fn read_shortcut(database: &Database) -> String {
    database
        .get_setting(SETTINGS_SHORTCUT)
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_SHORTCUT.to_string())
}

/// 截图翻译快捷键（默认 alt+s）；按钮入口只能截主窗口所在屏，
/// 快捷键入口跟随光标所在显示器。
pub fn read_ocr_shortcut(database: &Database) -> String {
    database
        .get_setting(SETTINGS_SHORTCUT_OCR)
        .ok()
        .flatten()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_OCR_SHORTCUT.to_string())
}

pub fn target_language(database: &Database) -> AppResult<Language> {
    Ok(Language::parse(&read_target_language_code(database)))
}

/// 同语言换向目标（`Some` = 启用；空串关闭 / 与目标相同由引擎视为无效）。
pub fn swap_language(database: &Database) -> Option<Language> {
    let code = read_setting(database, SETTINGS_SWAP_LANGUAGE)
        .unwrap_or_else(|| DEFAULT_SWAP_LANGUAGE.to_string());
    let code = code.trim().to_string();
    if code.is_empty() {
        None
    } else {
        Some(Language::parse(&code))
    }
}

/// 自定义翻译 system prompt（空白归一为 None = 用内置）。
pub fn prompt_override(database: &Database) -> lingua_translation::PromptOverride {
    lingua_translation::PromptOverride::normalized(
        Some(&read_setting(database, SETTINGS_PROMPT_TRANSLATE_SYSTEM).unwrap_or_default()),
        Some(&read_setting(database, SETTINGS_PROMPT_TRANSLATE_STREAM).unwrap_or_default()),
    )
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
        assert_eq!(read_ocr_shortcut(&database), "alt+s");
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
