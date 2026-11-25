//! 全局应用状态。

use lingua_core::{OcrImage, translation::TranslationResult};
use lingua_database::Database;
use lingua_platform::SelectionService;
use std::sync::Mutex;

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

/// 一次 OCR 截图会话：**所有**显示器的快照（内存中，不落盘）。
/// 每块屏铺一个 overlay 窗口（`ocr-selection-{下标}`），选区统一用虚拟桌面
/// 逻辑坐标在窗口间同步，跨屏拖拽由此成立。
/// 裁剪缩放比的首选基准是前端上报的 overlay CSS 视口（HiDPI 自校准）。
pub struct OcrSession {
    /// 各显示器快照；下标即 overlay 窗口序号。
    pub desktop: lingua_platform::DesktopCapture,
    /// 各显示器快照的 data URL（overlay 背景），与 `desktop.monitors` 同序。
    pub backgrounds: Vec<String>,
}

/// 一次截图翻译的识别来源（Popup 展示用）：最终产出文本的引擎、
/// 走的路径（primary 直出 / auto Vision 自动兜底 / manual 手动重识）、
/// 块平均置信度，以及 Vision 兜底尝试过但回退原始结果的原因。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrSourceInfo {
    /// 如 `tesseract-cli` / `http` / `vision:<provider-id>`。
    pub engine: String,
    pub via: String,
    /// 0..1；Vision 无几何数据时 `None`。
    pub confidence: Option<f32>,
    pub vision_note: Option<String>,
}

/// 通过 `app.manage` 注入、commands 里以 `State<AppState>` 取用。
pub struct AppState {
    pub database: Database,
    pub selection: SelectionService,
    pub popup_state: Mutex<Option<PopupState>>,
    pub ocr: Mutex<Option<OcrSession>>,
    /// OCR 翻译保留的选区截图，供「用 Vision 重新识别」手动重试
    /// （tesseract 常见故障是高置信度的错，自动兜底触发不了）；
    /// 新的 OCR 会话替换、划词翻译触发时清除。
    pub ocr_retry: Mutex<Option<OcrImage>>,
    /// 最近一次截图翻译的识别来源；划词翻译触发时清除。
    pub ocr_source: Mutex<Option<OcrSourceInfo>>,
    /// 发音库 .mdd 的随机读取句柄（id → 已解析句柄）。
    /// setup 阶段按库列表预载，导入/删除命令同步增删；资源协议
    /// 处理器只读缓存，不碰数据库。
    pub audio_files: Mutex<
        std::collections::HashMap<
            String,
            std::sync::Arc<lingua_dictionary::audio::MddResourceFile>,
        >,
    >,
}

impl AppState {
    pub fn new(database: Database, selection: SelectionService) -> Self {
        Self {
            database,
            selection,
            popup_state: Mutex::new(None),
            ocr: Mutex::new(None),
            ocr_retry: Mutex::new(None),
            ocr_source: Mutex::new(None),
            audio_files: Mutex::new(std::collections::HashMap::new()),
        }
    }
}
