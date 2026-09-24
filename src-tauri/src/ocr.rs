//! OCR 截图翻译：快照式选区 overlay + 截图→OCR→翻译 pipeline。
//!
//! 快照式设计（截图类工具的标准做法）：`start_ocr_capture` 先截取
//! 主显示器整屏，overlay 显示这张「冻结截图」，用户在图上框选。
//! 好处：
//!
//! * 不依赖窗口透明 / macOSPrivateApi；
//! * 所见即所选，遮罩永远不会被截进图；
//! * 选区坐标（CSS 像素 = 逻辑点）× 会话里存的缩放比 = 像素坐标，
//!   直接在整屏截图上裁剪后送 OCR。
//!
//! 整屏截图只存在内存会话里，选区裁剪即读即用，不落盘。

use crate::pipeline;
use crate::popup;
use crate::{platform_error, state::AppState, state::OcrSession, state::PopupState};
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

/// 获取或创建选区窗口（不透明，内容是整屏截图）。
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

/// 启动截图会话：截主显示器整屏 → 存入内存会话 → 显示选区 overlay。
pub async fn start_capture(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();

    let monitor = app
        .primary_monitor()
        .map_err(platform_error)?
        .ok_or_else(|| AppError::Platform("no primary monitor found".to_string()))?;
    let position = monitor.position();
    let size = monitor.size();
    let monitor_rect = Rect {
        x: f64::from(position.x),
        y: f64::from(position.y),
        width: f64::from(size.width),
        height: f64::from(size.height),
    };

    let image = tauri::async_runtime::spawn_blocking(move || {
        lingua_platform::capture_region(&monitor_rect)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))??;
    tracing::debug!(
        width = image.width,
        height = image.height,
        "ocr snapshot captured"
    );

    let data_url = format!(
        "data:image/png;base64,{}",
        lingua_core::base64_encode(&image.png)
    );

    {
        let mut guard = state
            .ocr
            .lock()
            .map_err(|_| AppError::Internal("ocr session lock poisoned".into()))?;
        *guard = Some(OcrSession {
            image,
            background_data_url: data_url,
            logical_width: monitor_rect.width,
            logical_height: monitor_rect.height,
        });
    }

    show_overlay(app).await
}

/// 当前会话的整屏截图（data URL），overlay 用它做选区背景。
pub fn current_background(app: &AppHandle) -> AppResult<Option<String>> {
    let state = app.state::<AppState>();
    let guard = state
        .ocr
        .lock()
        .map_err(|_| AppError::Internal("ocr session lock poisoned".into()))?;
    Ok(guard
        .as_ref()
        .map(|session| session.background_data_url.clone()))
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
    // 1. 立即隐藏 overlay（屏幕截图已在会话里，不再引用真实屏幕）。
    hide_overlay(app)?;

    let state = app.state::<AppState>();

    // 2. 取出快照会话，把逻辑坐标换算成像素坐标并裁剪。
    let session = {
        let mut guard = state
            .ocr
            .lock()
            .map_err(|_| AppError::Internal("ocr session lock poisoned".into()))?;
        guard.take()
    }
    .ok_or_else(|| AppError::InvalidInput("no active OCR capture session".to_string()))?;

    let scale_x = f64::from(session.image.width) / session.logical_width;
    let scale_y = f64::from(session.image.height) / session.logical_height;
    let px_x = ((rect.x * scale_x).round() as i64).clamp(0, i64::from(session.image.width)) as u32;
    let px_y = ((rect.y * scale_y).round() as i64).clamp(0, i64::from(session.image.height)) as u32;
    let px_width = ((rect.width * scale_x).round() as u32).min(session.image.width - px_x);
    let px_height = ((rect.height * scale_y).round() as u32).min(session.image.height - px_y);
    tracing::debug!(
        px_x,
        px_y,
        px_width,
        px_height,
        image_width = session.image.width,
        image_height = session.image.height,
        "cropping ocr selection"
    );

    popup::show(app, PopupState::Loading).await?;
    let capture = tauri::async_runtime::spawn_blocking(move || {
        lingua_platform::crop_image(&session.image, [px_x, px_y, px_width, px_height])
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))??;

    // 3. OCR。
    let ocr_provider = build_ocr_provider(&state.database.clone())?;
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

    // 4. 翻译（复用划词的 Router + Engine；多行文本走 AI 分支）。
    let router = pipeline::build_router(&state.database.clone())?;
    let ai_provider = router.provider_for(lingua_ai::AiTask::Translate)?;
    let dictionary = Arc::new(lingua_dictionary::SqliteDictionaryProvider::new(
        state.database.clone(),
    ));
    let engine = TranslationEngine::new(ai_provider).with_dictionary(Some(dictionary));
    let request = lingua_core::translation::TranslationRequest::new(
        ocr.text.clone(),
        pipeline::target_language(&state.database)?,
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
