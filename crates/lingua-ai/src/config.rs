//! Provider 配置模型。
//!
//! 持久化形态（Phase 2）：配置（不含 api_key）存 SQLite `providers` 表，
//! api_key 存 OS 密钥库（macOS Keychain / Windows Credential Manager /
//! Linux Secret Service）。**任何明文密钥都不允许落库**，
//! 因此这里 `api_key` 带 `#[serde(skip)]`，只存在于运行时内存中。

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Provider 协议族。本地服务（Ollama / LM Studio / llama.cpp server）
/// 兼容 OpenAI 协议，通过 `ProviderKind::OpenAiCompatible` + 自定义
/// `base_url` 接入，不需要单独的 Kind。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// 任何实现了 OpenAI `/chat/completions` 协议的服务（含本地）。
    #[default]
    OpenAiCompatible,
    /// OpenAI 官方 API（当前与 compatible 共用实现，保留枚举便于未来分化）。
    OpenAi,
}

/// 一个可配置的 AI 服务实例。
///
/// 用户可以添加任意多个（OpenAI、DeepSeek、本地 Ollama……），
/// `id` 是它们的稳定标识。`base_url` 必须可配置 ——
/// 业务代码中禁止写死 `https://api.openai.com`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: ProviderKind,
    /// 例如 `https://api.openai.com/v1`、`http://localhost:11434/v1`。
    pub base_url: Option<String>,
    /// 默认模型；Router 可按任务覆盖。
    pub model: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    /// 请求超时秒数，默认 30s。
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub capabilities: crate::types::ProviderCapabilities,
    /// 运行时密钥；**不序列化**。Phase 2 起由 OS 密钥库读写。
    #[serde(skip)]
    pub api_key: Option<String>,
}

fn default_timeout_secs() -> u64 {
    30
}

impl ProviderConfig {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind: ProviderKind::default(),
            base_url: None,
            model: None,
            organization: None,
            timeout_secs: default_timeout_secs(),
            capabilities: crate::types::ProviderCapabilities::default(),
            api_key: None,
        }
    }

    /// 超时时间。
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn api_key_never_appears_in_serialized_output() {
        let mut config = ProviderConfig::new("deepseek", "DeepSeek");
        config.base_url = Some("https://api.deepseek.com/v1".into());
        config.model = Some("deepseek-chat".into());
        config.api_key = Some("sk-super-secret".into());

        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("sk-super-secret"), "api_key leaked: {json}");
        assert!(!json.contains("api_key"));

        let back: ProviderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "deepseek");
        assert_eq!(back.api_key, None);
    }

    #[test]
    fn defaults_are_sane() {
        let config: ProviderConfig = serde_json::from_str(
            r#"{ "id": "local-ollama", "name": "Ollama", "baseUrl": "http://localhost:11434/v1" }"#,
        )
        .unwrap();
        assert_eq!(config.kind, ProviderKind::OpenAiCompatible);
        assert_eq!(config.timeout(), Duration::from_secs(30));
        assert!(config.capabilities.chat);
        assert!(!config.capabilities.vision);
    }
}
