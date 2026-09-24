//! Vision LLM 作为 OCR 的高级 fallback（Phase 6）。
//!
//! 定位：**不替代普通 OCR**。Tesseract / PaddleOCR 结果为空或置信度
//! 偏低时（游戏 UI、艺术字、低分辨率截图等场景），由上层 Router 派给
//! 具备 vision 能力的 AI Provider 重新识别。实现为 [`OcrProvider`]，
//! 与 Tesseract / HTTP 实现同构，pipeline 无需分支。
//!
//! 输出契约：prompt 约定严格 JSON（`{"text", "language"}`）；解析失败
//! 时整段输出当纯文本（与翻译引擎的优雅降级一致）。bbox 置零、
//! confidence 置 0——Vision LLM 不产生可靠的几何信息，结构化粒度
//! 保留给「原位置覆盖」等未来功能，不伪造数据。

use crate::provider::OcrProvider;
use lingua_ai::provider::AIProvider;
use lingua_ai::types::VisionRequest;
use lingua_core::ocr::base64_encode;
use lingua_core::{AppResult, OcrBlock, OcrImage, OcrLine, OcrResult};
use std::sync::Arc;

/// 识别提示词。要求严格 JSON；`language` 供翻译端做语种兜底。
const VISION_OCR_PROMPT: &str = "You are an OCR engine. Transcribe ALL visible text in the \
     image exactly as written, preserving reading order and line breaks. Do not translate, \
     do not add commentary.\n\
     Respond with ONLY a JSON object (no markdown fences):\n\
     {\"text\": \"<all visible text, lines separated by \\n>\", \"language\": \"<BCP-47 code \
     of the dominant language, or null>\"}\n\
     If the image contains no readable text, respond {\"text\": \"\", \"language\": null}.";

/// 包装一个 vision 能力的 [`AIProvider`]，暴露为 [`OcrProvider`]。
pub struct VisionOcrProvider {
    provider: Arc<dyn AIProvider>,
}

impl VisionOcrProvider {
    pub fn new(provider: Arc<dyn AIProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl OcrProvider for VisionOcrProvider {
    fn name(&self) -> &str {
        "vision-llm"
    }

    async fn recognize(&self, image: &OcrImage) -> AppResult<OcrResult> {
        let request = VisionRequest {
            image_base64: base64_encode(&image.png),
            mime_type: "image/png".to_string(),
            prompt: VISION_OCR_PROMPT.to_string(),
        };
        let response = self.provider.vision(&request).await?;
        Ok(parse_vision_output(&response.content))
    }
}

/// 模型输出的 JSON 形状。
#[derive(serde::Deserialize)]
struct VisionOcrOutput {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: Option<String>,
}

/// 解析模型输出：严格 JSON 优先；解析失败或 JSON 里 text 为空时，
/// 整段输出按纯文本处理（模型最常见的偏差是包了 ```json 围栏或
/// 附带说明文字，围栏已先行剥掉）。
fn parse_vision_output(raw: &str) -> OcrResult {
    let trimmed = raw.trim();
    if let Ok(parsed) = serde_json::from_str::<VisionOcrOutput>(strip_code_fence(trimmed))
        && !parsed.text.trim().is_empty()
    {
        return lines_to_result(&parsed.text, parsed.language);
    }
    lines_to_result(trimmed, None)
}

/// 剥掉 ``` / ```json 围栏；不是围栏包裹则原样返回。
fn strip_code_fence(text: &str) -> &str {
    let Some(without_open) = text.strip_prefix("```") else {
        return text;
    };
    // 开栏行可能是 ```json、```JSON 等，跳过到行尾。
    let body = without_open
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or(without_open);
    body.strip_suffix("```").unwrap_or(body).trim()
}

/// 纯文本 → [`OcrResult`]：每个非空行一个块（bbox 置零、置信度 0）。
fn lines_to_result(text: &str, language: Option<String>) -> OcrResult {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let blocks = lines
        .iter()
        .map(|line| OcrBlock {
            text: (*line).to_string(),
            bbox: [0, 0, 0, 0],
            confidence: 0.0,
            lines: vec![OcrLine {
                text: (*line).to_string(),
                bbox: [0, 0, 0, 0],
                confidence: 0.0,
                words: vec![],
            }],
        })
        .collect();
    OcrResult {
        text: lines.join("\n"),
        blocks,
        language,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use lingua_ai::types::{ChatRequest, ChatResponse};
    use lingua_core::{AppError, OcrImage};

    /// Vision 假 Provider：固定返回 `raw`。
    struct FakeVisionProvider {
        raw: String,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl AIProvider for FakeVisionProvider {
        fn id(&self) -> &str {
            "fake-vision"
        }

        fn capabilities(&self) -> lingua_ai::types::ProviderCapabilities {
            lingua_ai::types::ProviderCapabilities::all()
        }

        async fn chat(&self, _request: &ChatRequest) -> AppResult<ChatResponse> {
            Err(AppError::Provider {
                provider: "fake-vision".into(),
                message: "chat is not used here".into(),
            })
        }

        async fn vision(&self, request: &VisionRequest) -> AppResult<ChatResponse> {
            if self.fail {
                return Err(AppError::Provider {
                    provider: "fake-vision".into(),
                    message: "upstream exploded".into(),
                });
            }
            assert!(!request.prompt.is_empty(), "prompt must be sent");
            assert!(!request.image_base64.is_empty(), "image must be sent");
            Ok(ChatResponse {
                content: self.raw.clone(),
                model: Some("fake-vision-1".into()),
                finish_reason: Some("stop".into()),
            })
        }
    }

    /// 1×1 PNG（合法签名 + IHDR，足够通过 OcrImage 校验）。
    fn tiny_image() -> OcrImage {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        OcrImage::from_png(bytes).unwrap()
    }

    fn provider_with(raw: &str) -> VisionOcrProvider {
        VisionOcrProvider::new(Arc::new(FakeVisionProvider {
            raw: raw.to_string(),
            fail: false,
        }))
    }

    #[tokio::test]
    async fn strict_json_output_maps_to_result() {
        let result = provider_with(r#"{"text": "Hello\n世界", "language": "en"}"#)
            .recognize(&tiny_image())
            .await
            .unwrap();
        assert_eq!(result.text, "Hello\n世界");
        assert_eq!(result.language.as_deref(), Some("en"));
        assert_eq!(result.blocks.len(), 2);
        assert_eq!(result.blocks[0].text, "Hello");
        assert_eq!(result.blocks[0].bbox, [0, 0, 0, 0]);
    }

    #[tokio::test]
    async fn fenced_json_is_unwrapped() {
        let result = provider_with("```json\n{\"text\": \"fenced\", \"language\": null}\n```")
            .recognize(&tiny_image())
            .await
            .unwrap();
        assert_eq!(result.text, "fenced");
        assert_eq!(result.language, None);
    }

    #[tokio::test]
    async fn non_json_output_degrades_to_plain_text() {
        let result = provider_with("Just some\nplain text the model decided to emit")
            .recognize(&tiny_image())
            .await
            .unwrap();
        assert_eq!(
            result.text,
            "Just some\nplain text the model decided to emit"
        );
        assert_eq!(result.blocks.len(), 2);
        assert_eq!(result.language, None);
    }

    #[tokio::test]
    async fn empty_json_text_degrades_to_plain_text() {
        // JSON 合法但 text 为空：整段输出（就是那段 JSON）按纯文本处理，
        // 上层 is_empty() 会触发原始 OCR 结果兜底。
        let result = provider_with(r#"{"text": "", "language": null}"#)
            .recognize(&tiny_image())
            .await
            .unwrap();
        assert!(!result.is_empty(), "降级后应保留原始输出供上层判定");
    }

    #[tokio::test]
    async fn provider_errors_propagate() {
        let error = VisionOcrProvider::new(Arc::new(FakeVisionProvider {
            raw: String::new(),
            fail: true,
        }))
        .recognize(&tiny_image())
        .await
        .unwrap_err();
        assert_eq!(error.kind(), "provider");
    }

    #[test]
    fn strip_code_fence_handles_plain_and_fenced() {
        assert_eq!(strip_code_fence("plain"), "plain");
        assert_eq!(strip_code_fence("```json\n{}\n```"), "{}");
        assert_eq!(strip_code_fence("```\n{}\n```"), "{}");
        // 没有闭栏时原样返回开头之后的文本，不 panic。
        assert_eq!(strip_code_fence("```json\n{}"), "{}");
    }
}
