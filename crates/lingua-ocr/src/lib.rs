//! OCR 抽象层。
//!
//! [`OcrProvider`] 的实现负责把 [`OcrImage`](lingua_core::OcrImage)
//! 变成结构化的 [`OcrResult`](lingua_core::OcrResult)。
//!
//! 实现：
//!
//! * [`TesseractOcrProvider`] —— 调本机 `tesseract` CLI（`brew install
//!   tesseract` / `winget install UB-Mannheim.TesseractOCR` 即可），
//!   解析 TSV 输出（含词级 bbox / 置信度）；
//! * [`HttpOcrProvider`] —— 调本地 OCR HTTP 服务（附带的
//!   `scripts/paddleocr_server.py` 用官方 paddleocr 包起一个 sidecar，
//!   即「PaddleOCR 接入」的落地形式；自建服务照契约实现即可）；
//! * [`VisionOcrProvider`]（Phase 6）—— 包装 vision 能力的 AI
//!   Provider 做高级 fallback：普通 OCR 空结果 / 低置信度时由
//!   Router 派发，输出复用 `OcrResult` 契约（bbox 置零、不伪造）。
//!
//! ONNX 内嵌推理（rapidocr-onnxruntime 路线）留到本地模型阶段。

pub mod http;
pub mod provider;
pub mod tesseract;
pub mod vision;

pub use http::HttpOcrProvider;
pub use provider::{OCR_HTTP_DEFAULT_URL, OcrProvider};
pub use tesseract::TesseractOcrProvider;
pub use vision::VisionOcrProvider;
