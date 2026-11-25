//! Tesseract CLI 实现。
//!
//! 通过 `tesseract <image> stdout tsv` 拿到 TSV 输出，包含词级
//! 包围盒与置信度；按 (block, par, line) 分组还原行/块结构。
//! 需要本机安装 tesseract（macOS: `brew install tesseract`）。
//!
//! 可执行文件经 [`resolve_binary`] 定位：设置里的裸名先查 PATH，再探
//! 常见安装目录 —— macOS 上从 Finder / Dock 启动的 GUI 进程拿不到登录
//! shell 的 PATH，Homebrew 装的 tesseract 对它并不可见。

use lingua_core::{AppError, AppResult, OcrBlock, OcrImage, OcrLine, OcrResult};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

/// 定位 tesseract 可执行文件。
///
/// `configured` 是可执行文件路径或 PATH 中的名字。找不到时的提示同时给出
/// 两种修法（安装 / 在设置里填完整路径）。
fn resolve_binary(configured: &str) -> AppResult<PathBuf> {
    // 只有 Windows 往 extra_dirs 里放东西。
    #[allow(unused_mut)]
    let mut extra_dirs = Vec::new();
    #[cfg(target_os = "windows")]
    {
        // winget 包 `UB-Mannheim.TesseractOCR`（安装器）的默认安装目录；
        // 安装时未勾选「Add to PATH」的话，应用只能在这里找到它。
        for (variable, tail) in [
            ("ProgramFiles", "Tesseract-OCR"),
            ("ProgramFiles(x86)", "Tesseract-OCR"),
            ("LocalAppData", "Programs/Tesseract-OCR"),
        ] {
            if let Some(base) = std::env::var_os(variable) {
                extra_dirs.push(PathBuf::from(base).join(tail));
            }
        }
    }
    lingua_platform::resolve_executable(configured, &extra_dirs).map_err(|error| {
        AppError::Platform(format!(
            "{error}; install tesseract (e.g. `brew install tesseract` / `winget install \
             UB-Mannheim.TesseractOCR`) or set its full path in Settings"
        ))
    })
}

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

        let binary = resolve_binary(&self.binary())?;
        let output = Command::new(&binary)
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
            // 「未安装」已在 resolve_binary 里给出带修法的提示；走到这里说明
            // 文件在，但拉不起来（架构不符、权限不足等）。
            Err(error) => Err(AppError::Platform(format!(
                "failed to run tesseract at {}: {error}",
                binary.display()
            ))),
        };

        let _ = std::fs::remove_file(&path);
        result
    }
}

/// tesseract 自检结果（设置页展示用）。任何一步失败都不返回 `Err`，
/// 而是落到 `error`，让用户看到「实际用的是哪个文件、哪里不对」。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TesseractStatus {
    pub ok: bool,
    /// 实际使用的可执行文件（裸名经 PATH / 常见安装目录解析后的路径）。
    pub path: Option<String>,
    /// 版本号，如 `5.5.3`。
    pub version: Option<String>,
    /// 已安装的语言包（`--list-langs`），如 `["eng", "osd"]`。
    pub languages: Vec<String>,
    pub error: Option<String>,
}

/// 自检：定位可执行文件并读取版本与语言包列表。阻塞调用，
/// 上层用 `spawn_blocking` 包裹。
pub fn tesseract_status(configured: &str) -> TesseractStatus {
    let binary = match resolve_binary(configured) {
        Ok(binary) => binary,
        Err(error) => {
            return TesseractStatus {
                error: Some(error.to_string()),
                ..Default::default()
            };
        }
    };
    let mut status = TesseractStatus {
        path: Some(binary.display().to_string()),
        ..Default::default()
    };
    let run = |args: &[&str]| Command::new(&binary).args(args).output();

    match run(&["--version"]) {
        Ok(output) if output.status.success() => {
            // 首行形如 `tesseract 5.5.3`。
            let stdout = String::from_utf8_lossy(&output.stdout);
            status.version = Some(
                first_line(&stdout)
                    .trim_start_matches("tesseract")
                    .trim()
                    .to_string(),
            );
        }
        Ok(output) => {
            status.error = Some(format!(
                "tesseract --version failed: {}",
                first_line(&String::from_utf8_lossy(&output.stderr))
            ));
            return status;
        }
        Err(error) => {
            status.error = Some(format!(
                "failed to run tesseract at {}: {error}",
                binary.display()
            ));
            return status;
        }
    }

    match run(&["--list-langs"]) {
        Ok(output) if output.status.success() => {
            // 首行是「List of available languages in ... (N):」，其余每行一个语言包。
            status.languages = String::from_utf8_lossy(&output.stdout)
                .lines()
                .skip(1)
                .map(str::trim)
                .filter(|language| !language.is_empty())
                .map(str::to_string)
                .collect();
        }
        Ok(output) => {
            status.error = Some(format!(
                "tesseract --list-langs failed: {}",
                first_line(&String::from_utf8_lossy(&output.stderr))
            ));
            return status;
        }
        Err(error) => {
            status.error = Some(format!(
                "failed to run tesseract at {}: {error}",
                binary.display()
            ));
            return status;
        }
    }

    status.ok = true;
    status
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

    /// 真机自检（默认忽略）：裸名能定位到本机 tesseract，版本与语言包解析正确。
    /// `cargo test -p lingua-ocr reports_real_tesseract -- --ignored --nocapture`
    #[test]
    #[ignore = "依赖本机安装的 tesseract"]
    fn reports_real_tesseract() {
        let status = tesseract_status("tesseract");
        println!("{status:?}");
        assert!(status.ok, "{status:?}");
        let path = status.path.unwrap_or_default();
        assert!(
            path.ends_with("tesseract") || path.ends_with("tesseract.exe"),
            "{path}"
        );
        assert!(status.version.is_some_and(|version| !version.is_empty()));
    }

    #[test]
    fn missing_binary_reports_error_not_panic() {
        let status = tesseract_status("lingua-no-such-tesseract-binary");
        assert!(!status.ok);
        assert!(status.path.is_none());
        assert!(status.error.is_some());
    }
}
