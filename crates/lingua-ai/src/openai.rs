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
use crate::types::{ChatMessage, ChatRequest, ChatResponse, VisionRequest};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

/// 把 `extra_body`（厂商扩展参数，JSON 对象）逐键合并进请求体；
/// 同名键以扩展参数为准（显式覆盖标准字段）。非对象值忽略。
fn merge_extra_body(body: &mut serde_json::Value, extra: Option<&serde_json::Value>) {
    if let (Some(map), Some(extra)) = (
        body.as_object_mut(),
        extra.and_then(|value| value.as_object()),
    ) {
        for (key, value) in extra {
            map.insert(key.clone(), value.clone());
        }
    }
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
        let response = self.send(request, false).await?;
        let text = response_text(response).await?;
        self.parse_completion(text)
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> AppResult<ChatResponse> {
        let response = self.send(request, true).await?;
        Self::consume_sse(response, self.config.id.clone(), on_delta).await
    }

    /// 视觉理解：OpenAI 多模态消息格式——content 为分段数组，
    /// 图片经 `image_url` 以 data URL（base64）内联。
    async fn vision(&self, request: &VisionRequest) -> AppResult<ChatResponse> {
        let data_url = format!("data:{};base64,{}", request.mime_type, request.image_base64);
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": request.prompt },
                    { "type": "image_url", "image_url": { "url": data_url } },
                ],
            }],
        });
        let mut body = body;
        merge_extra_body(&mut body, self.config.extra_body.as_ref());
        let response = self.send_body(body).await?;
        let text = response_text(response).await?;
        self.parse_completion(text)
    }
}

impl OpenAiCompatibleProvider {
    /// 解析 chat/completions 响应体（chat 与 vision 共用）。
    fn parse_completion(&self, text: String) -> AppResult<ChatResponse> {
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

    /// 构造并发送 chat/completions 请求（chat 与 chat_stream 共用）。
    async fn send(&self, request: &ChatRequest, stream: bool) -> AppResult<reqwest::Response> {
        let model = request
            .model
            .as_deref()
            .or(self.config.model.as_deref())
            .map(str::to_owned);
        let typed = ChatCompletionsBody {
            model: model.as_deref(),
            messages: &request.messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: stream.then_some(true),
        };
        let mut body =
            serde_json::to_value(&typed).map_err(|error| AppError::Internal(error.to_string()))?;
        merge_extra_body(&mut body, self.config.extra_body.as_ref());
        self.send_body(body).await
    }

    /// 发送已构造好的请求体（vision 的多模态消息不走 ChatCompletionsBody）。
    async fn send_body(&self, body: serde_json::Value) -> AppResult<reqwest::Response> {
        let url = self.chat_url()?;
        let mut http_request = self.http.post(url).json(&body);
        if let Some(key) = self.config.api_key.as_deref().filter(|key| !key.is_empty()) {
            http_request = http_request.bearer_auth(key);
        }
        if let Some(org) = self.config.organization.as_deref() {
            http_request = http_request.header("OpenAI-Organization", org);
        }

        let response = http_request.send().await.map_err(map_request_error)?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.map_err(map_request_error)?;
            let message = serde_json::from_str::<ChatCompletionsResponse>(&text)
                .ok()
                .and_then(|parsed| parsed.error)
                .and_then(|error| error.message)
                .unwrap_or_else(|| truncate_for_message(&text));
            return Err(map_status_error(&self.config.id, status.as_u16(), message));
        }
        Ok(response)
    }

    /// 消费 SSE 流：解析 `data: {...}` 事件，把增量内容实时回调给
    /// `on_delta`（回调收到的是**到目前为止的完整文本**，前端可直接
    /// 替换渲染，天然抗丢事件）。
    async fn consume_sse<F: FnMut(String) + Send + ?Sized>(
        response: reqwest::Response,
        provider_id: String,
        on_delta: &mut F,
    ) -> AppResult<ChatResponse> {
        use futures_util::StreamExt;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut content = String::new();
        let mut model: Option<String> = None;
        let mut finish_reason: Option<String> = None;
        let mut delta_count = 0usize;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(map_request_error)?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline) = buffer.find('\n') {
                let line: String = buffer.drain(..=newline).collect();
                let line = line.trim_end();
                let Some(payload) = line.strip_prefix("data: ") else {
                    continue;
                };
                let payload = payload.trim();
                if payload == "[DONE]" {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
                    continue; // 心跳/注释行等
                };
                if let Some(error) = value.get("error") {
                    let message = error
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown provider error");
                    return Err(AppError::Provider {
                        provider: provider_id,
                        message: message.to_string(),
                    });
                }
                if let Some(name) = value.get("model").and_then(serde_json::Value::as_str) {
                    model = Some(name.to_string());
                }
                if let Some(choice) = value
                    .get("choices")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|choices| choices.first())
                {
                    if let Some(delta) = choice
                        .get("delta")
                        .and_then(|delta| delta.get("content"))
                        .and_then(serde_json::Value::as_str)
                    {
                        if !delta.is_empty() {
                            content.push_str(delta);
                            delta_count += 1;
                            on_delta(content.clone());
                        }
                    }
                    if let Some(reason) = choice
                        .get("finish_reason")
                        .and_then(serde_json::Value::as_str)
                    {
                        finish_reason = Some(reason.to_string());
                    }
                }
            }
        }

        tracing::debug!(
            deltas = delta_count,
            chars = content.chars().count(),
            "sse stream consumed"
        );

        Ok(ChatResponse {
            content,
            model,
            finish_reason,
        })
    }
}

/// 把响应体读成文本（传输错误 → Network/Timeout）。
async fn response_text(response: reqwest::Response) -> AppResult<String> {
    response.text().await.map_err(map_request_error)
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
    async fn vision_sends_multimodal_message_and_parses_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        spawn_blocking(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 16384];
            let mut request = String::new();
            loop {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.push_str(&String::from_utf8_lossy(&buffer[..read]));
                if request.contains("image_url") {
                    break;
                }
            }
            assert!(
                request.contains("\"image_url\""),
                "vision request must use image_url content part: {request}"
            );
            assert!(
                request.contains("data:image/png;base64,aGVsbG8="),
                "image must be inlined as a data URL: {request}"
            );
            assert!(
                request.contains("read the text"),
                "prompt must be sent as the text part: {request}"
            );
            let body = r#"{"choices":[{"message":{"content":"hi"},"finish_reason":"stop"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            let _ = stream.flush();
        });

        let mut config = ProviderConfig::new("vision", "Vision");
        config.base_url = Some(format!("http://{addr}/v1"));
        let provider = OpenAiCompatibleProvider::new(config).unwrap();

        let response = provider
            .vision(&VisionRequest {
                image_base64: "aGVsbG8=".into(), // base64("hello")
                mime_type: "image/png".into(),
                prompt: "read the text".into(),
            })
            .await
            .unwrap();
        assert_eq!(response.content, "hi");
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
    fn extra_body_merges_into_request_and_overrides() {
        let mut body = serde_json::json!({
            "model": "glm-4.6",
            "messages": [],
            "max_tokens": 100
        });
        let extra = serde_json::json!({
            "thinking": { "type": "disabled" },
            "max_tokens": 512
        });
        merge_extra_body(&mut body, Some(&extra));
        assert_eq!(body["thinking"], serde_json::json!({ "type": "disabled" }));
        assert_eq!(body["max_tokens"], 512, "扩展键覆盖标准字段");
    }

    #[test]
    fn extra_body_none_or_non_object_is_noop() {
        let mut body = serde_json::json!({ "model": "x" });
        merge_extra_body(&mut body, None);
        assert_eq!(body["model"], "x");

        let not_object = serde_json::json!([1, 2, 3]);
        merge_extra_body(&mut body, Some(&not_object));
        assert_eq!(body["model"], "x");
    }

    #[tokio::test]
    async fn sse_stream_emits_accumulated_deltas() {
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
            assert!(
                request.contains("\"stream\":true"),
                "must request stream: {request}"
            );

            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n\
                data: {\"choices\":[{\"delta\":{\"content\":\"世界\"}}]}\n\n\
                data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                data: [DONE]\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            let _ = stream.flush();
        });

        let mut config = ProviderConfig::new("sse", "SSE");
        config.base_url = Some(format!("http://{addr}/v1"));
        let provider = OpenAiCompatibleProvider::new(config).unwrap();

        let deltas = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let collected = deltas.clone();
        let response = provider
            .chat_stream(&sample_request(), &mut |text: String| {
                collected.lock().unwrap().push(text);
            })
            .await
            .unwrap();

        assert_eq!(response.content, "你好世界");
        let deltas = deltas.lock().unwrap();
        assert_eq!(deltas.len(), 2, "最终 [DONE] 前的两次增量都要回调");
        assert_eq!(deltas[0], "你好");
        assert_eq!(deltas[1], "你好世界");
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
