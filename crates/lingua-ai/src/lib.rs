//! AI Provider 抽象层。
//!
//! 上层（Translation Engine、Learning 服务）只面向 [`AIProvider`] trait；
//! OpenAI / DeepSeek / Ollama / LM Studio / 自建 API 的差异全部由
//! 各 Provider 实现消化。Phase 0 先固化 trait 与数据契约，
//! OpenAI-compatible HTTP 实现在 Phase 1 落地。

pub mod config;
pub mod openai;
pub mod provider;
pub mod router;
pub mod types;

pub use config::{ProviderConfig, ProviderKind};
pub use openai::OpenAiCompatibleProvider;
pub use provider::AIProvider;
pub use router::{AIRouter, AIRouterBuilder, AiTask};
pub use types::{
    ChatMessage, ChatRequest, ChatResponse, ChatRole, ProviderCapabilities, VisionRequest,
};
