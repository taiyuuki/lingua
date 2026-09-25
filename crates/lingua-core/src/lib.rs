//! Lingua 核心领域模型。
//!
//! 这个 crate 只包含纯数据契约（类型 + 序列化），不包含任何 IO、
//! 平台 API 或第三方服务调用。所有上层 crate（platform / ai / database
//! 以及未来的 translation / ocr）都以它为共同语言。
//!
//! 序列化约定：所有会跨越 IPC 边界的类型一律 `camelCase`，
//! 与前端 TypeScript 类型保持一致。

pub mod dictionary;
pub mod error;
pub mod history;
pub mod language;
pub mod lemma;
pub mod ocr;
pub mod selection;
pub mod translation;
pub mod vocabulary;

pub use dictionary::{DictionaryDefinition, DictionaryEntry};
pub use error::{AppError, AppResult};
pub use history::{HistoryItem, HistoryKind, HistoryRecord};
pub use language::Language;
pub use ocr::{OcrBlock, OcrImage, OcrLine, OcrResult, base64_encode};
pub use selection::{Rect, SelectedText, SelectionSource};
pub use translation::{
    TextKind, TranslationMode, TranslationRequest, TranslationResult, classify_text,
};
pub use vocabulary::{Mastery, VocabularyItem};
