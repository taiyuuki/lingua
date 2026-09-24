//! AI Router：按任务挑选 Provider。
//!
//! 需求约束：**不要假设所有 AI 请求都使用同一个模型**。Router 持有
//! Provider 注册表 + 「任务 → Provider」路由表 + 默认 Provider；
//! 未配置路由的任务回退到默认 Provider。
//!
//! 任务种类随阶段扩展（Vision → Phase 6，学习分析 → Phase 9），
//! Phase 2 只有 Translate。

use crate::provider::AIProvider;
use lingua_core::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// AI 任务种类，路由表的 key。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiTask {
    /// 划词翻译 / 文本翻译。
    Translate,
}

/// Provider 注册表 + 路由表。
#[derive(Default)]
pub struct AIRouter {
    providers: HashMap<String, Arc<dyn AIProvider>>,
    routes: HashMap<AiTask, String>,
    default: Option<String>,
}

impl AIRouter {
    pub fn builder() -> AIRouterBuilder {
        AIRouterBuilder::default()
    }

    /// 解析某个任务应使用的 Provider：任务路由 → 默认 Provider。
    pub fn provider_for(&self, task: AiTask) -> AppResult<Arc<dyn AIProvider>> {
        let id = self
            .routes
            .get(&task)
            .or(self.default.as_ref())
            .ok_or_else(|| {
                AppError::InvalidInput(
                    "no AI provider is configured; open Settings to add one".to_string(),
                )
            })?;
        self.providers.get(id).cloned().ok_or_else(|| {
            AppError::InvalidInput(format!(
                "provider `{id}` is routed but not registered; check Settings"
            ))
        })
    }

    /// 已注册的 Provider id 列表（调试与设置界面用）。
    pub fn provider_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.providers.keys().cloned().collect();
        ids.sort();
        ids
    }
}

/// [`AIRouter`] 的构建器。
#[derive(Default)]
pub struct AIRouterBuilder {
    providers: HashMap<String, Arc<dyn AIProvider>>,
    routes: HashMap<AiTask, String>,
    default: Option<String>,
}

impl AIRouterBuilder {
    /// 注册一个新构造的 Provider。
    pub fn register(mut self, provider: impl AIProvider + 'static) -> Self {
        self.providers
            .insert(provider.id().to_string(), Arc::new(provider));
        self
    }

    /// 注册一个已共享的 Provider（Arc）。
    pub fn register_shared(mut self, provider: Arc<dyn AIProvider>) -> Self {
        self.providers.insert(provider.id().to_string(), provider);
        self
    }

    /// 设置任务路由。同一任务重复设置以后者为准。
    pub fn route(mut self, task: AiTask, provider_id: impl Into<String>) -> Self {
        self.routes.insert(task, provider_id.into());
        self
    }

    /// 设置默认 Provider（路由未命中的任务回退到这里）。
    pub fn with_default(mut self, provider_id: impl Into<String>) -> Self {
        self.default = Some(provider_id.into());
        self
    }

    /// 构建 Router。路由指向未注册的 Provider 不在此时报错——
    /// 延迟到 `provider_for` 调用时（配置可能在运行期被修改）。
    pub fn build(self) -> AIRouter {
        AIRouter {
            providers: self.providers,
            routes: self.routes,
            default: self.default,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::types::{ChatMessage, ChatRequest, ChatResponse};
    use lingua_core::AppResult;

    struct NamedProvider(String);

    #[async_trait::async_trait]
    impl AIProvider for NamedProvider {
        fn id(&self) -> &str {
            &self.0
        }

        async fn chat(&self, request: &ChatRequest) -> AppResult<ChatResponse> {
            Ok(ChatResponse {
                content: format!("from {}", self.0),
                model: None,
                finish_reason: request.messages.first().map(|m| format!("{:?}", m.role)),
            })
        }
    }

    #[test]
    fn routes_to_task_provider_then_default() {
        let router = AIRouter::builder()
            .register(NamedProvider("cloud".into()))
            .register(NamedProvider("local".into()))
            .with_default("local")
            .route(AiTask::Translate, "cloud")
            .build();

        assert_eq!(
            router.provider_for(AiTask::Translate).unwrap().id(),
            "cloud"
        );
        assert_eq!(router.provider_ids(), vec!["cloud", "local"]);
    }

    #[test]
    fn falls_back_to_default_when_no_route() {
        let router = AIRouter::builder()
            .register(NamedProvider("local".into()))
            .with_default("local")
            .build();

        assert_eq!(
            router.provider_for(AiTask::Translate).unwrap().id(),
            "local"
        );
    }

    #[test]
    fn errors_when_nothing_configured() {
        let router = AIRouter::builder().build();
        let Err(error) = router.provider_for(AiTask::Translate) else {
            unreachable!("expected unconfigured error");
        };
        assert_eq!(error.kind(), "invalid_input");
    }

    #[test]
    fn route_to_unregistered_provider_errors_at_dispatch() {
        let router = AIRouter::builder()
            .register(NamedProvider("local".into()))
            .with_default("local")
            .route(AiTask::Translate, "ghost")
            .build();
        let Err(error) = router.provider_for(AiTask::Translate) else {
            unreachable!("expected missing-provider error");
        };
        assert!(error.to_string().contains("ghost"));
    }

    #[tokio::test]
    async fn chat_dispatches_through_router() {
        let router = AIRouter::builder()
            .register(NamedProvider("cloud".into()))
            .with_default("cloud")
            .build();

        let request = ChatRequest {
            messages: vec![ChatMessage::user("ping")],
            model: None,
            temperature: None,
            max_tokens: None,
        };
        let response = router
            .provider_for(AiTask::Translate)
            .unwrap()
            .chat(&request)
            .await
            .unwrap();
        assert_eq!(response.content, "from cloud");
    }

    #[test]
    fn task_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(AiTask::Translate).unwrap(),
            "translate"
        );
    }
}
