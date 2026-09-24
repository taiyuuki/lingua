//! OCR 截图翻译：选区 overlay 窗口管理 + 截图→OCR→翻译 pipeline。
//!
//! 流程（需求 OCR Pipeline）：截图 → 预处理（Phase 5 为空操作）→ OCR →
//! 文本合并 → 语言检测/翻译（复用 Translation Engine：短语/句子走 AI）
//! → Popup 展示。原始截图即读即删，不落盘。
//!
//! 坐标约定：overlay 里前端拿到的鼠标事件是 CSS 像素 = 全局逻辑点，
//! 与 `screencapture -R` 的坐标系一致，无需换算。

use crate::pipeline;
use crate::popup;
use crate::{platform_error, state::PopupState};
use lingua_core::{AppError, AppResult, OcrResult, Rect};
use lingua_database::Database;
use lingua_ocr::{HttpOcrProvider, OcrProvider, TesseractOcrProvider};
use lingua_translation::TranslationEngine;
use std::sync::Arc;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// OCR 选区窗口的固定 label（capabilities 里需放行）。
pub const OVERLAY_LABEL: &str = "ocr-selection";

/// settings 表键名。
pub const SETTINGS_OCR_PROVIDER: &str = "ocr.provider";
pub const SETTINGS_OCR_TESSERACT_PATH: &str = "ocr.tesseract_path";
pub const SETTINGS_OCR_HTTP_URL: &str = "ocr.http_url";
pub const OCR_PROVIDER_TESSERACT: &str = "tesseract";
pub const OCR_PROVIDER_HTTP: &str = "http";
pub const DEFAULT_OCR_TESSERACT_PATH: &str = "tesseract";

/// 获取或创建（隐藏的）OCR 选区窗口。
pub fn ensure_overlay(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(existing) = app.get_webview_window(OVERLAY_LABEL) {
        return Ok(existing);
    }
    tauri::webview::WebviewWindowBuilder::new(
        app,
        OVERLAY_LABEL,
        tauri::WebviewUrl::App("ocr.html".into()),
    )
    .title("Lingua OCR")
    .decorations(false)
    .transparent(true) // 需要 tauri.conf 的 macOSPrivateApi
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    .visible(false)
    .build()
    .map_err(platform_error)
}

/// 把 overlay 铺满主显示器并显示、聚焦。
pub async fn show_overlay(app: &AppHandle) -> AppResult<()> {
    let window = ensure_overlay(app)?;
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let position = monitor.position();
        let size = monitor.size();
        window
            .set_position(PhysicalPosition::new(position.x, position.y))
            .map_err(platform_error)?;
        window
            .set_size(PhysicalSize::new(size.width, size.height))
            .map_err(platform_error)?;
    }
    window.show().map_err(platform_error)?;
    window.set_focus().map_err(platform_error)?;
    Ok(())
}

pub fn hide_overlay(app: &AppHandle) -> AppResult<()> {
    if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
        window.hide().map_err(platform_error)?;
    }
    Ok(())
}

/// 从设置构建 OCR Provider。
pub fn build_ocr_provider(database: &Database) -> AppResult<Arc<dyn OcrProvider>> {
    let provider_kind = pipeline::read_setting(database, SETTINGS_OCR_PROVIDER);
    match provider_kind.as_deref() {
        Some(OCR_PROVIDER_HTTP) => {
            let url = pipeline::read_setting(database, SETTINGS_OCR_HTTP_URL)
                .unwrap_or_else(|| lingua_ocr::OCR_HTTP_DEFAULT_URL.to_string());
            Ok(Arc::new(HttpOcrProvider::new(url)))
        }
        _ => {
            let binary = pipeline::read_setting(database, SETTINGS_OCR_TESSERACT_PATH)
                .unwrap_or_else(|| DEFAULT_OCR_TESSERACT_PATH.to_string());
            Ok(Arc::new(TesseractOcrProvider::new(binary)))
        }
    }
}

/// 完整 OCR 翻译流程；失败以 popup 错误态呈现。
pub async fn run_ocr_pipeline(app: AppHandle, rect: Rect) {
    if let Err(error) = run(&app, rect).await {
        tracing::warn!(error = %error, "ocr pipeline failed");
        let popup_state = PopupState::Error {
            kind: error.kind().to_string(),
            message: error.to_string(),
        };
        if let Err(show_error) = popup::show(&app, popup_state).await {
            tracing::error!(error = %show_error, "failed to show ocr error popup");
        }
    }
}

async fn run(app: &AppHandle, rect: Rect) -> AppResult<()> {
    // 1. 先隐藏 overlay：否则遮罩会被截进图里。
    hide_overlay(app)?;

    let state = app.state::<crate::state::AppState>();

    // 2. 截图（阻塞子进程，放阻塞线程池）。
    popup::show(app, PopupState::Loading).await?;
    let database = state.database.clone();
    let capture =
        tauri::async_runtime::spawn_blocking(move || lingua_platform::capture_region(&rect))
            .await
            .map_err(|error| AppError::Internal(error.to_string()))??;
    tracing::debug!(
        width = capture.width,
        height = capture.height,
        "screen captured"
    );

    // 3. OCR。
    let ocr_provider = build_ocr_provider(&database)?;
    let ocr: OcrResult = ocr_provider.recognize(&capture).await?;
    if ocr.is_empty() {
        return Err(AppError::Ocr(
            "no text recognized in the selected area; try a larger selection".to_string(),
        ));
    }
    tracing::debug!(
        blocks = ocr.blocks.len(),
        chars = ocr.text.chars().count(),
        "ocr completed"
    );

    // 4. 翻译（复用划词的 Router + Engine；多行文本会走 AI 分支）。
    let router = pipeline::build_router(&database)?;
    let provider = router.provider_for(lingua_ai::AiTask::Translate)?;
    let dictionary = Arc::new(lingua_dictionary::SqliteDictionaryProvider::new(
        database.clone(),
    ));
    let engine = TranslationEngine::new(provider).with_dictionary(Some(dictionary));
    let request = lingua_core::translation::TranslationRequest::new(
        ocr.text.clone(),
        pipeline::target_language(&database)?,
    );
    let mut result = engine.translate(&request).await?;
    result.source_text = ocr.text;
    if result.detected_language.is_none() {
        result.detected_language = ocr.language.as_deref().map(lingua_core::Language::parse);
    }

    // 5. 结果态。
    popup::show(app, PopupState::Result(Box::new(result))).await?;
    Ok(())
}
