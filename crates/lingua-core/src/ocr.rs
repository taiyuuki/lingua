//! OCR 领域模型 —— 引擎与 UI 之间的统一契约。
//!
//! 需求约束：`OcrResult` 必须保留 text / bbox / confidence / language /
//! blocks / lines / words 粒度，为未来「OCR → 翻译 → 原位置覆盖显示」
//! 打基础——所以这里不只是一段纯文本。

use serde::{Deserialize, Serialize};

use crate::{AppError, AppResult};

/// 标准 base64 编码（标准库尚无稳定实现，Phase 5 的轻量版本；
/// `lingua-ocr` 与 OCR 会话快照共用）。
pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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

/// 一张待识别的图片（PNG 编码）。
#[derive(Debug, Clone)]
pub struct OcrImage {
    pub png: Vec<u8>,
    /// 像素尺寸（截图为 Retina 时是逻辑尺寸 × scale）。
    pub width: u32,
    pub height: u32,
}

impl OcrImage {
    /// 从 PNG 字节构造，解析 IHDR 取尺寸（不引入图片解码依赖；
    /// 识别端只需要字节，尺寸供日志与预处理使用）。
    pub fn from_png(bytes: Vec<u8>) -> AppResult<Self> {
        const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        if bytes.len() < 24 || bytes[..8] != SIG {
            return Err(AppError::Ocr("invalid PNG data".to_string()));
        }
        // IHDR：8 字节签名 + 4 长度 + "IHDR" + 4 宽 + 4 高（大端）。
        if &bytes[12..16] != b"IHDR" {
            return Err(AppError::Ocr("missing PNG IHDR chunk".to_string()));
        }
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        if width == 0 || height == 0 {
            return Err(AppError::Ocr("PNG has zero dimension".to_string()));
        }
        Ok(Self {
            png: bytes,
            width,
            height,
        })
    }
}

/// 一行文字（词的有序集合）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLine {
    pub text: String,
    /// 行包围盒，图片像素坐标，x/y 为左上角。
    pub bbox: [u32; 4],
    pub confidence: f32,
    #[serde(default)]
    pub words: Vec<String>,
}

/// 一个文本块（通常对应一个视觉区域，由若干行组成）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrBlock {
    pub text: String,
    pub bbox: [u32; 4],
    pub confidence: f32,
    #[serde(default)]
    pub lines: Vec<OcrLine>,
}

/// 一次 OCR 的完整结果。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrResult {
    /// 全部文本（块按阅读顺序以换行连接）。
    pub text: String,
    #[serde(default)]
    pub blocks: Vec<OcrBlock>,
    /// 引擎报告的主要语言（BCP-47），未知为 `None`。
    pub language: Option<String>,
}

impl OcrResult {
    /// 是否识别到了任何非空文本。
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]); // bit depth / color type / ...
        bytes
    }

    #[test]
    fn parses_png_dimensions() {
        let image = OcrImage::from_png(png_header(1920, 1080)).unwrap();
        assert_eq!(image.width, 1920);
        assert_eq!(image.height, 1080);
    }

    #[test]
    fn rejects_invalid_png() {
        assert!(OcrImage::from_png(b"not a png".to_vec()).is_err());
        assert!(OcrImage::from_png(png_header(0, 10)).is_err());
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn result_serializes_blocks_and_lines() {
        let result = OcrResult {
            text: "Hello\nWorld".into(),
            blocks: vec![OcrBlock {
                text: "Hello".into(),
                bbox: [10, 20, 100, 30],
                confidence: 0.97,
                lines: vec![OcrLine {
                    text: "Hello".into(),
                    bbox: [10, 20, 100, 30],
                    confidence: 0.97,
                    words: vec!["Hello".into()],
                }],
            }],
            language: Some("en".into()),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(
            json["blocks"][0]["bbox"],
            serde_json::json!([10, 20, 100, 30])
        );
        assert_eq!(json["blocks"][0]["lines"][0]["words"][0], "Hello");
    }
}
