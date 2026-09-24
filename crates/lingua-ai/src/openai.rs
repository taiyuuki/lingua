//! OpenAI-compatible HTTP Provider（OpenAI / DeepSeek / Ollama / LM Studio / 自建 API）。
//!
//! 通用约定：
//!
//! * `base_url` 必须由配置提供（业务代码不写死任何官方地址），
//!   请求路径为 `{base_url}/chat/completions`；
//! * 非流式调用；流式输出在 Phase 2 加入；
//! * 错误映射：连接/请求失败 → `Network`，超时 → `Timeout`，
//!   401/403 → `Authentication`，其余 4xx/5xx 与协议解析失败 → `Provider`。

use crate::config::ProviderConfig;
use crate::types::{ChatMessage, ChatRequest, ChatResponse};
use lingua_core::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug)]
pub struct OpenAiCompatibleProvider {
    config: ProviderConfig,
    http: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    /// 构造 Provider。`base_url` 缺失是配置错误，立即报错。
    pub fn new(config: ProviderConfig) -> AppResult<Self> {
        if config.base_url.as_deref().is_none_or(str::is_empty) {
            return Err(AppError::InvalidInput(format!(
                "provider `{}` is missing `baseUrl`",
                config.id
            )));
        }
        let http = reqwest::Client::builder()
            .timeout(config.timeout())
            .build()
            .map_err(|error| AppError::Network(error.to_string()))?;
        Ok(Self { config, http })
    }

    /// 请求超时时间（暴露给测试与上层诊断）。
    pub fn timeout(&self) -> Duration {
        self.config.timeout()
    }

    fn endpoint(&self, path: &str) -> AppResult<String> {
        let base = self
            .config
            .base_url
            .as_deref()
            .ok_or_else(|| AppError::InvalidInput("baseUrl is required".to_string()))?
            .trim_end_matches('/');
        Ok(format!("{base}{path}"))
    }

    fn chat_url(&self) -> AppResult<String> {
        self.endpoint("/chat/completions")
    }
}

/// 请求体。注意 OpenAI 协议字段是 snake_case（`max_tokens`），
/// 不能复用内部 camelCase 的 ChatRequest。
#[derive(Serialize)]
struct ChatCompletionsBody<'a> {
    model: Option<&'a str>,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// 厂商扩展：`{"type": "disabled"}` 关闭深度思考（智谱 GLM 等）。
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingControl>,
}

#[derive(Serialize)]
struct ThinkingControl {
    r#type: &'static str,
}

/// 响应体：字段做宽容解析（部分兼容服务会省略字段）。
#[derive(Deserialize)]
struct ChatCompletionsResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    error: Option<ApiErrorBody>,
}

#[derive(Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: Option<ChatChoiceMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    #[serde(default)]
    message: Option<String>,
}

#[async_trait::async_trait]
impl crate::provider::AIProvider for OpenAiCompatibleProvider {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn capabilities(&self) -> crate::types::ProviderCapabilities {
        self.config.capabilities
    }

    async fn chat(&self, request: &ChatRequest) -> AppResult<ChatResponse> {
        let url = self.chat_url()?;
        let model = request
            .model
            .as_deref()
            .or(self.config.model.as_deref())
            .map(str::to_owned);
        let body = ChatCompletionsBody {
            model: model.as_deref(),
            messages: &request.messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            thinking: if self.config.disable_thinking {
                Some(ThinkingControl { r#type: "disabled" })
            } else {
                None
            },
        };

        let mut http_request = self.http.post(url).json(&body);
        if let Some(key) = self.config.api_key.as_deref().filter(|key| !key.is_empty()) {
            http_request = http_request.bearer_auth(key);
        }
        if let Some(org) = self.config.organization.as_deref() {
            http_request = http_request.header("OpenAI-Organization", org);
        }

        let response = http_request.send().await.map_err(map_request_error)?;
        let status = response.status();
        let text = response.text().await.map_err(map_request_error)?;

        if !status.is_success() {
            let message = serde_json::from_str::<ChatCompletionsResponse>(&text)
                .ok()
                .and_then(|parsed| parsed.error)
                .and_then(|error| error.message)
                .unwrap_or_else(|| truncate_for_message(&text));
            return Err(map_status_error(&self.config.id, status.as_u16(), message));
        }

        let parsed: ChatCompletionsResponse =
            serde_json::from_str(&text).map_err(|error| AppError::Provider {
                provider: self.config.id.clone(),
                message: format!("unexpected response format: {error}"),
            })?;

        if let Some(error) = parsed.error {
            return Err(AppError::Provider {
                provider: self.config.id.clone(),
                message: error
                    .message
                    .unwrap_or_else(|| "unknown provider error".into()),
            });
        }

        let choice = parsed.choices.first().ok_or_else(|| AppError::Provider {
            provider: self.config.id.clone(),
            message: "response contains no choices".into(),
        })?;
        let content = choice
            .message
            .as_ref()
            .and_then(|message| message.content.clone())
            .unwrap_or_default();

        Ok(ChatResponse {
            content,
            model: parsed.model,
            finish_reason: choice.finish_reason.clone(),
        })
    }
}

fn map_request_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        AppError::Timeout(error.to_string())
    } else {
        // 连接失败、请求构造失败与其它传输层错误都归为网络问题。
        AppError::Network(error.to_string())
    }
}

fn map_status_error(provider: &str, status: u16, message: String) -> AppError {
    match status {
        401 | 403 | 407 => AppError::Authentication(message),
        408 => AppError::Timeout(format!(
            "upstream request timeout (HTTP {status}): {message}"
        )),
        _ => AppError::Provider {
            provider: provider.to_string(),
            message: format!("HTTP {status}: {message}"),
        },
    }
}

fn truncate_for_message(text: &str) -> String {
    const LIMIT: usize = 300;
    if text.chars().count() <= LIMIT {
        text.to_string()
    } else {
        let cut: String = text.chars().take(LIMIT).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::provider::AIProvider;
    use crate::types::ChatRole;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use tokio::task::spawn_blocking;

    /// 极简 HTTP mock：单次响应，断言请求里带上了 Authorization 与路径。
    fn spawn_mock(
        status_line: &'static str,
        body: &'static str,
        expect_auth: Option<&'static str>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        spawn_blocking(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 8192];
            let mut request = String::new();
            loop {
                let read = stream.read(&mut buffer).unwrap();
                request.push_str(&String::from_utf8_lossy(&buffer[..read]));
                if request.contains("\r\n\r\n") {
                    break;
                }
            }
            if let Some(auth) = expect_auth {
                assert!(
                    request
                        .to_lowercase()
                        .contains(&format!("authorization: bearer {auth}")),
                    "expected auth header, got: {request}"
                );
            }
            assert!(
                request.contains("POST /v1/chat/completions"),
                "got: {request}"
            );
            let response = format!(
                "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            let _ = stream.flush();
        });
        format!("http://{addr}/v1")
    }

    fn provider(base_url: String, api_key: Option<&str>) -> OpenAiCompatibleProvider {
        let mut config = ProviderConfig::new("mock", "Mock");
        config.base_url = Some(base_url);
        config.timeout_secs = 2;
        config.api_key = api_key.map(str::to_string);
        OpenAiCompatibleProvider::new(config).unwrap()
    }

    fn sample_request() -> ChatRequest {
        ChatRequest::simple("be brief", "hello")
    }

    #[tokio::test]
    async fn parses_successful_completion() {
        let url = spawn_mock(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"content":"你好"},"finish_reason":"stop"}],"model":"m1"}"#,
            Some("sk-test"),
        );
        let response = provider(url, Some("sk-test"))
            .chat(&sample_request())
            .await
            .unwrap();
        assert_eq!(response.content, "你好");
        assert_eq!(response.model.as_deref(), Some("m1"));
        assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test]
    async fn unauthorized_maps_to_authentication() {
        let url = spawn_mock(
            "HTTP/1.1 401 Unauthorized",
            r#"{"error":{"message":"bad key"}}"#,
            None,
        );
        let error = provider(url, None)
            .chat(&sample_request())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "authentication");
    }

    #[tokio::test]
    async fn server_error_maps_to_provider() {
        let url = spawn_mock(
            "HTTP/1.1 502 Bad Gateway",
            r#"{"error":{"message":"upstream"}}"#,
            None,
        );
        let error = provider(url, None)
            .chat(&sample_request())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "provider");
        assert!(error.to_string().contains("502"));
    }

    #[tokio::test]
    async fn missing_base_url_is_config_error() {
        let mut config = ProviderConfig::new("broken", "Broken");
        config.base_url = None;
        let error = OpenAiCompatibleProvider::new(config).unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
    }

    #[tokio::test]
    async fn silent_server_maps_to_timeout() {
        // 环境无关的超时测试：对端接受连接但永不响应，
        // 客户端 1s 超时后应映射为 Timeout（连接拒绝行为因环境而异，不可依赖）。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        spawn_blocking(move || {
            let (stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(3));
            drop(stream);
        });

        let mut config = ProviderConfig::new("slow", "Slow");
        config.base_url = Some(format!("http://{addr}/v1"));
        config.timeout_secs = 1;
        let error = OpenAiCompatibleProvider::new(config)
            .unwrap()
            .chat(&sample_request())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "timeout");
    }

    #[test]
    fn thinking_control_serialized_only_when_enabled() {
        let mut config = ProviderConfig::new("glm", "GLM");
        config.base_url = Some("https://open.bigmodel.cn/api/paas/v4".into());
        config.disable_thinking = true;
        let provider = OpenAiCompatibleProvider::new(config).unwrap();
        let body = ChatCompletionsBody {
            model: None,
            messages: &[],
            temperature: None,
            max_tokens: None,
            thinking: if provider.config.disable_thinking {
                Some(ThinkingControl { r#type: "disabled" })
            } else {
                None
            },
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["thinking"], serde_json::json!({ "type": "disabled" }));

        let mut plain = ProviderConfig::new("openai", "OpenAI");
        plain.base_url = Some("https://api.openai.com/v1".into());
        let provider = OpenAiCompatibleProvider::new(plain).unwrap();
        let body = ChatCompletionsBody {
            model: None,
            messages: &[],
            temperature: None,
            max_tokens: None,
            thinking: if provider.config.disable_thinking {
                Some(ThinkingControl { r#type: "disabled" })
            } else {
                None
            },
        };
        let json = serde_json::to_value(&body).unwrap();
        assert!(
            json.get("thinking").is_none(),
            "未开启时不应发送 thinking 字段"
        );
    }

    #[test]
    fn role_serializes_to_openai_names() {
        assert_eq!(serde_json::to_value(ChatRole::System).unwrap(), "system");
        assert_eq!(
            serde_json::to_value(ChatRole::Assistant).unwrap(),
            "assistant"
        );
    }
}
