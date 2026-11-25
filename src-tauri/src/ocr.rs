//! OCR 截图翻译：快照式选区 overlay + 截图→OCR→翻译 pipeline。
//!
//! 快照式设计（截图类工具的标准做法）：`start_capture` 先把**每一块显示器**
//! 截成冻结截图，每块屏铺一个 overlay 窗口显示对应的快照，用户在图上框选。
//! 好处：
//!
//! * 不依赖窗口透明 / macOSPrivateApi；
//! * 所见即所选，遮罩永远不会被截进图；
//! * 选区用「虚拟桌面逻辑坐标」在所有 overlay 间同步，拖拽可以从一块屏
//!   起笔、拖到另一块屏松手（见 `lingua_platform::DesktopCapture`）；
//! * 每块屏的选区坐标（CSS 像素 = 逻辑点）× 该屏截图的缩放比 = 像素坐标，
//!   直接在快照上裁剪后送 OCR。
//!
//! 整屏截图只存在内存会话里，选区裁剪即读即用，不落盘。

use crate::pipeline;
use crate::popup;
use crate::{platform_error, state::AppState, state::PopupState};
use lingua_core::{AppError, AppResult, OcrResult, Rect};
use lingua_database::Database;
use lingua_ocr::{HttpOcrProvider, OcrProvider, TesseractOcrProvider};
use lingua_platform::{DesktopCapture, MonitorCapture, MonitorGeometry};
use lingua_translation::TranslationEngine;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// OCR 选区窗口的 label 前缀：每块显示器一个窗口，`ocr-selection-{显示器下标}`
/// （capabilities 以 `ocr-selection-*` 放行）。
pub const OVERLAY_LABEL_PREFIX: &str = "ocr-selection-";
/// 会话更新通知：overlay 收到后重新拉取背景截图。
/// 窗口是复用的（Vue 只挂载一次），所以每次 start 都要推一次。
pub const BACKGROUND_UPDATED_EVENT: &str = "ocr://background-updated";

/// 截图会话进行中标记：快捷键连按时忽略重复触发，免得两次截图互相把对方的
/// overlay 截进画面。
static CAPTURING: AtomicBool = AtomicBool::new(false);

/// settings 表键名。
pub const SETTINGS_OCR_PROVIDER: &str = "ocr.provider";
pub const SETTINGS_OCR_TESSERACT_PATH: &str = "ocr.tesseract_path";
pub const SETTINGS_OCR_HTTP_URL: &str = "ocr.http_url";
pub const OCR_PROVIDER_TESSERACT: &str = "tesseract";
pub const OCR_PROVIDER_HTTP: &str = "http";
pub const DEFAULT_OCR_TESSERACT_PATH: &str = "tesseract";

fn overlay_label(index: usize) -> String {
    format!("{OVERLAY_LABEL_PREFIX}{index}")
}

/// overlay 窗口 label → 显示器下标；非 overlay 窗口为 `None`。
fn overlay_index(label: &str) -> Option<usize> {
    label.strip_prefix(OVERLAY_LABEL_PREFIX)?.parse().ok()
}

/// 获取或创建第 `index` 块显示器的选区窗口（不透明，内容是该屏截图）。
fn ensure_overlay(app: &AppHandle, index: usize) -> AppResult<WebviewWindow> {
    if let Some(existing) = app.get_webview_window(&overlay_label(index)) {
        return Ok(existing);
    }
    tauri::webview::WebviewWindowBuilder::new(
        app,
        overlay_label(index),
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

/// 把第 `index` 块屏的 overlay 铺满对应显示器（保持当前的显示 / 隐藏状态）。
/// 与快照截的是同一块屏幕，选区坐标（overlay CSS）与截图天然对齐。
fn place_overlay(
    app: &AppHandle,
    index: usize,
    monitor: &tauri::Monitor,
) -> AppResult<WebviewWindow> {
    let window = ensure_overlay(app, index)?;
    let position = monitor.position();
    let size = monitor.size();
    window
        .set_position(PhysicalPosition::new(position.x, position.y))
        .map_err(platform_error)?;
    window
        .set_size(PhysicalSize::new(size.width, size.height))
        .map_err(platform_error)?;
    Ok(window)
}

/// 隐藏全部 overlay；返回隐藏前是否有可见的（调用方据此决定截图前要不要
/// 等一拍，让系统把窗口从屏幕上撤下）。
pub fn hide_overlays(app: &AppHandle) -> AppResult<bool> {
    let mut was_visible = false;
    for (label, window) in app.webview_windows() {
        if !label.starts_with(OVERLAY_LABEL_PREFIX) {
            continue;
        }
        if window.is_visible().map_err(platform_error)? {
            was_visible = true;
            window.hide().map_err(platform_error)?;
        }
    }
    Ok(was_visible)
}

/// 显示器几何（物理像素矩形 + 缩放比），截屏与逻辑坐标换算的输入。
fn geometry_of(monitor: &tauri::Monitor) -> MonitorGeometry {
    let position = monitor.position();
    let size = monitor.size();
    MonitorGeometry {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        scale_factor: monitor.scale_factor(),
    }
}

/// 光标所在显示器在本次布局里的下标：焦点 overlay 跟着光标走
/// （Esc 在那块屏生效）；对不上时退回第一块。
fn cursor_monitor_index(app: &AppHandle, geometries: &[MonitorGeometry]) -> usize {
    crate::popup::cursor_monitor(app)
        .map(|monitor| geometry_of(&monitor))
        .and_then(|cursor| geometries.iter().position(|geometry| *geometry == cursor))
        .unwrap_or(0)
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

/// 触发 Vision 兜底的块平均置信度阈值（tesseract/Paddle 的 conf 已归一到 0..1）。
const VISION_FALLBACK_CONFIDENCE: f32 = 0.6;

/// 块平均置信度；无块结构（部分 HTTP 引擎不返回块）时 `None`。
fn mean_confidence(ocr: &OcrResult) -> Option<f32> {
    if ocr.blocks.is_empty() {
        return None;
    }
    Some(ocr.blocks.iter().map(|block| block.confidence).sum::<f32>() / ocr.blocks.len() as f32)
}

/// 普通 OCR 结果是否需要 Vision 兜底：空结果，或块平均置信度偏低。
fn needs_vision_fallback(ocr: &OcrResult) -> bool {
    if ocr.is_empty() {
        return true;
    }
    matches!(mean_confidence(ocr), Some(mean) if mean < VISION_FALLBACK_CONFIDENCE)
}

/// 普通引擎直出的来源信息。
fn primary_source(
    engine: String,
    confidence: Option<f32>,
    vision_note: Option<&str>,
) -> crate::state::OcrSourceInfo {
    crate::state::OcrSourceInfo {
        engine,
        via: "primary".to_string(),
        confidence,
        vision_note: vision_note.map(str::to_string),
    }
}

/// 把识别来源记入全局状态（Popup 展示用；锁中毒时静默跳过）。
fn remember_ocr_source(state: &AppState, info: crate::state::OcrSourceInfo) {
    if let Ok(mut guard) = state.ocr_source.lock() {
        *guard = Some(info);
    }
}

/// 普通 OCR → Vision LLM 高级 fallback（Phase 6）。
/// Vision 不可用（未配置 / 无能力 Provider）或识别失败时**不阻断**：
/// 退回原始结果，由空结果错误路径给用户提示。
/// 返回结果 + 来源信息（哪个引擎、哪条路径、置信度），Popup 据此
/// 让用户能区分「识别错了」和「Vision 根本没触发」。
async fn recognize_with_vision_fallback(
    database: &Database,
    capture: &lingua_core::OcrImage,
) -> AppResult<(OcrResult, crate::state::OcrSourceInfo)> {
    let provider = build_ocr_provider(database)?;
    let engine = provider.name().to_string();
    let primary = provider.recognize(capture).await?;
    let primary_confidence = mean_confidence(&primary);

    if !needs_vision_fallback(&primary) {
        return Ok((primary, primary_source(engine, primary_confidence, None)));
    }

    tracing::debug!("ocr result empty or low-confidence; trying vision fallback");
    let vision_provider = pipeline::build_router(database)
        .and_then(|router| router.provider_for(lingua_ai::AiTask::Vision));
    let Some(ai_provider) = vision_provider.ok() else {
        tracing::debug!("no vision-capable provider configured; skipping fallback");
        return Ok((
            primary,
            primary_source(
                engine,
                primary_confidence,
                Some("未配置视觉能力的 Provider，未走 Vision 兜底"),
            ),
        ));
    };
    let vision_engine = format!("vision:{}", ai_provider.id());
    let vision = lingua_ocr::VisionOcrProvider::new(ai_provider);
    match vision.recognize(capture).await {
        Ok(result) if !result.is_empty() => {
            tracing::debug!(
                chars = result.text.chars().count(),
                "ocr recovered via vision fallback"
            );
            Ok((
                result,
                crate::state::OcrSourceInfo {
                    engine: vision_engine,
                    via: "auto".to_string(),
                    confidence: None,
                    vision_note: None,
                },
            ))
        }
        Ok(_) => {
            tracing::debug!("vision fallback returned no text; keeping primary result");
            Ok((
                primary,
                primary_source(
                    engine,
                    primary_confidence,
                    Some("Vision 识别为空，保留原始结果"),
                ),
            ))
        }
        Err(error) => {
            tracing::warn!(error = %error, "vision fallback failed; keeping primary result");
            Ok((
                primary,
                primary_source(
                    engine,
                    primary_confidence,
                    Some(&format!("Vision 调用失败，保留原始结果：{error}")),
                ),
            ))
        }
    }
}

/// 截图进行中标记的守卫：`start_capture` 以任何方式退出（含 panic）都复位。
struct CaptureGuard;

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CAPTURING.store(false, Ordering::SeqCst);
    }
}

/// 启动截图会话：把**所有**显示器截成快照存入内存会话，并在每块屏上显示
/// 对应的选区 overlay —— 框选可以从一块屏起笔、拖到另一块屏松手；
/// 焦点给光标所在那块屏的 overlay。
pub async fn start_capture(app: &AppHandle) -> AppResult<()> {
    if CAPTURING.swap(true, Ordering::SeqCst) {
        tracing::debug!("ocr capture already in progress; ignoring retrigger");
        return Ok(());
    }
    let _guard = CaptureGuard;
    capture_desktop(app).await
}

async fn capture_desktop(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();

    // 0. overlay 还在屏幕上（快捷键连按）→ 先全部撤下并等一拍，
    //    免得把上一帧的冻结画面截进新快照。
    if hide_overlays(app)? {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    // 1. 枚举显示器；一块都取不到时退回主显示器。
    let mut monitors = app.available_monitors().map_err(platform_error)?;
    if monitors.is_empty() {
        monitors = app
            .primary_monitor()
            .map_err(platform_error)?
            .into_iter()
            .collect();
    }
    if monitors.is_empty() {
        return Err(AppError::Platform("no monitor found".to_string()));
    }
    let geometries: Vec<MonitorGeometry> = monitors.iter().map(geometry_of).collect();
    let focus_index = cursor_monitor_index(app, &geometries);

    // 2. overlay 窗口就位（保持隐藏）：窗口几何 = 目标显示器。
    for (index, monitor) in monitors.iter().enumerate() {
        place_overlay(app, index, monitor)?;
    }

    // 3. 逐屏截图：每块屏一个阻塞任务，并行。
    let mut pending: Vec<_> = geometries
        .iter()
        .map(|geometry| {
            let geometry = *geometry;
            tauri::async_runtime::spawn_blocking(move || {
                lingua_platform::capture_monitor(&geometry)
            })
        })
        .collect();
    let mut captures = Vec::with_capacity(pending.len());
    for geometry in &geometries {
        let image = pending
            .remove(0)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))??;
        tracing::debug!(
            width = image.width,
            height = image.height,
            "ocr snapshot captured"
        );
        captures.push(MonitorCapture {
            image,
            rect: geometry.logical_rect(),
        });
    }

    let backgrounds: Vec<String> = captures
        .iter()
        .map(|capture| {
            format!(
                "data:image/png;base64,{}",
                lingua_core::base64_encode(&capture.image.png)
            )
        })
        .collect();

    {
        let mut guard = state
            .ocr
            .lock()
            .map_err(|_| AppError::Internal("ocr session lock poisoned".into()))?;
        *guard = Some(crate::state::OcrSession {
            desktop: DesktopCapture::new(captures)?,
            backgrounds,
        });
    }

    // 4. 通知各 overlay 拉取新背景，再逐块显示。窗口隐藏时 webview 仍然存活、
    //    能收事件；放在 show 之前，尽量让新图在窗口可见前就位。
    let count = monitors.len();
    for index in 0..count {
        app.emit_to(overlay_label(index), BACKGROUND_UPDATED_EVENT, ())
            .map_err(platform_error)?;
    }
    for index in 0..count {
        ensure_overlay(app, index)?.show().map_err(platform_error)?;
    }
    ensure_overlay(app, focus_index)?
        .set_focus()
        .map_err(platform_error)?;
    tracing::debug!(monitors = count, focus = focus_index, "ocr overlays shown");
    Ok(())
}

/// overlay 窗口的归属：本窗口铺在哪块显示器上（虚拟桌面逻辑坐标矩形）
/// + 该屏的冻结截图。前端据此把本地 CSS 坐标换算成全局坐标。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrSurface {
    pub monitor_index: usize,
    /// 该显示器在虚拟桌面逻辑坐标系里的矩形；overlay 的 CSS 像素相对它定位。
    pub rect: Rect,
    /// 该显示器的冻结截图（data URL）。
    pub background: String,
}

/// 当前会话里某个 overlay 窗口对应的显示器快照；没有活动会话
/// （或该窗口对应的显示器这次没截到）为 `None`。
pub fn current_surface(window: &WebviewWindow) -> AppResult<Option<OcrSurface>> {
    let Some(index) = overlay_index(window.label()) else {
        return Ok(None);
    };
    let state = window.state::<AppState>();
    let guard = state
        .ocr
        .lock()
        .map_err(|_| AppError::Internal("ocr session lock poisoned".into()))?;
    Ok(guard.as_ref().and_then(|session| {
        let monitor = session.desktop.monitors.get(index)?;
        let background = session.backgrounds.get(index)?;
        Some(OcrSurface {
            monitor_index: index,
            rect: monitor.rect,
            background: background.clone(),
        })
    }))
}

/// 完整 OCR 翻译流程；失败以 popup 错误态呈现。
/// `rect` 是发起框选的那个 overlay 的 CSS 像素选区（跨屏拖拽时可以越出该屏），
/// `viewport` 是该 overlay 的 CSS 视口尺寸（HiDPI 自校准基准）。
pub async fn run_ocr_pipeline(
    app: AppHandle,
    window: WebviewWindow,
    rect: Rect,
    viewport: (f64, f64),
) {
    if let Err(error) = run(&app, &window, rect, viewport).await {
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

async fn run(
    app: &AppHandle,
    window: &WebviewWindow,
    rect: Rect,
    viewport: (f64, f64),
) -> AppResult<()> {
    // 1. 立即隐藏 overlay（屏幕截图已在会话里，不再引用真实屏幕）。
    hide_overlays(app)?;

    let state = app.state::<AppState>();

    // 2. 取出快照会话，把 overlay 的 CSS 坐标换算成虚拟桌面逻辑坐标后裁剪。
    //    缩放比 = 截图像素 / overlay CSS 视口；视口由前端上报
    //    （缩放显示器上唯一可信的来源），非法时退回 1:1。
    let monitor_index = overlay_index(window.label())
        .ok_or_else(|| AppError::InvalidInput("not an OCR selection window".to_string()))?;
    let session = {
        let mut guard = state
            .ocr
            .lock()
            .map_err(|_| AppError::Internal("ocr session lock poisoned".into()))?;
        guard.take()
    }
    .ok_or_else(|| AppError::InvalidInput("no active OCR capture session".to_string()))?;
    let selection = session
        .desktop
        .selection_from_viewport(monitor_index, rect, viewport)?;
    tracing::debug!(
        monitor = monitor_index,
        x = selection.x,
        y = selection.y,
        width = selection.width,
        height = selection.height,
        "cropping ocr selection"
    );

    popup::show(app, PopupState::Loading { source_text: None }).await?;
    let capture = tauri::async_runtime::spawn_blocking(move || session.desktop.crop(&selection))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))??;

    // 保留选区截图供「用 Vision 重新识别」手动重试：
    // tesseract 常见故障是高置信度的错，自动兜底触发不了。
    if let Ok(mut retry) = state.ocr_retry.lock() {
        *retry = Some(capture.clone());
    }

    // 3. OCR（普通引擎 → 空结果 / 低置信度时 Vision LLM 兜底）。
    let (ocr, ocr_source) =
        recognize_with_vision_fallback(&state.database.clone(), &capture).await?;
    remember_ocr_source(&state, ocr_source);
    if ocr.is_empty() {
        return Err(AppError::Ocr(
            "no text recognized in the selected area; try a larger selection or enable a \
             vision-capable provider in Settings"
                .to_string(),
        ));
    }
    tracing::debug!(
        blocks = ocr.blocks.len(),
        chars = ocr.text.chars().count(),
        "ocr completed"
    );

    translate_and_show(app, &state.database.clone(), ocr.text, ocr.language).await
}

/// 对识别出的文本走翻译（Router + Engine，流式填充）并切到结果态。
/// OCR pipeline 与手动 Vision 重试共用；`language_hint` 为识别端
/// 报告的语种（翻译端未检出时兜底）。
async fn translate_and_show(
    app: &AppHandle,
    database: &lingua_database::Database,
    source_text: String,
    language_hint: Option<String>,
) -> AppResult<()> {
    // 识别文本先行亮出，译文流式填充。Keychain 授权面板可能被用户
    // 搁置（面板会一直等），不能挡弹窗——与划词管线同一策略。
    popup::show(
        app,
        PopupState::Loading {
            source_text: Some(source_text.clone()),
        },
    )
    .await?;

    // 先查词典（不碰 Keychain：命中即出结果，授权面板被搁置也拖不慢词典）。
    let target_language = pipeline::target_language(database)?;
    let request = lingua_core::translation::TranslationRequest::new(
        source_text.clone(),
        target_language.clone(),
    );
    let dictionary_probe = TranslationEngine::without_provider().with_dictionary(Some(Arc::new(
        lingua_dictionary::SqliteDictionaryProvider::new(database.clone()),
    )));
    let dictionary_hit = dictionary_probe.lookup_dictionary(&request).await;

    // 词典未命中才组装 Router（2s 超时：keychain 面板被搁置时降级）。
    let mut result = if let Some(hit) = dictionary_hit {
        hit
    } else {
        let database_for_router = database.clone();
        let engine = match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tauri::async_runtime::spawn_blocking(move || {
                pipeline::build_router(&database_for_router)
                    .and_then(|router| router.provider_for(lingua_ai::AiTask::Translate))
            }),
        )
        .await
        {
            Err(_) => {
                tracing::warn!("router build timed out (keychain prompt pending?)");
                TranslationEngine::without_provider()
            }
            Ok(Err(error)) => {
                tracing::warn!(error = %error, "router task failed");
                TranslationEngine::without_provider()
            }
            Ok(Ok(Err(error))) => {
                tracing::warn!(error = %error, "no usable translate provider");
                TranslationEngine::without_provider()
            }
            Ok(Ok(Ok(ai_provider))) => TranslationEngine::new(ai_provider),
        }
        .with_swap_language(pipeline::swap_language(database))
        .with_prompt_override(pipeline::prompt_override(database));
        let app_for_delta = app.clone();
        engine
            .translate_stream(&request, &mut |text: String| {
                crate::popup::set_streamed(&text);
                if let Err(error) = app_for_delta.emit("popup://delta", text) {
                    tracing::warn!(error = %error, "failed to emit popup delta");
                }
            })
            .await?
    };
    result.source_text = source_text;
    if result.detected_language.is_none() {
        result.detected_language = language_hint.as_deref().map(lingua_core::Language::parse);
    }

    let history_record = pipeline::history_record_from_result(
        &result,
        target_language,
        lingua_core::HistoryKind::Ocr,
    );
    popup::show(app, PopupState::Result(Box::new(result))).await?;
    pipeline::record_history_task(database.clone(), history_record).await;
    Ok(())
}

/// 手动「用 Vision 重新识别」：对保留的选区截图直接走 Vision
/// （跳过普通 OCR），成功后复用翻译链路；失败以 popup 错误态呈现。
pub async fn run_vision_retry(app: AppHandle) {
    if let Err(error) = vision_retry(&app).await {
        tracing::warn!(error = %error, "vision retry failed");
        let popup_state = PopupState::Error {
            kind: error.kind().to_string(),
            message: error.to_string(),
        };
        if let Err(show_error) = popup::show(&app, popup_state).await {
            tracing::error!(error = %show_error, "failed to show vision retry error");
        }
    }
}

async fn vision_retry(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    let capture = {
        let guard = state
            .ocr_retry
            .lock()
            .map_err(|_| AppError::Internal("ocr retry lock poisoned".into()))?;
        guard.clone().ok_or_else(|| {
            AppError::InvalidInput("no captured image to re-recognize".to_string())
        })?
    };

    // Router 先行（Keychain 授权可能重置 WKWebView 透明；见 translate_and_show）。
    let router = pipeline::build_router(&state.database.clone())?;
    let ai_provider = router.provider_for(lingua_ai::AiTask::Vision)?;

    popup::show(app, PopupState::Loading { source_text: None }).await?;
    let vision_engine = format!("vision:{}", ai_provider.id());
    let ocr = lingua_ocr::VisionOcrProvider::new(ai_provider)
        .recognize(&capture)
        .await?;
    if ocr.is_empty() {
        return Err(AppError::Ocr(
            "vision recognized no text in the selection".to_string(),
        ));
    }
    tracing::debug!(chars = ocr.text.chars().count(), "vision retry recognized");
    remember_ocr_source(
        &state,
        crate::state::OcrSourceInfo {
            engine: vision_engine,
            via: "manual".to_string(),
            confidence: None,
            vision_note: None,
        },
    );

    translate_and_show(app, &state.database.clone(), ocr.text, ocr.language).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use lingua_core::{OcrBlock, OcrLine};

    fn result_with(confidences: &[f32]) -> OcrResult {
        let blocks = confidences
            .iter()
            .map(|confidence| OcrBlock {
                text: "word".into(),
                bbox: [0, 0, 10, 10],
                confidence: *confidence,
                lines: vec![OcrLine {
                    text: "word".into(),
                    bbox: [0, 0, 10, 10],
                    confidence: *confidence,
                    words: vec![],
                }],
            })
            .collect();
        let text = if confidences.is_empty() {
            String::new()
        } else {
            "word".into()
        };
        OcrResult {
            text,
            blocks,
            language: None,
        }
    }

    #[test]
    fn vision_fallback_triggers_on_empty_or_low_confidence() {
        assert!(needs_vision_fallback(&OcrResult::default()));
        assert!(needs_vision_fallback(&result_with(&[0.31, 0.52])));
    }

    #[test]
    fn vision_fallback_skips_on_confident_result() {
        assert!(!needs_vision_fallback(&result_with(&[0.95, 0.89])));
        // 有文本但没有块结构（如部分 HTTP 引擎不返回块）：只按非空判定。
        let mut no_blocks = result_with(&[0.9]);
        no_blocks.blocks.clear();
        assert!(!needs_vision_fallback(&no_blocks));
    }
}
