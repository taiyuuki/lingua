//! 划词结果的数据契约。
//!
//! [`SelectionProvider`](lingua_platform) 等平台实现层返回这个统一结构，
//! UI 与业务层不感知具体来源（Accessibility / UI Automation / 剪贴板）。

use crate::language::Language;
use serde::{Deserialize, Serialize};

/// 屏幕上的一个矩形，单位为逻辑像素（CSS px）。
///
/// 来自系统 API 的坐标（如 macOS 屏幕点）在平台层统一换算后填入，
/// 用于把 Popup 定位到选中文本附近。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// 选中文本的来源，用于调试与策略决策（例如剪贴板来源需要恢复剪贴板）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionSource {
    Accessibility,
    Clipboard,
    Unknown,
}

/// 统一的划词结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedText {
    pub text: String,
    pub source: SelectionSource,
    pub language: Option<Language>,
    pub rect: Option<Rect>,
}

impl SelectedText {
    /// 便捷构造：来源未知、语言与位置未知的原始结果。
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            source: SelectionSource::Unknown,
            language: None,
            rect: None,
        }
    }

    /// 空白文本视为无效选区，调用方应直接忽略而不是发起翻译。
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn blank_detection_trims_whitespace() {
        assert!(SelectedText::new("  \n\t").is_blank());
        assert!(!SelectedText::new("ephemeral").is_blank());
    }

    #[test]
    fn serializes_camel_case() {
        let text = SelectedText {
            text: "hello".into(),
            source: SelectionSource::Clipboard,
            language: Some(Language::En),
            rect: Some(Rect {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 24.0,
            }),
        };
        let json = serde_json::to_value(&text).unwrap();
        assert_eq!(json["source"], "clipboard");
        assert!(json.get("rect").unwrap()["width"].as_f64().unwrap() == 100.0);
        assert!(json.get("language").is_some());
    }
}
