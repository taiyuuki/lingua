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
    /// 学习分析任务路由（`None` = 跟随默认 Provider）。
    pub study_provider_id: Option<String>,
    /// 划词翻译的目标语言（BCP-47 基础码）。
    pub target_language: Language,
    /// 同语言换向目标（BCP-47 基础码；空串 = 关闭）。
    #[serde(default)]
    pub swap_language: String,
    /// 自定义翻译 system prompt（JSON 路径）；空串 = 用内置。
    #[serde(default)]
    pub prompt_translate_system: String,
    /// 自定义翻译 system prompt（流式路径）；空串 = 用内置。
    #[serde(default)]
    pub prompt_translate_stream: String,
    pub shortcut: String,
    /// 截图翻译快捷键（默认 alt+s；多显示器下跟随光标所在屏）。
    pub ocr_shortcut: String,
    /// 界面主题：light | dark。
    pub ui_theme: String,
    /// OCR 引擎：`tesseract` | `http`。
    pub ocr_provider: String,
    pub ocr_tesseract_path: String,
    pub ocr_http_url: String,
    /// 查词自动记入生词本（默认关；Popup 的 ★ 手动收藏不受影响）。
    #[serde(default)]
    pub auto_vocabulary: bool,
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
            study_provider_id: pipeline::read_route_provider(
                &database,
                pipeline::SETTINGS_ROUTE_STUDY,
            ),
            target_language: Language::parse(&pipeline::read_target_language_code(&database)),
            swap_language: pipeline::read_setting(&database, pipeline::SETTINGS_SWAP_LANGUAGE)
                .unwrap_or_else(|| pipeline::DEFAULT_SWAP_LANGUAGE.to_string()),
            prompt_translate_system: pipeline::read_setting(
                &database,
                pipeline::SETTINGS_PROMPT_TRANSLATE_SYSTEM,
            )
            .unwrap_or_default(),
            prompt_translate_stream: pipeline::read_setting(
                &database,
                pipeline::SETTINGS_PROMPT_TRANSLATE_STREAM,
            )
            .unwrap_or_default(),
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
            auto_vocabulary: pipeline::read_setting(&database, pipeline::SETTINGS_AUTO_VOCABULARY)
                .as_deref()
                == Some("true"),
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
        match settings.study_provider_id.as_deref() {
            None => {
                database.remove_setting(pipeline::SETTINGS_ROUTE_STUDY)?;
            }
            Some(id) => {
                database.set_setting(pipeline::SETTINGS_ROUTE_STUDY, &serde_json::json!(id))?
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
            pipeline::SETTINGS_SWAP_LANGUAGE,
            &serde_json::json!(settings.swap_language.trim()),
        )?;
        database.set_setting(
            pipeline::SETTINGS_PROMPT_TRANSLATE_SYSTEM,
            &serde_json::json!(settings.prompt_translate_system.trim()),
        )?;
        database.set_setting(
            pipeline::SETTINGS_PROMPT_TRANSLATE_STREAM,
            &serde_json::json!(settings.prompt_translate_stream.trim()),
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
        database.set_setting(
            pipeline::SETTINGS_AUTO_VOCABULARY,
            &serde_json::json!(if settings.auto_vocabulary {
                "true"
            } else {
                "false"
            }),
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
pub async fn toggle_popup_pin(_app: tauri::AppHandle) -> Result<bool, AppError> {
    // 图钉 = 点击外部不隐藏；窗口置顶始终开启（不再随图钉切换）。
    let next = !crate::popup::popup_pinned();
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
    /// 词典 id。留空则按文件名自动生成（导入表单已不再要求手填）。
    pub id: String,
    pub name: String,
    pub source_language: Language,
    pub gloss_language: Language,
    /// 词典 TSV 文件的绝对路径（Phase 3 用路径输入，文件选择器后续接入）。
    pub path: String,
}

/// FNV-1a 64 位（稳定哈希：跨版本/跨平台生成同样的词典 id）。
fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 从文件名自动生成词典 id：小写 slug + 文件名短哈希后缀。
/// 同名文件重导 → 同 id（覆盖更新）；不同文件基本不撞。
fn generate_dictionary_id(path: &str) -> String {
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("dict");
    let slug = stem
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | '0'..='9' => Some(c),
            'A'..='Z' => Some(c.to_ascii_lowercase()),
            ' ' | '_' | '-' | '.' => Some('-'),
            _ => None,
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base: String = if slug.len() >= 3 {
        slug.chars().take(24).collect()
    } else {
        // 纯中文等无法 slug 化的文件名。
        "dict".to_string()
    };
    format!("{base}-{:06x}", fnv1a(stem) & 0x00ff_ffff)
}

/// 导入 id 裁决：显式填写优先，留空按文件名生成；返回 (id, name)，
/// name 为空时同样回退文件名 stem。
fn resolve_dictionary_identity(request: &ImportDictionaryRequest) -> (String, String) {
    let stem = std::path::Path::new(&request.path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string();
    let id = {
        let trimmed = request.id.trim();
        if trimmed.is_empty() {
            generate_dictionary_id(&request.path)
        } else {
            trimmed.to_string()
        }
    };
    let name = {
        let trimmed = request.name.trim();
        if trimmed.is_empty() {
            stem.clone()
        } else {
            trimmed.to_string()
        }
    };
    (id, name)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDictionaryResult {
    pub imported_entries: usize,
    pub skipped_lines: usize,
    /// MDX 释义引用了 CSS/图片资源，但同目录没有配对的 .mdd
    /// （或解包失败）——释义将以基础排版显示。
    pub resources_missing: bool,
    /// 用户中途取消：本次导入的半成品（词条 + 资源）已清理。
    #[serde(default)]
    pub cancelled: bool,
}

/// 导入取消标志：`cancel_dictionary_import` 置位，导入循环在批边界
/// 检查并停止（5000 条一批，粒度在秒级内）。
static IMPORT_CANCELLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 取消正在进行的词典导入。
#[tauri::command]
pub async fn cancel_dictionary_import() -> Result<(), AppError> {
    IMPORT_CANCELLED.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

/// 从 TSV 文件导入词典（格式见 `lingua_dictionary::tsv` 模块文档）。
#[tauri::command]
pub async fn import_dictionary_tsv(
    state: tauri::State<'_, AppState>,
    request: ImportDictionaryRequest,
) -> Result<ImportDictionaryResult, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // id 留空按文件名生成；name 留空回退文件名 stem。
        let (id, name) = resolve_dictionary_identity(&request);
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
            name,
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
            resources_missing: false,
            cancelled: false,
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

/// 散装资源扩展名：与 .mdx 同目录平铺的样式/字体/图标
/// （OALD10 的 OALD8C.css 布局：css 不在 .mdd 里而是和 mdx 并排）。
const LOOSE_RESOURCE_EXTENSIONS: [&str; 14] = [
    "css", "js", "png", "gif", "jpg", "jpeg", "svg", "webp", "bmp", "ico", "woff", "woff2", "ttf",
    "otf",
];

/// 把 .mdx 同目录的散装资源文件复制进词典资源目录（根层）。
/// 返回复制数量。.js 会被复制但净化器始终剥离 <script>，不会执行。
fn copy_loose_resources(mdx_path: &std::path::Path, resource_dir: &std::path::Path) -> usize {
    let Some(dir) = mdx_path.parent() else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut copied = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_resource = path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    LOOSE_RESOURCE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
                });
        if !is_resource {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if std::fs::copy(&path, resource_dir.join(name)).is_ok() {
            copied += 1;
        }
    }
    copied
}

/// 同目录同名 .mdd 及编号分卷（`name.mdd`、`name.1.mdd`、`name.2.mdd`…），
/// 按编号升序返回存在的分卷路径。
fn sibling_mdd_volumes(mdx_path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let stem = mdx_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string();
    let mut volumes = Vec::new();
    if stem.is_empty() {
        return volumes;
    }
    let main = mdx_path.with_extension("mdd");
    if main.is_file() {
        volumes.push(main);
    }
    let Some(dir) = mdx_path.parent() else {
        return volumes;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return volumes;
    };
    let mut numbered: Vec<(u32, std::path::PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| {
            let name = path.file_name().and_then(|name| name.to_str())?;
            let mid = name
                .strip_prefix(&format!("{stem}."))
                .and_then(|mid| mid.strip_suffix(".mdd"))?;
            if mid.is_empty() || !mid.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            Some((mid.parse::<u32>().ok()?, path))
        })
        .collect();
    numbered.sort_by_key(|(index, _)| *index);
    volumes.extend(numbered.into_iter().map(|(_, path)| path));
    volumes
}

/// 词典包的 HTML 常引用 `O8C.css` 这类短名，而散装 css 的文件名
/// 却是词典名（OALD8 的经典包装：`牛津高阶8简体.css`）。引用的
/// css 在资源目录缺失、且目录里恰好只有一个 css 时，把它按引用
/// 名复制一份作别名。
fn alias_missing_css(resource_dir: &std::path::Path, htmls: &[&str]) {
    use std::collections::BTreeSet;
    let prefix = format!(
        "{}://localhost/",
        lingua_dictionary::html_sanitize::RESOURCE_SCHEME
    );
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    for html in htmls {
        let mut rest = *html;
        while let Some(at) = rest.find(&prefix) {
            let after = &rest[at + prefix.len()..];
            if let Some(slash) = after.find('/') {
                let path = &after[slash + 1..];
                let end = path.find(['"', ' ', '?']).unwrap_or(path.len());
                let path = &path[..end];
                if path.to_ascii_lowercase().ends_with(".css") && !path.contains("..") {
                    referenced.insert(path.to_string());
                }
            }
            rest = &rest[at + prefix.len()..];
        }
    }
    if referenced.is_empty() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(resource_dir) else {
        return;
    };
    let css_files: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("css"))
        })
        .collect();
    if css_files.len() != 1 {
        return;
    }
    let source = &css_files[0];
    for name in referenced {
        let target = resource_dir.join(&name);
        if !target.exists() && std::fs::copy(source, &target).is_ok() {
            tracing::info!(alias = %name, "css alias created");
        }
    }
}

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
        // id 留空按文件名生成；name 留空回退文件名 stem。
        let (id, name) = resolve_dictionary_identity(&request);
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
        // 清掉可能残留的上一次取消信号。
        IMPORT_CANCELLED.store(false, std::sync::atomic::Ordering::SeqCst);
        let dict =
            lingua_dictionary::mdx::MdxFile::open(std::path::Path::new(&request.path))?;
        let total = dict.entry_count();
        if total == 0 {
            return Err(AppError::InvalidInput(format!(
                "no valid entries found in `{}`",
                request.path
            )));
        }

        // 资源收集：同目录 .mdd（含编号分卷 name.1.mdd…）+ 同目录散装
        // 文件（OALD10 的 OALD8C.css 布局），统一落到 app_data/mdx/<id>/，
        // 净化改写过的 mdx-res:// 链接由自定义协议从这里服务。
        // 失败只告警：词条照常导入，HTML 排版缺资源兜底显示。
        let mut mdd_extracted = false;
        let resource_dir = app
            .path()
            .app_data_dir()
            .ok()
            .map(|data_dir| data_dir.join("mdx").join(&id));
        if let Some(resource_dir) = resource_dir.as_ref() {
            let _ = std::fs::remove_dir_all(resource_dir);
            if std::fs::create_dir_all(resource_dir).is_ok() {
                let mut total_extracted = 0usize;
                for volume in sibling_mdd_volumes(std::path::Path::new(&request.path)) {
                    match lingua_dictionary::mdx::extract_mdd_resources(
                        &volume,
                        resource_dir,
                    ) {
                        Ok(count) => {
                            total_extracted += count;
                            tracing::info!(dictionary = %id, volume = %volume.display(), count, "mdd resources extracted");
                        }
                        Err(error) => {
                            tracing::warn!(%error, "mdd extraction failed; rendering without resources");
                        }
                    }
                }
                mdd_extracted = total_extracted > 0;
                let loose = copy_loose_resources(std::path::Path::new(&request.path), resource_dir);
                if loose > 0 {
                    mdd_extracted = true;
                    tracing::info!(dictionary = %id, count = loose, "loose resources copied");
                }
            }
        }
        let meta = DictionaryMeta {
            id: id.clone(),
            name,
            source_language: request.source_language.clone(),
            gloss_language: request.gloss_language.clone(),
        };
        let source_language = request.source_language;
        let gloss_language = request.gloss_language;

        // HTML 净化（html5ever）是导入的 CPU 大头：按批收集原始词条后
        // 用 scoped threads 并行转换，数据库写入保持串行分批提交。
        let mut raw_batch = Vec::with_capacity(MDX_IMPORT_BATCH);
        let mut imported = 0usize;
        let mut skipped = 0usize;
        // 资源缺失检测：首批转换结果里出现 mdx-res:// 引用但 .mdd
        // 没解出来（不存在或解包失败）→ 结果里提示用户补 .mdd 重导。
        // 同一批里顺带做 css 引用名对齐（O8C.css ↔ 词典名.css）。
        let mut resources_missing = false;
        let mut first_batch_done = false;
        let references_resources = |batch: &[lingua_core::DictionaryEntry]| {
            batch.iter().any(|entry| {
                entry.definitions.iter().any(|definition| {
                    definition
                        .html
                        .as_deref()
                        .is_some_and(|html| html.contains(lingua_dictionary::html_sanitize::RESOURCE_SCHEME))
                })
            })
        };
        for entry in dict.entries() {
            if entry.is_redirect() {
                skipped += 1;
                continue;
            }
            raw_batch.push(entry);
            if raw_batch.len() >= MDX_IMPORT_BATCH {
                let batch = lingua_dictionary::mdx::convert_entries_parallel(
                    &raw_batch,
                    &source_language,
                    &gloss_language,
                    &id,
                )?;
                if !first_batch_done {
                    first_batch_done = true;
                    if let Some(resource_dir) = resource_dir.as_ref() {
                        let htmls: Vec<&str> = batch
                            .iter()
                            .flat_map(|entry| entry.definitions.iter())
                            .filter_map(|definition| definition.html.as_deref())
                            .collect();
                        alias_missing_css(resource_dir, &htmls);
                    }
                }
                if !mdd_extracted && !resources_missing {
                    resources_missing = references_resources(&batch);
                }
                if IMPORT_CANCELLED.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    return cancel_import(&database, &app, &meta, resource_dir.as_deref(), imported, skipped, resources_missing);
                }
                imported += database.import_dictionary(&meta, &batch)?;
                emit_progress("importing", imported, total);
                raw_batch.clear();
            }
        }
        if !raw_batch.is_empty() {
            let batch = lingua_dictionary::mdx::convert_entries_parallel(
                &raw_batch,
                &source_language,
                &gloss_language,
                &id,
            )?;
            if !mdd_extracted && !resources_missing {
                resources_missing = references_resources(&batch);
            }
            imported += database.import_dictionary(&meta, &batch)?;
        }
        emit_progress("importing", imported, total);
        tracing::info!(
            dictionary = %meta.id,
            imported,
            skipped,
            total,
            resources_missing,
            "mdx dictionary imported"
        );
        Ok(ImportDictionaryResult {
            imported_entries: imported,
            skipped_lines: skipped,
            resources_missing,
            cancelled: false,
        })
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 取消导入：清理半成品（词典词条 + 资源目录），返回取消结果。
fn cancel_import(
    database: &lingua_database::Database,
    app: &tauri::AppHandle,
    meta: &DictionaryMeta,
    resource_dir: Option<&std::path::Path>,
    imported: usize,
    skipped: usize,
    resources_missing: bool,
) -> Result<ImportDictionaryResult, AppError> {
    database.delete_dictionary(&meta.id)?;
    if let Some(dir) = resource_dir {
        let _ = std::fs::remove_dir_all(dir);
    }
    tracing::info!(dictionary = %meta.id, imported, "mdx import cancelled; partial data cleaned");
    let _ = app.emit(
        "dictionary://import-progress",
        DictionaryImportProgress {
            dictionary_id: meta.id.clone(),
            phase: "importing",
            imported,
            total: imported,
        },
    );
    Ok(ImportDictionaryResult {
        imported_entries: 0,
        skipped_lines: skipped,
        resources_missing,
        cancelled: true,
    })
}

/// 词条内链跳词（`entry://` 链接点击）：按词头查词典。
/// 不做释义语言过滤（弹窗不知道目标语言），取首条有释义的词条。
#[tauri::command]
pub async fn dictionary_lookup(
    state: tauri::State<'_, AppState>,
    word: String,
    language: Option<Language>,
) -> Result<Option<lingua_core::DictionaryEntry>, AppError> {
    let word = word.trim().to_string();
    if word.is_empty() {
        return Ok(None);
    }
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let entries = database.lookup_entries(&word, language)?;
        Ok(entries
            .into_iter()
            .find(lingua_core::DictionaryEntry::has_definitions))
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 多词典词卡：按词头查全部命中词典（导入顺序，含完整释义 HTML），
/// 生词本词卡与弹窗内链跳词共用。
#[tauri::command]
pub async fn lookup_word_dictionaries(
    state: tauri::State<'_, AppState>,
    word: String,
    language: Option<Language>,
) -> Result<Vec<lingua_core::DictionaryEntry>, AppError> {
    let word = word.trim().to_string();
    if word.is_empty() {
        return Ok(Vec::new());
    }
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let entries = database.lookup_entries(&word, language)?;
        Ok(entries
            .into_iter()
            .filter(lingua_core::DictionaryEntry::has_definitions)
            .collect::<Vec<_>>())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 词典手动排序：按传入 id 顺序重写排序（查询命中顺序随之变化）。
#[tauri::command]
pub async fn reorder_dictionaries(
    state: tauri::State<'_, AppState>,
    ids: Vec<String>,
) -> Result<(), AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.reorder_dictionaries(&ids))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

#[tauri::command]
pub async fn delete_dictionary(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), AppError> {
    // 词典的 MDD 资源目录随之清理。
    if let Some(resource_dir) = app
        .path()
        .app_data_dir()
        .ok()
        .map(|data_dir| data_dir.join("mdx").join(id.trim()))
    {
        let _ = std::fs::remove_dir_all(resource_dir);
    }
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        database.delete_dictionary(&id)?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

// ---------- 发音库（AudioLib 形态的 .mdd，按词头定位音频） ----------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAudioLibraryRequest {
    /// 发音库 .mdd 文件的绝对路径。
    pub path: String,
    /// 库名（列表展示用）；空则用文件名。
    pub name: Option<String>,
}

/// 一路口音的播放定位（查词面板的喇叭按钮）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordAudioDto {
    /// "uk" | "us"。
    pub accent: String,
    /// `mdx-res://` 资源 URL（发音库 id 充当词典 id 段，前端照常做平台形态转换）。
    pub url: String,
}

/// 导入发音库：.mdd 收编到 `app_data/audiolibs/<id>.mdd`，词头音频索引
/// 写库，解析句柄进缓存。同文件重导（同 id）覆盖旧库。
/// 几百 MB 的库复制 + 只读元数据段建索引，秒级完成，不做进度事件。
#[tauri::command]
pub async fn import_audio_library(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    request: ImportAudioLibraryRequest,
) -> Result<lingua_database::AudioLibraryInfo, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = request.path.trim();
        if path.is_empty() {
            return Err(AppError::InvalidInput("path is required".to_string()));
        }
        if !path.to_ascii_lowercase().ends_with(".mdd") {
            return Err(AppError::InvalidInput(
                "请选择 .mdd 发音库文件（词典的 .mdx 不能作为发音库导入）".to_string(),
            ));
        }
        let source = std::path::Path::new(path);
        if !source.is_file() {
            return Err(AppError::InvalidInput(format!("文件不存在：{path}")));
        }
        let id = format!("al-{}", generate_dictionary_id(path));
        let name = request
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                source
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("发音库")
                    .to_string()
            });

        // 收编原件（几百 MB 复制，几秒）。
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| AppError::Platform(error.to_string()))?;
        let libraries_dir = data_dir.join("audiolibs");
        std::fs::create_dir_all(&libraries_dir)
            .map_err(|error| AppError::Internal(format!("无法创建发音库目录：{error}")))?;
        let destination = libraries_dir.join(format!("{id}.mdd"));
        std::fs::copy(source, &destination)
            .map_err(|error| AppError::InvalidInput(format!("复制发音库失败（{path}）：{error}")))?;

        // 解析 key 区 + 提取词头索引（只读元数据段，不扫音频数据区）。
        let library = lingua_dictionary::audio::MddResourceFile::open(&destination)?;
        let entries = lingua_dictionary::audio::extract_audio_entries(library.keys());
        if entries.is_empty() {
            let _ = std::fs::remove_file(&destination);
            return Err(AppError::InvalidInput(format!(
                "`{name}` 里没有可按词头定位的音频：需要 word.spx / us\\word.mp3 这类按词头命名的文件"
            )));
        }
        let mut has_uk = false;
        let mut has_us = false;
        let records: Vec<lingua_database::AudioKeyRecord> = entries
            .iter()
            .map(|entry| {
                match entry.accent {
                    lingua_dictionary::audio::Accent::Uk => has_uk = true,
                    lingua_dictionary::audio::Accent::Us => has_us = true,
                }
                lingua_database::AudioKeyRecord {
                    word_lower: entry.word.clone(),
                    accent: entry.accent.as_str().to_string(),
                    resource_path: entry.resource_path.clone(),
                }
            })
            .collect();
        let written = database.replace_audio_library(
            &id,
            &name,
            &destination.display().to_string(),
            &records,
        )?;
        let mut accents = Vec::new();
        if has_uk {
            accents.push("uk".to_string());
        }
        if has_us {
            accents.push("us".to_string());
        }

        // 句柄进缓存：资源协议按 id 直接取，不再查库。
        if let Some(app_state) = app.try_state::<AppState>() {
            if let Ok(mut cache) = app_state.audio_files.lock() {
                cache.insert(id.clone(), std::sync::Arc::new(library));
            }
        }
        tracing::info!(library = %id, keys = written, "audio library imported");
        Ok(lingua_database::AudioLibraryInfo {
            id,
            name,
            entry_count: written,
            accents,
        })
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

#[tauri::command]
pub async fn list_audio_libraries(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<lingua_database::AudioLibraryInfo>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.list_audio_libraries())
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 删除发音库：缓存句柄、收编的 .mdd 文件与库行（词头索引级联）一并清理。
#[tauri::command]
pub async fn delete_audio_library(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), AppError> {
    let id = id.trim().to_string();
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(AppError::InvalidInput("invalid audio library id".to_string()));
    }
    if let Some(app_state) = app.try_state::<AppState>() {
        if let Ok(mut cache) = app_state.audio_files.lock() {
            cache.remove(&id);
        }
    }
    let Ok(data_dir) = app.path().app_data_dir() else {
        return Ok(());
    };
    let _ = std::fs::remove_file(data_dir.join("audiolibs").join(format!("{id}.mdd")));
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        database.delete_audio_library(&id)?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 按词头查发音库音频（大小写不敏感），每种口音取最先导入的命中。
#[tauri::command]
pub async fn lookup_word_audio(
    state: tauri::State<'_, AppState>,
    word: String,
) -> Result<Vec<WordAudioDto>, AppError> {
    let word = word.trim().to_string();
    if word.is_empty() {
        return Ok(Vec::new());
    }
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let hits = database.lookup_audio(&word)?;
        let mut out = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for hit in hits {
            if seen.contains(&hit.accent) {
                continue;
            }
            seen.push(hit.accent.clone());
            let url = format!(
                "{}://localhost/{}/{}",
                lingua_dictionary::html_sanitize::RESOURCE_SCHEME,
                hit.library_id,
                lingua_dictionary::html_sanitize::percent_encode_path(&hit.resource_path),
            );
            out.push(WordAudioDto {
                accent: hit.accent,
                url,
            });
        }
        Ok(out)
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
    pub offset: u32,
}

#[tauri::command]
pub async fn list_vocabulary(
    state: tauri::State<'_, AppState>,
    filter: Option<VocabularyFilterDto>,
) -> Result<Vec<lingua_core::VocabularyItem>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let filter = vocabulary_filter_from(filter.unwrap_or_default());
        database.list_vocabulary(&filter)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 与筛选条件匹配的词条总数（分页用）。
#[tauri::command]
pub async fn vocabulary_count(
    state: tauri::State<'_, AppState>,
    filter: Option<VocabularyFilterDto>,
) -> Result<usize, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let filter = vocabulary_filter_from(filter.unwrap_or_default());
        database.count_vocabulary(&filter)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

fn vocabulary_filter_from(dto: VocabularyFilterDto) -> VocabularyFilter {
    VocabularyFilter {
        search: dto.search,
        tag: dto.tag,
        language: dto.language,
        mastery: dto.mastery,
        limit: dto.limit,
        offset: dto.offset,
    }
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

/// 批量删除生词（单事务）。返回实际删除条数。
#[tauri::command]
pub async fn vocabulary_batch_delete(
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<usize, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.batch_delete_vocabulary(&ids))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 批量更新生词：设掌握度和/或追加标签（去重）。返回实际更新条数。
#[tauri::command]
pub async fn vocabulary_batch_update(
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
    mastery: Option<Mastery>,
    add_tags: Option<Vec<String>>,
) -> Result<usize, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        database.batch_update_vocabulary(&ids, mastery, add_tags.as_deref().unwrap_or(&[]))
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 复习评分（0=忘记 1=记得 2=简单）：打点 + SRS-lite 重排期。
#[tauri::command]
pub async fn grade_vocabulary_review(
    state: tauri::State<'_, AppState>,
    id: i64,
    result: i64,
) -> Result<bool, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.grade_review(id, result))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 到期待复习条数。
#[tauri::command]
pub async fn due_vocabulary_count(state: tauri::State<'_, AppState>) -> Result<usize, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.due_vocabulary_count())
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 到期复习队列（最早到期优先）。
#[tauri::command]
pub async fn list_due_vocabulary(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<lingua_core::VocabularyItem>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.list_due_vocabulary(limit.unwrap_or(20)))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 按语言分组计数（生词本分组筛选用）。
#[tauri::command]
pub async fn vocabulary_language_counts(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<(String, usize)>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.vocabulary_language_counts())
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

// ---------- 生词本导出（Anki TSV） ----------

/// 把词条字段净化为单行 TSV 字段：tab / 换行会破坏行结构，折叠为空格。
fn flatten_tsv_field(field: &str) -> String {
    field
        .split(['\t', '\r', '\n'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Anki 可直接导入的 TSV（`文件 → 导入`，无需插件）。
/// 头部指令声明分隔符与标签列；列：Word / Meaning / Examples / Notes，
/// 第 5 列为 Anki 标签（词条标签 + 语言）。
fn build_anki_tsv(items: &[lingua_core::VocabularyItem]) -> String {
    let mut out = String::from(
        "#separator:tab\n#html:false\n#columns:Word\tMeaning\tExamples\tNotes\n#tags column:5\n",
    );
    for item in items {
        let word = flatten_tsv_field(&item.word);
        let meaning = item
            .translation
            .as_deref()
            .map(flatten_tsv_field)
            .unwrap_or_default();
        let examples = item
            .examples
            .iter()
            .map(|example| flatten_tsv_field(example))
            .filter(|example| !example.is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        let notes = flatten_tsv_field(&item.notes);
        let mut tags = item.tags.clone();
        tags.push(item.language.code().to_string());
        let tags = tags
            .iter()
            .map(|tag| flatten_tsv_field(tag))
            .filter(|tag| !tag.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str(&format!("{word}\t{meaning}\t{examples}\t{notes}\t{tags}\n"));
    }
    out
}

/// 导出全部生词为 Anki 可导入的 TSV。返回导出条数。
#[tauri::command]
pub async fn export_vocabulary_anki(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<usize, AppError> {
    if path.trim().is_empty() {
        return Err(AppError::InvalidInput("path is required".to_string()));
    }
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let items = database.list_vocabulary(&lingua_database::VocabularyFilter {
            limit: 1_000_000,
            ..Default::default()
        })?;
        let count = items.len();
        std::fs::write(&path, build_anki_tsv(&items))
            .map_err(|error| AppError::Internal(format!("failed to write export file: {error}")))?;
        Ok(count)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

// ---------- 学习分析（Phase 9） ----------

/// 学习分析请求 DTO（Popup 结果态发起）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyRequestDto {
    pub text: String,
    pub language: Option<Language>,
    /// 已有译文（帮助模型保持一致；可空）。
    pub translation: Option<String>,
}

/// 学习分析：经 Router 的 Study 任务（未配置路由时跟随默认 Provider）
/// 对文本做单词深析 / 句子解构，返回结构化学习卡片。
#[tauri::command]
pub async fn analyze_study(
    state: tauri::State<'_, AppState>,
    request: StudyRequestDto,
) -> Result<lingua_translation::StudyAnalysis, AppError> {
    let text = request.text.trim().to_string();
    if text.is_empty() {
        return Err(AppError::InvalidInput("text is required".to_string()));
    }
    let database = state.database.clone();
    let router = pipeline::build_router(&database)?;
    let provider = router.provider_for(lingua_ai::AiTask::Study)?;
    let analyzer = lingua_translation::StudyAnalyzer::new(provider);
    let study_request = lingua_translation::StudyRequest {
        text,
        source_language: request.language,
        target_language: pipeline::target_language(&database)?,
        translation: request.translation,
    };
    analyzer.analyze(&study_request).await
}

// ---------- 翻译历史（Phase 8） ----------

/// 历史列表筛选（search 匹配原文/译文；kind 区分划词/截图入口）。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HistoryFilterDto {
    pub search: Option<String>,
    pub kind: Option<lingua_core::HistoryKind>,
    pub limit: u32,
    pub offset: u32,
}

fn history_filter_from(dto: HistoryFilterDto) -> lingua_database::HistoryFilter {
    lingua_database::HistoryFilter {
        search: dto.search,
        kind: dto.kind,
        limit: dto.limit,
        offset: dto.offset,
    }
}

#[tauri::command]
pub async fn list_history(
    state: tauri::State<'_, AppState>,
    filter: Option<HistoryFilterDto>,
) -> Result<Vec<lingua_core::HistoryItem>, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let filter = history_filter_from(filter.unwrap_or_default());
        database.list_history(&filter)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 与筛选条件匹配的历史条数（分页用）。
#[tauri::command]
pub async fn history_count(
    state: tauri::State<'_, AppState>,
    filter: Option<HistoryFilterDto>,
) -> Result<usize, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let filter = history_filter_from(filter.unwrap_or_default());
        database.count_history(&filter)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

#[tauri::command]
pub async fn delete_history(state: tauri::State<'_, AppState>, id: i64) -> Result<bool, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.delete_history(id))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

/// 清空全部历史。返回删除条数。
#[tauri::command]
pub async fn clear_history(state: tauri::State<'_, AppState>) -> Result<usize, AppError> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.clear_history())
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use lingua_core::Mastery;

    fn vocab_item(
        word: &str,
        translation: Option<&str>,
        examples: &[&str],
        notes: &str,
        tags: &[&str],
    ) -> lingua_core::VocabularyItem {
        lingua_core::VocabularyItem {
            id: 1,
            word: word.into(),
            language: Language::En,
            translation: translation.map(str::to_string),
            notes: notes.into(),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            mastery: Mastery::New,
            lookup_count: 1,
            review_count: 0,
            source: "ai".into(),
            examples: examples.iter().map(|example| example.to_string()).collect(),
            created_at: "2026-01-01T00:00:00.000Z".into(),
            last_seen_at: "2026-01-01T00:00:00.000Z".into(),
        }
    }

    #[test]
    fn anki_tsv_has_header_and_rows() {
        let items = vec![
            vocab_item(
                "ephemeral",
                Some("短暂的"),
                &["an ephemeral moment"],
                "GRE 高频",
                &["gre"],
            ),
            vocab_item("猫", None, &[], "", &[]),
        ];
        let tsv = build_anki_tsv(&items);
        let mut lines = tsv.lines();
        assert_eq!(lines.next(), Some("#separator:tab"));
        assert_eq!(lines.next(), Some("#html:false"));
        assert_eq!(
            lines.next(),
            Some("#columns:Word\tMeaning\tExamples\tNotes")
        );
        assert_eq!(lines.next(), Some("#tags column:5"));
        let row1 = lines.next().unwrap();
        let cells: Vec<&str> = row1.split('\t').collect();
        assert_eq!(cells[0], "ephemeral");
        assert_eq!(cells[1], "短暂的");
        assert_eq!(cells[2], "an ephemeral moment");
        assert_eq!(cells[3], "GRE 高频");
        assert_eq!(cells[4], "gre en");
        let row2 = lines.next().unwrap();
        let cells: Vec<&str> = row2.split('\t').collect();
        assert_eq!(cells[0], "猫");
        assert_eq!(cells[1], "", "无译文留空");
        assert_eq!(cells[2], "");
        assert_eq!(cells[4], "en", "无标签也带语言标签");
        assert_eq!(lines.next(), None);
    }

    #[test]
    fn anki_tsv_fields_are_flattened() {
        let items = vec![vocab_item(
            "multi\nline\tword",
            Some("含\t制表符"),
            &["例句一\n第二行", ""],
            "笔记\r\n换行",
            &["tag with\nnewline"],
        )];
        let tsv = build_anki_tsv(&items);
        let row = tsv.lines().last().unwrap();
        assert_eq!(row.split('\t').count(), 5, "字段内换行/制表符已折叠: {row}");
    }

    #[test]
    fn generated_ids_are_slugged_and_stable() {
        let a = generate_dictionary_id(r"C:\dicts\Oxford Advanced.mdx");
        let b = generate_dictionary_id(r"D:\other\Oxford Advanced.mdx");
        assert!(a.starts_with("oxford-advanced-"), "{a}");
        assert_eq!(a, b, "同名文件 → 同 id（重导覆盖更新）");
        // 纯中文文件名 slug 化为空，回退 dict- 前缀。
        let cjk = generate_dictionary_id(r"C:\词库\牛津高阶.mdx");
        assert!(cjk.starts_with("dict-"), "{cjk}");
        let tsv = generate_dictionary_id("my_words.tsv");
        assert!(tsv.starts_with("my-words-"), "{tsv}");
    }

    #[test]
    fn identity_falls_back_to_filename() {
        let request = ImportDictionaryRequest {
            id: "".into(),
            name: "".into(),
            source_language: Language::En,
            gloss_language: Language::Zh,
            path: r"C:\d\My Dict.mdx".into(),
        };
        let (id, name) = resolve_dictionary_identity(&request);
        assert!(id.starts_with("my-dict-"), "{id}");
        assert_eq!(name, "My Dict");
    }

    #[test]
    fn numbered_mdd_volumes_collected_in_numeric_order() {
        let dir = std::env::temp_dir().join("lingua-mdd-vol-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mdx = dir.join("dict.mdx");
        std::fs::write(&mdx, b"x").unwrap();
        for name in [
            "dict.mdd",
            "dict.2.mdd",
            "dict.10.mdd",
            "dict.1.mdd",
            "dictx.mdd",
            "dict.css.mdd",
            "dict.a1.mdd",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let volumes = sibling_mdd_volumes(&mdx);
        let names: Vec<String> = volumes
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["dict.mdd", "dict.1.mdd", "dict.2.mdd", "dict.10.mdd"],
            "{names:?}"
        );
    }

    #[test]
    fn css_alias_created_for_missing_reference() {
        let dir = std::env::temp_dir().join("lingua-css-alias-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("牛津高阶8简体.css"), b"body{}").unwrap();
        let html = r#"<link rel="stylesheet" href="mdx-res://localhost/d1/O8C.css">"#;
        alias_missing_css(&dir, &[html]);
        assert!(dir.join("O8C.css").is_file(), "应按引用名建立别名");
        // 再次运行不覆盖已有别名。
        std::fs::write(dir.join("O8C.css"), b"kept").unwrap();
        alias_missing_css(&dir, &[html]);
        assert_eq!(std::fs::read(dir.join("O8C.css")).unwrap(), b"kept");
    }

    /// 发音库导入全链路（真机文件，默认忽略）：
    /// `cargo test -p lingua --lib imports_real -- --ignored --nocapture`
    /// 复制收编 → key 解析 → 词头索引入库 → 查词 → 随机读取 → Speex 转码。
    #[test]
    #[ignore = "依赖本机的英式美式发音库文件"]
    fn imports_real_sound_library_end_to_end() {
        use lingua_database::AudioKeyRecord;
        let source = std::path::Path::new(
            r"C:\Users\falwy\Documents\dict词库\发音库\英式美式发音库\Sound_us_uk.mdd",
        );
        if !source.is_file() {
            return;
        }

        let dir = std::env::temp_dir().join("lingua-audio-e2e");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("al-sound.mdd");
        std::fs::copy(source, &destination).unwrap();

        let library = lingua_dictionary::audio::MddResourceFile::open(&destination).unwrap();
        let entries = lingua_dictionary::audio::extract_audio_entries(library.keys());
        assert!(entries.len() > 200_000, "{}", entries.len());

        let database = lingua_database::Database::open(&dir.join("e2e.sqlite3")).unwrap();
        database.migrate().unwrap();
        let records: Vec<AudioKeyRecord> = entries
            .iter()
            .map(|entry| AudioKeyRecord {
                word_lower: entry.word.clone(),
                accent: entry.accent.as_str().to_string(),
                resource_path: entry.resource_path.clone(),
            })
            .collect();
        let written = database
            .replace_audio_library("al-e2e", "英式美式发音库", &destination.display().to_string(), &records)
            .unwrap();
        assert_eq!(written, entries.len(), "词头索引应全部写入");

        // 查词 → 命中英/美两路 → 随机读取 → Speex 解码为可播放 WAV。
        let hits = database.lookup_audio("The").unwrap();
        assert_eq!(hits.len(), 2, "the 应有英/美两路");
        for hit in hits {
            let data = library.read_resource(&hit.resource_path).unwrap().unwrap();
            let wav = crate::speex::speex_to_wav(&data).expect("spx 应能解码为 WAV");
            assert!(wav.len() > 1000, "{}", wav.len());
            assert_eq!(&wav[0..4], b"RIFF", "应是 WAV 容器");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
