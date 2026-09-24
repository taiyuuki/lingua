//! 全局应用状态。

use lingua_database::Database;

/// 通过 `app.manage` 注入、commands 里以 `State<AppState>` 取用。
pub struct AppState {
    pub database: Database,
}

impl AppState {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
}
