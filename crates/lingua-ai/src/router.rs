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
    /// 视觉识别（截图 OCR 的高级 fallback，只派给 vision 能力 Provider）。
    Vision,
    /// 学习分析（Phase 9：单词深析 / 句子解构，输出结构化学习卡片）。
    Study,
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
        match task {
            // Translate / Study 都是普通文本任务：路由 → 默认。
            AiTask::Translate | AiTask::Study => self.provider_by_route(task),
            // Vision 按能力分派：路由/默认指向的 Provider 不具备
            // vision 能力时自动改派（见 vision_provider）。
            AiTask::Vision => self.vision_provider(),
        }
    }

    fn provider_by_route(&self, task: AiTask) -> AppResult<Arc<dyn AIProvider>> {
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

    /// Vision 分派：任务路由 / 默认 Provider 优先（须具备 vision 能力，
    /// 不具备则告警跳过）；否则在注册表里自动挑选第一个具备 vision
    /// 能力的 Provider；一个都没有才报「未配置」。
    fn vision_provider(&self) -> AppResult<Arc<dyn AIProvider>> {
        if let Ok(candidate) = self.provider_by_route(AiTask::Vision) {
            if candidate.capabilities().vision {
                return Ok(candidate);
            }
            tracing::warn!(
                provider = candidate.id(),
                "routed/default provider lacks vision capability; scanning registered providers"
            );
        }
        let mut ids: Vec<&String> = self.providers.keys().collect();
        ids.sort();
        for id in ids {
            if let Some(provider) = self.providers.get(id)
                && provider.capabilities().vision
            {
                return Ok(provider.clone());
            }
        }
        Err(AppError::InvalidInput(
            "no vision-capable provider is configured; add one in Settings and enable its \
             vision capability"
                .to_string(),
        ))
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

    struct NamedProvider(String, Option<crate::types::ProviderCapabilities>);

    #[async_trait::async_trait]
    impl AIProvider for NamedProvider {
        fn id(&self) -> &str {
            &self.0
        }

        fn capabilities(&self) -> crate::types::ProviderCapabilities {
            self.1.unwrap_or_default()
        }

        async fn chat(&self, request: &ChatRequest) -> AppResult<ChatResponse> {
            Ok(ChatResponse {
                content: format!("from {}", self.0),
                model: None,
                finish_reason: request.messages.first().map(|m| format!("{:?}", m.role)),
            })
        }
    }

    /// 不带能力参数的 Provider（trait 默认能力：chat 开、vision 关）。
    fn named(id: &str) -> NamedProvider {
        NamedProvider(id.into(), None)
    }

    fn vision_only(id: &str) -> NamedProvider {
        NamedProvider(
            id.into(),
            Some(crate::types::ProviderCapabilities {
                chat: false,
                translate: false,
                vision: true,
            }),
        )
    }

    #[test]
    fn routes_to_task_provider_then_default() {
        let router = AIRouter::builder()
            .register(named("cloud"))
            .register(named("local"))
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
            .register(named("local"))
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
            .register(named("local"))
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
            .register(named("cloud"))
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
        assert_eq!(serde_json::to_value(AiTask::Vision).unwrap(), "vision");
    }

    #[test]
    fn vision_route_to_capable_provider_wins() {
        let router = AIRouter::builder()
            .register(vision_only("eye"))
            .register(named("plain"))
            .route(AiTask::Vision, "eye")
            .build();
        assert_eq!(router.provider_for(AiTask::Vision).unwrap().id(), "eye");
    }

    #[test]
    fn vision_route_to_incapable_provider_falls_back_to_scan() {
        let router = AIRouter::builder()
            .register(named("plain"))
            .register(vision_only("eye"))
            .route(AiTask::Vision, "plain")
            .build();
        // 路由指向的 Provider 不具备视觉能力：自动改派，而不是报错。
        assert_eq!(router.provider_for(AiTask::Vision).unwrap().id(), "eye");
    }

    #[test]
    fn vision_auto_picks_capable_provider_without_route() {
        let router = AIRouter::builder()
            .register(named("plain"))
            .register(vision_only("eye"))
            .with_default("plain")
            .build();
        assert_eq!(router.provider_for(AiTask::Vision).unwrap().id(), "eye");
    }

    #[test]
    fn vision_errors_when_no_provider_is_capable() {
        let router = AIRouter::builder().register(named("plain")).build();
        let Err(error) = router.provider_for(AiTask::Vision) else {
            unreachable!("expected unconfigured vision error");
        };
        assert_eq!(error.kind(), "invalid_input");
        assert!(error.to_string().contains("vision"));
    }
}
