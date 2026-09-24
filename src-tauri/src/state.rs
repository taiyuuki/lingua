//! 全局应用状态。

use lingua_core::translation::TranslationResult;
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
    Loading,
    Result(TranslationResult),
    Error { kind: String, message: String },
}

/// 通过 `app.manage` 注入、commands 里以 `State<AppState>` 取用。
pub struct AppState {
    pub database: Database,
    pub selection: SelectionService,
    pub popup_state: Mutex<Option<PopupState>>,
}

impl AppState {
    pub fn new(database: Database, selection: SelectionService) -> Self {
        Self {
            database,
            selection,
            popup_state: Mutex::new(None),
        }
    }
}
