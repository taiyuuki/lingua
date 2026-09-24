//! Windows 的两个划词实现（与 `macos/providers.rs` 对称）。
//!
//! * [`UiaSelectionProvider`]：UI Automation TextPattern，最干净，
//!   不污染剪贴板；覆盖率取决于应用是否实现 TextPattern
//!   （主流浏览器 / Windows Terminal / 常见编辑器均可）。
//! * [`ClipboardSelectionProvider`]：备份剪贴板 → 模拟 Ctrl+C →
//!   轮询剪贴板 → 恢复剪贴板（模拟按键前先等快捷键修饰键释放，
//!   否则 Alt 未松开时发出去的是 Alt+Ctrl+C，见 ffi 文档）。
//!   永远可用，但会短暂触碰用户剪贴板，只作为 fallback。

use super::ffi;
use crate::selection::SelectionProvider;
use lingua_core::{AppError, AppResult, Rect, SelectedText, SelectionSource};
use std::time::{Duration, Instant};

fn cursor_rect() -> Option<Rect> {
    ffi::cursor_position().map(|(x, y)| Rect {
        x,
        y,
        width: 1.0,
        height: 20.0,
    })
}

/// 通过 UI Automation TextPattern 读取选中文本。
pub struct UiaSelectionProvider;

#[async_trait::async_trait]
impl SelectionProvider for UiaSelectionProvider {
    fn name(&self) -> &str {
        "windows-uia"
    }

    async fn capture(&self) -> AppResult<SelectedText> {
        // UIA 是跨进程 COM 调用，放到阻塞线程池。
        let text = tokio::task::spawn_blocking(ffi::uia_capture_selected_text)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))??;

        tracing::debug!(chars = text.chars().count(), "selection captured via UIA");
        Ok(SelectedText {
            text,
            source: SelectionSource::Accessibility,
            language: None,
            rect: cursor_rect(),
        })
    }
}

/// 通过「模拟 Ctrl+C + 读剪贴板」获取选中文本。
#[derive(Clone, Copy)]
pub struct ClipboardSelectionProvider {
    poll_interval: Duration,
    max_wait: Duration,
}

impl Default for ClipboardSelectionProvider {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(40),
            max_wait: Duration::from_millis(600),
        }
    }
}

impl ClipboardSelectionProvider {
    fn capture_blocking(self) -> AppResult<SelectedText> {
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|error| AppError::Platform(format!("clipboard unavailable: {error}")))?;

        // 1. 备份现有剪贴板文本（仅文本；富格式会丢失，见架构文档已知限制）。
        let previous = clipboard.get_text().ok();

        // 2. 模拟 Ctrl+C。
        ffi::post_copy_shortcut();

        // 3. 轮询剪贴板，等待内容变化。
        let deadline = Instant::now() + self.max_wait;
        let mut captured: AppResult<String> = Err(AppError::NoSelection);
        while Instant::now() < deadline {
            std::thread::sleep(self.poll_interval);
            if let Ok(current) = clipboard.get_text() {
                let changed = previous.as_ref() != Some(&current);
                if changed && !current.trim().is_empty() {
                    captured = Ok(current);
                    break;
                }
            }
        }

        // 4. 恢复用户剪贴板（尽力而为）。
        if let Some(previous) = previous {
            if clipboard.set_text(&previous).is_err() {
                tracing::warn!("failed to restore previous clipboard content");
            }
        }

        let text = captured?;
        tracing::debug!(
            chars = text.chars().count(),
            "selection captured via clipboard"
        );
        Ok(SelectedText {
            text,
            source: SelectionSource::Clipboard,
            language: None,
            rect: cursor_rect(),
        })
    }
}

#[async_trait::async_trait]
impl SelectionProvider for ClipboardSelectionProvider {
    fn name(&self) -> &str {
        "windows-clipboard"
    }

    async fn capture(&self) -> AppResult<SelectedText> {
        let this = *self;
        tokio::task::spawn_blocking(move || this.capture_blocking())
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
    }
}
