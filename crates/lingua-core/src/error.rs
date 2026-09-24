//! 结构化错误。
//!
//! 规则：业务代码不允许 `unwrap()` / `expect()` / `panic!`，
//! 一切失败都必须落到 [`AppError`]，由前端负责展示用户可读的信息。

use serde::{Serialize, Serializer};

/// 全局统一错误类型。
///
/// 变体集合与产品需求中的错误分类一一对应（Network / Authentication /
/// Provider / Database / OCR / Permission / Platform / InvalidInput /
/// Timeout）。`#[non_exhaustive]` 保证未来加变体不破坏下游匹配。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AppError {
    #[error("network error: {0}")]
    Network(String),
    #[error("authentication failed: {0}")]
    Authentication(String),
    #[error("provider `{provider}` failed: {message}")]
    Provider { provider: String, message: String },
    #[error("database error: {0}")]
    Database(String),
    #[error("OCR failed: {0}")]
    Ocr(String),
    #[error("permission denied: {0}")]
    Permission(String),
    #[error("platform error: {0}")]
    Platform(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("operation timed out: {0}")]
    Timeout(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    /// 机器可读的错误类别，前端据此做分支处理与展示。
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Network(_) => "network",
            AppError::Authentication(_) => "authentication",
            AppError::Provider { .. } => "provider",
            AppError::Database(_) => "database",
            AppError::Ocr(_) => "ocr",
            AppError::Permission(_) => "permission",
            AppError::Platform(_) => "platform",
            AppError::InvalidInput(_) => "invalid_input",
            AppError::Timeout(_) => "timeout",
            AppError::Internal(_) => "internal",
        }
    }
}

impl AppError {
    /// 序列化成 `{ kind, message }`，跨 IPC 时前端拿到稳定结构。
    fn serialize_app_error<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("kind", self.kind())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.serialize_app_error(serializer)
    }
}

/// 全局 Result 别名。
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn kind_matches_variant() {
        assert_eq!(AppError::Network("offline".into()).kind(), "network");
        assert_eq!(AppError::Timeout("30s".into()).kind(), "timeout");
        assert_eq!(
            AppError::Provider {
                provider: "openai".into(),
                message: "rate limited".into(),
            }
            .kind(),
            "provider"
        );
    }

    #[test]
    fn display_is_user_readable() {
        let err = AppError::Permission("accessibility access required".into());
        assert_eq!(
            err.to_string(),
            "permission denied: accessibility access required"
        );
    }

    #[test]
    fn serializes_to_kind_and_message() {
        let err = AppError::Database("locked".into());
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "database");
        assert_eq!(json["message"], "database error: locked");
    }
}
