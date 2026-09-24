//! Translation Engine。
//!
//! 职责（按需求）：翻译不是一个「直接调 OpenAI」的函数，而是按请求
//! 决定走 Dictionary / 本地翻译 / AI / Vision 的路由器。Phase 1 只有
//! AI 路由可用，但路由边界、请求契约与 Prompt 模板先按最终形态建立。

pub mod engine;
pub mod template;

pub use engine::TranslationEngine;
pub use template::PromptTemplate;
