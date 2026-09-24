//! Tesseract CLI 实现。
//!
//! 通过 `tesseract <image> stdout tsv` 拿到 TSV 输出，包含词级
//! 包围盒与置信度；按 (block, par, line) 分组还原行/块结构。
//! 需要本机安装 tesseract（macOS: `brew install tesseract`）。

use lingua_core::{AppError, AppResult, OcrBlock, OcrImage, OcrLine, OcrResult};
use std::process::Command;
use std::sync::Mutex;

pub struct TesseractOcrProvider {
    binary: Mutex<String>,
}

impl Clone for TesseractOcrProvider {
    fn clone(&self) -> Self {
        Self {
            binary: Mutex::new(self.binary()),
        }
    }
}

impl TesseractOcrProvider {
    /// `binary` 是 tesseract 可执行文件路径或 PATH 中的名字。
    pub fn new(binary: impl Into<String>) -> Self {
        Self {
            binary: Mutex::new(binary.into()),
        }
    }

    /// 运行期更新二进制路径（设置保存后立即生效，无需重建 Provider）。
    pub fn set_binary(&self, binary: impl Into<String>) {
        *self
            .binary
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = binary.into();
    }

    fn binary(&self) -> String {
        self.binary
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// 调 CLI 并解析 TSV。阻塞调用，上层用 `spawn_blocking` 包裹。
    fn recognize_blocking(&self, image: &OcrImage) -> AppResult<OcrResult> {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!(
            "lingua-ocr-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&path, &image.png)
            .map_err(|error| AppError::Ocr(format!("failed to stage image: {error}")))?;

        let output = Command::new(self.binary())
            .args([path.to_string_lossy().as_ref(), "stdout", "tsv"])
            .output();

        let result = match output {
            Ok(output) if output.status.success() => {
                let tsv = String::from_utf8_lossy(&output.stdout);
                Ok(parse_tsv(&tsv))
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let message = first_line(&stderr);
                if message.contains("unable to load")
                    || message.to_lowercase().contains("not found")
                {
                    Err(AppError::Platform(format!(
                        "tesseract failed to load language data: {message}"
                    )))
                } else {
                    Err(AppError::Ocr(format!("tesseract failed: {message}")))
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(AppError::Platform(
                "tesseract is not installed or not in PATH; install it (e.g. `brew install \
                 tesseract`) or set the path in Settings"
                    .to_string(),
            )),
            Err(error) => Err(AppError::Platform(format!(
                "failed to run tesseract: {error}"
            ))),
        };

        let _ = std::fs::remove_file(&path);
        result
    }
}

#[async_trait::async_trait]
impl crate::provider::OcrProvider for TesseractOcrProvider {
    fn name(&self) -> &str {
        "tesseract-cli"
    }

    async fn recognize(&self, image: &OcrImage) -> AppResult<OcrResult> {
        let this = self.clone();
        let image = image.clone();
        tokio::task::spawn_blocking(move || this.recognize_blocking(&image))
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("unknown error")
        .trim()
        .to_string()
}

/// tesseract TSV 一行：level page block par line word left top width height conf text
#[derive(Debug, Clone)]
struct TsvWord {
    block: u32,
    par: u32,
    line: u32,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    confidence: f32,
    text: String,
}

/// 解析 tesseract TSV（level==5 为词级；conf < 0 的行跳过）。
pub(crate) fn parse_tsv(tsv: &str) -> OcrResult {
    let mut words: Vec<TsvWord> = Vec::new();
    for row in tsv.lines().skip(1) {
        let columns: Vec<&str> = row.split('\t').collect();
        if columns.len() < 12 {
            continue;
        }
        // level page block par line word left top width height conf text...
        let Ok(level) = columns[0].trim().parse::<u32>() else {
            continue;
        };
        if level != 5 {
            continue;
        }
        let text = columns[11].trim();
        if text.is_empty() {
            continue;
        }
        let Ok(confidence) = columns[10].trim().parse::<f32>() else {
            continue;
        };
        if confidence < 0.0 {
            continue;
        }
        let parsed = (|| -> Option<TsvWord> {
            Some(TsvWord {
                block: columns[2].trim().parse().ok()?,
                par: columns[3].trim().parse().ok()?,
                line: columns[4].trim().parse().ok()?,
                left: columns[6].trim().parse().ok()?,
                top: columns[7].trim().parse().ok()?,
                width: columns[8].trim().parse().ok()?,
                height: columns[9].trim().parse().ok()?,
                confidence: confidence / 100.0,
                text: text.to_string(),
            })
        })();
        if let Some(word) = parsed {
            words.push(word);
        }
    }

    // 按 (block, par, line) 聚合成行，再按 block 聚合成块。
    struct GroupedLine {
        words: Vec<TsvWord>,
    }

    let mut lines: Vec<(u32, u32, u32, GroupedLine)> = Vec::new();
    for word in words.iter().cloned() {
        let key = (word.block, word.par, word.line);
        match lines
            .iter_mut()
            .find(|(b, p, l, _)| *b == key.0 && *p == key.1 && *l == key.2)
        {
            Some((_, _, _, group)) => group.words.push(word),
            None => lines.push((key.0, key.1, key.2, GroupedLine { words: vec![word] })),
        }
    }

    fn line_of(group: &GroupedLine) -> OcrLine {
        let (left, top) = group
            .words
            .iter()
            .map(|w| (w.left, w.top))
            .fold((u32::MAX, u32::MAX), |(x1, y1), (x2, y2)| {
                (x1.min(x2), y1.min(y2))
            });
        let right = group
            .words
            .iter()
            .map(|w| w.left + w.width)
            .max()
            .unwrap_or(left);
        let bottom = group
            .words
            .iter()
            .map(|w| w.top + w.height)
            .max()
            .unwrap_or(top);
        let confidence =
            group.words.iter().map(|w| w.confidence).sum::<f32>() / group.words.len() as f32;
        OcrLine {
            text: group
                .words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            bbox: [
                left,
                top,
                right.saturating_sub(left),
                bottom.saturating_sub(top),
            ],
            confidence,
            words: group.words.iter().map(|w| w.text.clone()).collect(),
        }
    }

    let mut blocks: Vec<(u32, Vec<OcrLine>)> = Vec::new();
    for (block, _, _, group) in lines {
        let line = line_of(&group);
        match blocks.iter_mut().find(|(b, _)| *b == block) {
            Some((_, lines)) => lines.push(line),
            None => blocks.push((block, vec![line])),
        }
    }

    let blocks: Vec<OcrBlock> = blocks
        .into_iter()
        .map(|(_, lines)| {
            let text = lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let (left, top, right, bottom) =
                lines
                    .iter()
                    .fold((u32::MAX, u32::MAX, 0, 0), |(x1, y1, x2, y2), line| {
                        let [lx, ly, lw, lh] = line.bbox;
                        (x1.min(lx), y1.min(ly), x2.max(lx + lw), y2.max(ly + lh))
                    });
            let confidence = if lines.is_empty() {
                0.0
            } else {
                lines.iter().map(|line| line.confidence).sum::<f32>() / lines.len() as f32
            };
            OcrBlock {
                text,
                bbox: [
                    left,
                    top,
                    right.saturating_sub(left),
                    bottom.saturating_sub(top),
                ],
                confidence,
                lines,
            }
        })
        .collect();

    let text = blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    OcrResult {
        text,
        blocks,
        language: None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const SAMPLE_TSV: &str = "level\tpage\tblock\tpar\tline\tword\tleft\ttop\twidth\theight\tconf\ttext\n\
        1\t1\t0\t0\t0\t-1\t0\t0\t300\t100\t-1\t\n\
        5\t1\t1\t1\t1\t1\t10\t20\t50\t14\t96.5\tHello\n\
        5\t1\t1\t1\t1\t2\t70\t20\t60\t14\t92.1\tworld\n\
        5\t1\t2\t1\t2\t1\t10\t50\t80\t14\t88.0\tephemeral\n\
        5\t1\t2\t1\t2\t2\t10\t50\t80\t14\t-1\t\n";

    #[test]
    fn parses_blocks_lines_and_words() {
        let result = parse_tsv(SAMPLE_TSV);
        assert_eq!(result.blocks.len(), 2);

        let first = &result.blocks[0];
        assert_eq!(first.text, "Hello world");
        assert_eq!(first.lines.len(), 1);
        assert_eq!(first.lines[0].words, vec!["Hello", "world"]);
        assert_eq!(first.bbox, [10, 20, 120, 14]);
        assert!((first.confidence - (96.5 + 92.1) / 200.0).abs() < 1e-4);

        let second = &result.blocks[1];
        assert_eq!(second.text, "ephemeral");
        assert_eq!(second.lines[0].confidence, 0.88);
        assert_eq!(result.text, "Hello world\nephemeral");
    }

    #[test]
    fn empty_tsv_yields_empty_result() {
        let result = parse_tsv(
            "level\tpage\tblock\tpar\tline\tword\tleft\ttop\twidth\theight\tconf\ttext\n",
        );
        assert!(result.is_empty());
        assert!(result.blocks.is_empty());
    }
}
