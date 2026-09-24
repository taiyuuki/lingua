//! 全局应用状态。

use lingua_core::translation::TranslationResult;
use lingua_database::Database;
use lingua_platform::SelectionService;
use std::sync::Mutex;

use lingua_core::OcrImage;

/// Popup 当前展示内容；新建/重建的 popup 窗口挂载后会主动拉一次，
/// 避免错过事件（事件 + 快照双通道）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(
    rename_all = "camelCase",
    tag = "status",
    rename_all_fields = "camelCase"
)]
pub enum PopupState {
    /// 加载中；拿到原文后先行展示（流式翻译的起点）。
    Loading {
        source_text: Option<String>,
    },
    Result(Box<TranslationResult>),
    Error {
        kind: String,
        message: String,
    },
}

/// 一次 OCR 截图会话：整屏快照（内存中，不落盘）+ 逻辑尺寸，
/// 选区坐标（逻辑点）按 `像素 / 逻辑` 缩放比换算后在快照上裁剪。
pub struct OcrSession {
    pub image: OcrImage,
    pub background_data_url: String,
    pub logical_width: f64,
    pub logical_height: f64,
}

/// 通过 `app.manage` 注入、commands 里以 `State<AppState>` 取用。
pub struct AppState {
    pub database: Database,
    pub selection: SelectionService,
    pub popup_state: Mutex<Option<PopupState>>,
    pub ocr: Mutex<Option<OcrSession>>,
}

impl AppState {
    pub fn new(database: Database, selection: SelectionService) -> Self {
        Self {
            database,
            selection,
            popup_state: Mutex::new(None),
            ocr: Mutex::new(None),
        }
    }
}
