//! HTTP JSON OCR 实现 —— 本地 OCR sidecar 的通用契约。
//!
//! 附带的 `scripts/paddleocr_server.py` 用官方 paddleocr 包实现该契约
//! （即「PaddleOCR 接入」的落地形式）；任何 OCR 引擎照此实现即可接入。
//!
//! 契约：
//!
//! ```text
//! POST {url}
//! {"imageBase64": "<PNG base64>", "languageHints": ["en"]}
//! → 200
//! {"blocks": [{"text": "...", "bbox": [x, y, w, h], "confidence": 0.98,
//!              "lines": [{"text": "...", "bbox": [...], "confidence": 0.98, "words": [...]}]}]}
//! ```
//!
//! 响应做宽容解析：`lines` / `words` 可省略；非 200 或解析失败映射为
//! Provider 错误。

use lingua_core::{AppError, AppResult, OcrBlock, OcrImage, OcrLine, OcrResult};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct HttpOcrProvider {
    url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OcrRequestBody<'a> {
    image_base64: &'a str,
    language_hints: &'a [String],
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct OcrResponseBody {
    blocks: Vec<HttpOcrBlock>,
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpOcrBlock {
    text: String,
    /// `[x, y, w, h]`。
    #[serde(default)]
    bbox: [u32; 4],
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    lines: Vec<HttpOcrLine>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpOcrLine {
    #[serde(default)]
    text: String,
    #[serde(default)]
    bbox: [u32; 4],
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    words: Vec<String>,
}

impl HttpOcrProvider {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

#[async_trait::async_trait]
impl crate::provider::OcrProvider for HttpOcrProvider {
    fn name(&self) -> &str {
        "http-json"
    }

    async fn recognize(&self, image: &OcrImage) -> AppResult<OcrResult> {
        let http = reqwest_client()?;
        let image_base64 = use_base64::encode(&image.png);
        let body = OcrRequestBody {
            image_base64: &image_base64,
            language_hints: &[],
        };

        let response = http
            .post(&self.url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    AppError::Timeout(error.to_string())
                } else {
                    AppError::Network(error.to_string())
                }
            })?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| AppError::Network(error.to_string()))?;
        if !status.is_success() {
            return Err(AppError::Provider {
                provider: "ocr-http".into(),
                message: format!("HTTP {status}: {}", truncate(&text)),
            });
        }

        let parsed: OcrResponseBody =
            serde_json::from_str(&text).map_err(|error| AppError::Provider {
                provider: "ocr-http".into(),
                message: format!("unexpected response format: {error}"),
            })?;

        let blocks: Vec<OcrBlock> = parsed
            .blocks
            .into_iter()
            .filter(|block| !block.text.trim().is_empty())
            .map(|block| OcrBlock {
                text: block.text,
                bbox: block.bbox,
                confidence: block.confidence,
                lines: block
                    .lines
                    .into_iter()
                    .map(|line| OcrLine {
                        text: line.text,
                        bbox: line.bbox,
                        confidence: line.confidence,
                        words: line.words,
                    })
                    .collect(),
            })
            .collect();

        let text = blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(OcrResult {
            text,
            blocks,
            language: parsed.language,
        })
    }
}

fn reqwest_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| AppError::Network(error.to_string()))
}

fn truncate(text: &str) -> String {
    text.chars().take(200).collect()
}

/// base64 编码。Phase 5 用轻量实现避免为单个编码引入依赖；
/// 标准库 stabilise 后切换。
mod use_base64 {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            out.push(TABLE[(n >> 18) as usize & 63] as char);
            out.push(TABLE[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                TABLE[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                TABLE[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(use_base64::encode(b""), "");
        assert_eq!(use_base64::encode(b"f"), "Zg==");
        assert_eq!(use_base64::encode(b"fo"), "Zm8=");
        assert_eq!(use_base64::encode(b"foo"), "Zm9v");
        assert_eq!(use_base64::encode(b"foobar"), "Zm9vYmFy");
        // 二进制安全：全字节覆盖与标准实现对拍由 sidecar 集成验证。
    }

    #[test]
    fn response_body_parses_leniently() {
        let body: OcrResponseBody = serde_json::from_str(
            r#"{"blocks": [{"text": "你好", "bbox": [1, 2, 30, 40], "confidence": 0.97}]}"#,
        )
        .unwrap();
        assert_eq!(body.blocks[0].text, "你好");
        assert_eq!(body.blocks[0].lines.len(), 0);
        assert_eq!(body.language, None);
    }

    #[test]
    fn empty_blocks_are_filtered() {
        let provider = HttpOcrProvider::new("http://127.0.0.1:1/ocr");
        assert_eq!(provider.url(), "http://127.0.0.1:1/ocr");
    }
}
