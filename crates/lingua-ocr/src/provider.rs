//! [`OcrProvider`] trait。

use lingua_core::{AppResult, OcrImage, OcrResult};

/// OCR 引擎抽象。实现必须是可并发共享的。
///
/// 失败映射：引擎缺失/启动失败 → `Platform`，识别过程错误 → `Ocr`，
/// HTTP 服务问题 → `Network` / `Timeout`。识别到零文本不是错误，
/// 返回空的 [`OcrResult`]（调用方决定如何提示）。
#[async_trait::async_trait]
pub trait OcrProvider: Send + Sync {
    /// Provider 名称（日志与设置界面用，如 `"tesseract-cli"`）。
    fn name(&self) -> &str;

    async fn recognize(&self, image: &OcrImage) -> AppResult<OcrResult>;
}

/// `HttpJsonProvider` 的默认服务地址（本地 sidecar）。
pub const OCR_HTTP_DEFAULT_URL: &str = "http://127.0.0.1:8433/ocr";
