//! Tauri commands —— 前端唯一的后端入口。
//!
//! 约定：
//!
//! * 命令返回值序列化为 camelCase JSON；
//! * 可能失败的命令返回 `Result<T, AppError>`，前端拿到 `{kind, message}`；
//! * 涉及数据库 / 密钥库的命令用 `spawn_blocking` 包裹，避免阻塞主线程。

use crate::{
    POPUP_LABEL, pipeline, platform_error, popup,
    state::{AppState, PopupState},
};
use lingua_ai::{AIProvider, ProviderKind};
use lingua_core::{AppError, AppResult, Language, Mastery, TextKind, classify_text};
use lingua_database::providers::ProviderRecord;
use lingua_database::{DictionaryInfo, DictionaryMeta, VocabularyFilter, VocabularyUpsert};
use lingua_dictionary::parse_tsv;
use lingua_secrets::{KeyringSecretStore, SecretRef, SecretStore};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

/// 应用基本信息（Phase 0 用于验证 IPC 链路）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

/// 数据库状态（Phase 0 用于验证「打开 → migration」链路）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStatus {
    pub schema_version: u32,
    pub migrations_total: usize,
}

#[tauri::command]
pub async fn database_status(
    state: tauri::State<'_, AppState>,
) -> Result<DatabaseStatus, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 顺带确保 schema 已是最新（正常流程 setup 阶段已完成，这里幂等）。
        let report = database.migrate()?;
        Ok(DatabaseStatus {
            schema_version: report.current_version,
            migrations_total: lingua_database::MIGRATIONS.len(),
        })
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

// ---------- Provider 管理（Phase 2） ----------

/// Provider 配置 DTO（前端表单 ↔ 后端）。
///
/// 出参时 `apiKey` 恒为 `None`、`hasApiKey` 指示密钥库中是否有密钥；
/// 入参时 `apiKey` 按 [`save_provider`] 注释中的语义解释。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDto {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: Option<String>,
    pub organization: Option<String>,
    pub timeout_secs: u64,
    /// 能力声明（对话 / 翻译 / 视觉）；Router 的 Vision 任务只派给
    /// `can_vision == true` 的 Provider。
    #[serde(default = "default_true")]
    pub can_chat: bool,
    #[serde(default = "default_true")]
    pub can_translate: bool,
    #[serde(default)]
    pub can_vision: bool,
    /// 厂商扩展参数（JSON 对象文本）；保存时校验必须是合法 JSON 对象。
    #[serde(default)]
    pub extra_body: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub has_api_key: bool,
}

/// bool 字段的 serde 缺省：保持旧客户端不传能力字段时的历史行为
/// （chat / translate 默认开启）。
fn default_true() -> bool {
    true
}

impl ProviderDto {
    fn from_record(record: ProviderRecord, has_api_key: bool) -> Self {
        Self {
            id: record.id,
            name: record.name,
            kind: record.kind,
            base_url: record.base_url,
            model: record.model,
            organization: record.organization,
            timeout_secs: record.timeout_secs,
            can_chat: record.can_chat,
            can_translate: record.can_translate,
            can_vision: record.can_vision,
            extra_body: record.extra_body,
            api_key: None,
            has_api_key,
        }
    }

    fn to_record(&self) -> AppResult<ProviderRecord> {
        Ok(ProviderRecord {
            id: self.id.trim().to_string(),
            name: self.name.trim().to_string(),
            kind: self.kind,
            base_url: self.base_url.trim().to_string(),
            model: self
                .model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            organization: self.organization.clone(),
            timeout_secs: self.timeout_secs,
            can_chat: self.can_chat,
            can_translate: self.can_translate,
            can_vision: self.can_vision,
            extra_body: normalized_extra_body(self.extra_body.as_deref())?,
            api_key_ref: Some(SecretRef::provider(self.id.trim()).account),
        })
    }
}

/// 列出全部 Provider（含每个的密钥存在状态），按创建顺序。
/// 校验并归一化 extra_body：空 → None；必须是合法 JSON 对象，
/// 存储为紧凑形式（非法输入返回 InvalidInput，不让坏数据进库）。
fn normalized_extra_body(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(text) = raw.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_str(text).map_err(|error| {
        AppError::InvalidInput(format!("extra_body is not valid JSON: {error}"))
    })?;
    if !value.is_object() {
        return Err(AppError::InvalidInput(
            "extra_body must be a JSON object".to_string(),
        ));
    }
    Ok(Some(value.to_string()))
}

#[tauri::command]
pub async fn list_providers(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProviderDto>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let secrets = KeyringSecretStore::default();
        database
            .list_providers()?
            .into_iter()
            .map(|record| {
                let has_api_key = secrets
                    .get_secret(&SecretRef::provider(&record.id))?
                    .is_some();
                Ok(ProviderDto::from_record(record, has_api_key))
            })
            .collect()
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 插入或更新一个 Provider。
///
/// `api_key` 字段语义：`None` = 不修改；`Some("")` = 删除；非空字符串 = 覆盖。
/// 保存后若当前没有默认 Provider，则把该 Provider 设为默认。
#[tauri::command]
pub async fn save_provider(
    state: tauri::State<'_, AppState>,
    provider: ProviderDto,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let record = provider.to_record()?;
        if record.id.is_empty() || record.base_url.is_empty() {
            return Err(AppError::InvalidInput(
                "provider id and base_url are required".to_string(),
            ));
        }
        database.upsert_provider(&record)?;

        let no_usable_default = match pipeline::read_default_provider_id(&database) {
            Some(id) => database.get_provider(&id)?.is_none(),
            None => true,
        };
        if no_usable_default {
            database.set_setting(
                pipeline::SETTINGS_DEFAULT_PROVIDER,
                &serde_json::json!(record.id),
            )?;
        }

        let reference = SecretRef::provider(&record.id);
        let secrets = KeyringSecretStore::default();
        match provider.api_key.as_deref() {
            None => {}
            Some("") => {
                secrets.delete_secret(&reference)?;
            }
            Some(key) => {
                secrets.set_secret(&reference, key)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 删除 Provider（连同其密钥），并清理指向它的默认 / 路由设置。
#[tauri::command]
pub async fn delete_provider(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if database.delete_provider(&id)? {
            KeyringSecretStore::default().delete_secret(&SecretRef::provider(&id))?;
        }
        if pipeline::read_default_provider_id(&database).as_deref() == Some(id.as_str()) {
            database.remove_setting(pipeline::SETTINGS_DEFAULT_PROVIDER)?;
        }
        if pipeline::read_route_provider(&database, pipeline::SETTINGS_ROUTE_TRANSLATE).as_deref()
            == Some(id.as_str())
        {
            database.remove_setting(pipeline::SETTINGS_ROUTE_TRANSLATE)?;
        }
        if pipeline::read_route_provider(&database, pipeline::SETTINGS_ROUTE_VISION).as_deref()
            == Some(id.as_str())
        {
            database.remove_setting(pipeline::SETTINGS_ROUTE_VISION)?;
        }
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 连接测试结果。测试失败不算命令失败，结构化返回给前端展示。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestProviderResult {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub reply: Option<String>,
    pub error: Option<String>,
}

/// 连接测试：向该 Provider 发一个极小的对话请求。
#[tauri::command]
pub async fn test_provider(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<TestProviderResult, AppError> {
    let database = state.database.clone();
    // 阻塞部分（DB + 钥匙串）先在阻塞线程里完成。
    let provider = tauri::async_runtime::spawn_blocking(move || -> AppResult<_> {
        let record = database
            .get_provider(&id)?
            .ok_or_else(|| AppError::InvalidInput(format!("provider `{id}` does not exist")))?;
        let mut config = record.to_config();
        config.api_key = KeyringSecretStore::default().get_secret(&SecretRef::provider(&id))?;
        lingua_ai::OpenAiCompatibleProvider::new(config)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))??;

    let request = lingua_ai::ChatRequest {
        messages: vec![
            lingua_ai::ChatMessage::system("You are a connection health check."),
            lingua_ai::ChatMessage::user("Reply with the single word: pong"),
        ],
        model: None,
        temperature: Some(0.0),
        max_tokens: Some(16),
    };

    let start = std::time::Instant::now();
    match provider.chat(&request).await {
        Ok(response) => Ok(TestProviderResult {
            ok: true,
            latency_ms: Some(start.elapsed().as_millis() as u64),
            reply: Some(response.content.chars().take(60).collect()),
            error: None,
        }),
        Err(error) => Ok(TestProviderResult {
            ok: false,
            latency_ms: Some(start.elapsed().as_millis() as u64),
            reply: None,
            error: Some(error.to_string()),
        }),
    }
}

// ---------- 应用设置（路由 + 快捷键） ----------

/// 应用设置：AI Router 路由 + 全局快捷键。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsDto {
    /// 默认 Provider（`None` = 未设置，路由未命中时报「未配置」）。
    pub default_provider_id: Option<String>,
    /// 翻译任务路由（`None` = 跟随默认 Provider）。
    pub translate_provider_id: Option<String>,
    /// Vision 任务路由（`None` = 自动挑选任一 vision 能力 Provider）。
    pub vision_provider_id: Option<String>,
    /// 划词翻译的目标语言（BCP-47 基础码）。
    pub target_language: Language,
    pub shortcut: String,
    /// 截图翻译快捷键（默认 alt+s；多显示器下跟随光标所在屏）。
    pub ocr_shortcut: String,
    /// 界面主题：light | dark。
    pub ui_theme: String,
    /// OCR 引擎：`tesseract` | `http`。
    pub ocr_provider: String,
    pub ocr_tesseract_path: String,
    pub ocr_http_url: String,
}

#[tauri::command]
pub async fn get_app_settings(
    state: tauri::State<'_, AppState>,
) -> Result<AppSettingsDto, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(AppSettingsDto {
            default_provider_id: pipeline::read_default_provider_id(&database),
            translate_provider_id: pipeline::read_route_provider(
                &database,
                pipeline::SETTINGS_ROUTE_TRANSLATE,
            ),
            vision_provider_id: pipeline::read_route_provider(
                &database,
                pipeline::SETTINGS_ROUTE_VISION,
            ),
            target_language: Language::parse(&pipeline::read_target_language_code(&database)),
            shortcut: pipeline::read_shortcut(&database),
            ocr_shortcut: pipeline::read_ocr_shortcut(&database),
            ui_theme: pipeline::read_ui_theme(&database),
            ocr_provider: pipeline::read_setting(&database, crate::ocr::SETTINGS_OCR_PROVIDER)
                .unwrap_or_else(|| crate::ocr::OCR_PROVIDER_TESSERACT.to_string()),
            ocr_tesseract_path: pipeline::read_setting(
                &database,
                crate::ocr::SETTINGS_OCR_TESSERACT_PATH,
            )
            .unwrap_or_else(|| crate::ocr::DEFAULT_OCR_TESSERACT_PATH.to_string()),
            ocr_http_url: pipeline::read_setting(&database, crate::ocr::SETTINGS_OCR_HTTP_URL)
                .unwrap_or_else(|| lingua_ocr::OCR_HTTP_DEFAULT_URL.to_string()),
        })
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 保存应用设置；快捷键变更会立即重新注册。
#[tauri::command]
pub async fn save_app_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    settings: AppSettingsDto,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        match settings.default_provider_id.as_deref() {
            None => {
                database.remove_setting(pipeline::SETTINGS_DEFAULT_PROVIDER)?;
            }
            Some(id) => {
                database.set_setting(pipeline::SETTINGS_DEFAULT_PROVIDER, &serde_json::json!(id))?
            }
        }
        match settings.translate_provider_id.as_deref() {
            None => {
                database.remove_setting(pipeline::SETTINGS_ROUTE_TRANSLATE)?;
            }
            Some(id) => {
                database.set_setting(pipeline::SETTINGS_ROUTE_TRANSLATE, &serde_json::json!(id))?
            }
        }
        match settings.vision_provider_id.as_deref() {
            None => {
                database.remove_setting(pipeline::SETTINGS_ROUTE_VISION)?;
            }
            Some(id) => {
                database.set_setting(pipeline::SETTINGS_ROUTE_VISION, &serde_json::json!(id))?
            }
        }
        database.set_setting(
            pipeline::SETTINGS_UI_THEME,
            &serde_json::json!(settings.ui_theme),
        )?;
        database.set_setting(
            pipeline::SETTINGS_TARGET_LANGUAGE,
            &serde_json::json!(settings.target_language.code()),
        )?;
        database.set_setting(
            crate::ocr::SETTINGS_OCR_PROVIDER,
            &serde_json::json!(settings.ocr_provider),
        )?;
        database.set_setting(
            crate::ocr::SETTINGS_OCR_TESSERACT_PATH,
            &serde_json::json!(settings.ocr_tesseract_path),
        )?;
        database.set_setting(
            crate::ocr::SETTINGS_OCR_HTTP_URL,
            &serde_json::json!(settings.ocr_http_url),
        )?;
        database.set_setting(
            pipeline::SETTINGS_SHORTCUT,
            &serde_json::json!(settings.shortcut),
        )?;
        database.set_setting(
            pipeline::SETTINGS_SHORTCUT_OCR,
            &serde_json::json!(settings.ocr_shortcut),
        )?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))??;

    crate::register_shortcut_from_settings(&app, &state.database)?;
    Ok(())
}

// ---------- Popup / 权限 ----------

/// 手动打开（或聚焦已隐藏的）Popup 窗口。
/// 主要用于开发与设置页验证；正常入口是全局快捷键触发的 pipeline。
#[tauri::command]
pub async fn open_popup(app: tauri::AppHandle) -> Result<(), AppError> {
    let window = popup::ensure_window(&app)?;
    window.show().map_err(platform_error)?;
    window.set_focus().map_err(platform_error)?;
    Ok(())
}

/// 流式译文最新累积文本（弹窗挂载晚于流开始时的追平用）。
#[tauri::command]
pub async fn latest_streamed() -> Result<String, AppError> {
    Ok(crate::popup::streamed())
}

/// 切换 Popup 置顶（工具栏图钉），返回切换后的状态。
#[tauri::command]
pub async fn toggle_popup_pin(app: tauri::AppHandle) -> Result<bool, AppError> {
    let window = crate::popup::ensure_window(&app)?;
    let next = !crate::popup::popup_pinned();
    window.set_always_on_top(next).map_err(platform_error)?;
    crate::popup::set_popup_pinned(next);
    Ok(next)
}

/// 隐藏 Popup（前端按 Esc 时调用）。
#[tauri::command]
pub async fn hide_popup(app: tauri::AppHandle) -> Result<(), AppError> {
    if let Some(window) = app.get_webview_window(POPUP_LABEL) {
        window.hide().map_err(platform_error)?;
    }
    Ok(())
}

/// Popup 最新状态快照（窗口刚创建时补拉，避免错过事件）。
#[tauri::command]
pub async fn latest_popup_state(app: tauri::AppHandle) -> Result<Option<PopupState>, AppError> {
    let state = app.state::<AppState>();
    let guard = state
        .popup_state
        .lock()
        .map_err(|_| AppError::Internal("popup state lock poisoned".into()))?;
    Ok(guard.clone())
}

/// 划词权限状态（macOS：辅助功能授权）。
#[tauri::command]
pub async fn selection_permission_status() -> Result<bool, AppError> {
    Ok(lingua_platform::selection_permission_granted())
}

// ---------- 词典管理（Phase 3） ----------

#[tauri::command]
pub async fn list_dictionaries(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<DictionaryInfo>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.list_dictionaries())
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDictionaryRequest {
    pub id: String,
    pub name: String,
    pub source_language: Language,
    pub gloss_language: Language,
    /// 词典 TSV 文件的绝对路径（Phase 3 用路径输入，文件选择器后续接入）。
    pub path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDictionaryResult {
    pub imported_entries: usize,
    pub skipped_lines: usize,
}

/// 从 TSV 文件导入词典（格式见 `lingua_dictionary::tsv` 模块文档）。
#[tauri::command]
pub async fn import_dictionary_tsv(
    state: tauri::State<'_, AppState>,
    request: ImportDictionaryRequest,
) -> Result<ImportDictionaryResult, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let id = request.id.trim().to_string();
        if id.is_empty() {
            return Err(AppError::InvalidInput(
                "dictionary id is required".to_string(),
            ));
        }
        let text = std::fs::read_to_string(&request.path).map_err(|error| {
            AppError::InvalidInput(format!("cannot read `{}`: {error}", request.path))
        })?;
        let parsed = parse_tsv(
            &text,
            request.source_language.clone(),
            request.gloss_language.clone(),
            &id,
        );
        if parsed.entries.is_empty() {
            return Err(AppError::InvalidInput(format!(
                "no valid entries found in `{}`; check the TSV format",
                request.path
            )));
        }
        let meta = DictionaryMeta {
            id,
            name: request.name.trim().to_string(),
            source_language: request.source_language,
            gloss_language: request.gloss_language,
        };
        let imported_entries = database.import_dictionary(&meta, &parsed.entries)?;
        tracing::info!(
            dictionary = %meta.id,
            imported = imported_entries,
            skipped = parsed.skipped_lines,
            "dictionary imported"
        );
        Ok(ImportDictionaryResult {
            imported_entries,
            skipped_lines: parsed.skipped_lines,
        })
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// MDX 导入进度事件 payload（事件名 `dictionary://import-progress`）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryImportProgress {
    pub dictionary_id: String,
    /// `parsing` = 解析索引；`importing` = 批量写库。
    pub phase: &'static str,
    pub imported: usize,
    pub total: usize,
}

/// MDX 每批写库的词条数（单事务粒度，控制数据库互斥锁的持有时长）。
const MDX_IMPORT_BATCH: usize = 5000;

/// 从 MDX（MDict）文件导入词典。
///
/// 大词库（几十万词条）分批写库并通过 `dictionary://import-progress`
/// 事件上报进度；释义 HTML 转纯文本；注册码加密的词库返回明确错误。
#[tauri::command]
pub async fn import_dictionary_mdx(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    request: ImportDictionaryRequest,
) -> Result<ImportDictionaryResult, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let id = request.id.trim().to_string();
        if id.is_empty() {
            return Err(AppError::InvalidInput(
                "dictionary id is required".to_string(),
            ));
        }
        let emit_progress = |phase: &'static str, imported: usize, total: usize| {
            let _ = app.emit(
                "dictionary://import-progress",
                DictionaryImportProgress {
                    dictionary_id: id.clone(),
                    phase,
                    imported,
                    total,
                },
            );
        };

        emit_progress("parsing", 0, 0);
        let dict =
            lingua_dictionary::mdx::MdxFile::open(std::path::Path::new(&request.path))?;
        let total = dict.entry_count();
        if total == 0 {
            return Err(AppError::InvalidInput(format!(
                "no valid entries found in `{}`",
                request.path
            )));
        }
        let meta = DictionaryMeta {
            id: id.clone(),
            name: request.name.trim().to_string(),
            source_language: request.source_language.clone(),
            gloss_language: request.gloss_language.clone(),
        };
        let source_language = request.source_language;
        let gloss_language = request.gloss_language;

        let mut batch = Vec::with_capacity(MDX_IMPORT_BATCH);
        let mut imported = 0usize;
        let mut skipped = 0usize;
        for entry in dict.entries() {
            if entry.is_redirect() {
                skipped += 1;
                continue;
            }
            batch.push(entry.to_dictionary_entry(&source_language, &gloss_language, &id));
            if batch.len() >= MDX_IMPORT_BATCH {
                imported += database.import_dictionary(&meta, &batch)?;
                emit_progress("importing", imported, total);
                batch.clear();
            }
        }
        if !batch.is_empty() {
            imported += database.import_dictionary(&meta, &batch)?;
        }
        emit_progress("importing", imported, total);
        tracing::info!(
            dictionary = %meta.id,
            imported,
            skipped,
            total,
            "mdx dictionary imported"
        );
        Ok(ImportDictionaryResult {
            imported_entries: imported,
            skipped_lines: skipped,
        })
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

#[tauri::command]
pub async fn delete_dictionary(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        database.delete_dictionary(&id)?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

// ---------- 生词本（Phase 4） ----------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VocabularyFilterDto {
    pub search: Option<String>,
    pub tag: Option<String>,
    pub language: Option<Language>,
    pub mastery: Option<Mastery>,
    pub limit: u32,
}

#[tauri::command]
pub async fn list_vocabulary(
    state: tauri::State<'_, AppState>,
    filter: Option<VocabularyFilterDto>,
) -> Result<Vec<lingua_core::VocabularyItem>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dto = filter.unwrap_or_default();
        let filter = VocabularyFilter {
            search: dto.search,
            tag: dto.tag,
            language: dto.language,
            mastery: dto.mastery,
            limit: dto.limit,
        };
        database.list_vocabulary(&filter)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

#[tauri::command]
pub async fn update_vocabulary(
    state: tauri::State<'_, AppState>,
    id: i64,
    notes: String,
    tags: Vec<String>,
    mastery: Mastery,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        database.update_vocabulary(id, &notes, &tags, mastery)?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

#[tauri::command]
pub async fn delete_vocabulary(state: tauri::State<'_, AppState>, id: i64) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        database.delete_vocabulary(id)?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

#[tauri::command]
pub async fn vocabulary_tags(state: tauri::State<'_, AppState>) -> Result<Vec<String>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.vocabulary_tags())
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveVocabularyRequest {
    pub word: String,
    pub language: Option<Language>,
    pub translation: Option<String>,
    pub source: Option<String>,
    #[serde(default)]
    pub examples: Vec<String>,
}

/// 是否是可收藏的单词（整段/短语不支持收藏）。
#[tauri::command]
pub async fn is_word(text: String) -> Result<bool, AppError> {
    Ok(classify_text(&text) == TextKind::Word && !text.trim().is_empty())
}

/// 生词本中是否已有该词（星标初始状态）。
#[tauri::command]
pub async fn vocabulary_word_status(
    state: tauri::State<'_, AppState>,
    word: String,
    language: Option<Language>,
) -> Result<bool, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(database
            .find_vocabulary(&word, language.unwrap_or_else(|| Language::parse("und")))?
            .is_some())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 从生词本移除该词（弹窗取消收藏）。
#[tauri::command]
pub async fn remove_vocabulary_word(
    state: tauri::State<'_, AppState>,
    word: String,
    language: Option<Language>,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        database
            .delete_vocabulary_word(&word, language.unwrap_or_else(|| Language::parse("und")))?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// Popup 手动收藏（不累计 lookup_count）；仅单词可收藏。
#[tauri::command]
pub async fn save_to_vocabulary(
    state: tauri::State<'_, AppState>,
    request: SaveVocabularyRequest,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let word = request.word.trim().to_string();
        if word.is_empty() {
            return Err(AppError::InvalidInput("word is required".to_string()));
        }
        if classify_text(&word) != TextKind::Word {
            return Err(AppError::InvalidInput(
                "only single words can be saved to vocabulary".to_string(),
            ));
        }
        database.upsert_vocabulary(&VocabularyUpsert {
            word,
            language: request.language.unwrap_or_else(|| Language::parse("und")),
            translation: request.translation,
            source: request.source.unwrap_or_else(|| "manual".to_string()),
            examples: request.examples,
            bump_lookup: false,
        })?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

// ---------- OCR 截图翻译（Phase 5） ----------

/// 打开全屏选区 overlay（先预检屏幕录制权限，未授予则触发系统授权框）。
#[tauri::command]
pub async fn start_ocr_capture(app: tauri::AppHandle) -> Result<(), AppError> {
    if !lingua_platform::screen_recording_granted() {
        lingua_platform::request_screen_recording_access();
        if !lingua_platform::screen_recording_granted() {
            return Err(AppError::Permission(
                "screen recording permission is required for OCR; grant it in System Settings → \
                 Privacy & Security → Screen & System Audio Recording"
                    .to_string(),
            ));
        }
    }
    crate::ocr::start_capture(&app).await
}

/// 当前 OCR 会话的整屏截图（data URL）；overlay 挂载后拉取。
#[tauri::command]
pub async fn get_ocr_background(app: tauri::AppHandle) -> Result<Option<String>, AppError> {
    crate::ocr::current_background(&app)
}

/// 用户框选完成：坐标为 overlay CSS 像素；viewport 是 overlay 自身的
/// CSS 视口尺寸 —— HiDPI 下像素↔CSS 换算的自校准基准（前端是唯一
/// 可信的 CSS 尺寸来源，Rust 侧按 monitor 推断在缩放显示器上不可靠）。
#[tauri::command]
pub async fn perform_ocr_translation(
    app: tauri::AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    viewport_width: f64,
    viewport_height: f64,
) -> Result<(), AppError> {
    if width < 8.0 || height < 8.0 {
        return Err(AppError::InvalidInput("selection is too small".to_string()));
    }
    crate::ocr::run_ocr_pipeline(
        app,
        lingua_core::Rect {
            x,
            y,
            width,
            height,
        },
        (viewport_width, viewport_height),
    )
    .await;
    Ok(())
}

/// 取消选区（Esc）。
#[tauri::command]
pub async fn cancel_ocr(app: tauri::AppHandle) -> Result<(), AppError> {
    crate::ocr::hide_overlay(&app)
}

/// 屏幕录制权限状态。
#[tauri::command]
pub async fn screen_recording_status() -> Result<bool, AppError> {
    Ok(lingua_platform::screen_recording_granted())
}

/// 当前是否可以「用 Vision 重新识别」：保留了选区截图
/// （上一次结果来自截图翻译）且存在开启视觉能力的 Provider。
#[tauri::command]
pub async fn ocr_retry_available(state: tauri::State<'_, AppState>) -> Result<bool, AppError> {
    let database = state.database.clone();
    let has_capture = state
        .ocr_retry
        .lock()
        .map(|guard| guard.is_some())
        .unwrap_or(false);
    if !has_capture {
        return Ok(false);
    }
    let vision_capable = tauri::async_runtime::spawn_blocking(move || {
        database
            .list_providers()
            .map(|records| records.iter().any(|record| record.can_vision))
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    Ok(vision_capable)
}

/// 手动「用 Vision 重新识别」：对保留的选区截图直接走 Vision 并重翻译。
#[tauri::command]
pub async fn retry_ocr_with_vision(app: tauri::AppHandle) -> Result<(), AppError> {
    crate::ocr::run_vision_retry(app).await;
    Ok(())
}

/// 最近一次截图翻译的识别来源（引擎 / 路径 / 置信度 / 兜底说明）；
/// 划词翻译结果为 `None`。用于区分「识别错了」与「Vision 没触发」。
#[tauri::command]
pub async fn ocr_source_info(
    state: tauri::State<'_, AppState>,
) -> Result<Option<crate::state::OcrSourceInfo>, AppError> {
    Ok(state
        .ocr_source
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or(None))
}

// ---------- 托盘 / 开机自启 ----------

/// 开机自启状态（托盘勾选与设置页开关共用）。
#[tauri::command]
pub async fn autostart_enabled(app: tauri::AppHandle) -> Result<bool, AppError> {
    use tauri_plugin_autostart::ManagerExt;
    Ok(app.autolaunch().is_enabled().unwrap_or(false))
}

/// 设置开机自启；同步托盘菜单的勾选项（两个入口双向一致）。
#[tauri::command]
pub async fn set_autostart(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::TrayState>,
    enabled: bool,
) -> Result<(), AppError> {
    crate::apply_autostart(&app, enabled)?;
    if let Ok(item) = state.autostart_item.lock() {
        let _ = item.set_checked(enabled);
    }
    Ok(())
}
