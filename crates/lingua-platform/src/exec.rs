//! 外部工具可执行文件的定位。
//!
//! OCR 引擎（tesseract）以 CLI 方式调用，设置里默认只填裸名 `tesseract`、
//! 靠进程 PATH 查找。但 GUI 进程的 PATH 与登录 shell 不是一回事：macOS 上
//! 从 Finder / Dock / LaunchAgent 启动的应用拿到的是 launchd 的默认 PATH
//! （`/usr/bin:/bin:/usr/sbin:/sbin`），Homebrew 的 `/opt/homebrew/bin`
//! 不在其中 —— 「brew 明明装了，应用却提示未安装」的根因就在这里。
//!
//! [`resolve_executable`] 在 PATH 查找失败后继续探测调用方给出的工具专属
//! 目录与本平台的常见安装目录；目录清单、可执行判定、`PATHEXT` 等平台
//! 差异全部收敛在本模块。

use lingua_core::{AppError, AppResult};
use std::path::{Path, PathBuf};

/// 按设置值定位可执行文件。
///
/// * `configured` 含路径分隔符（或绝对路径）→ 显式路径，只校验存在且可执行；
/// * 裸名 → 依次在进程 PATH、`extra_dirs`（工具专属安装目录）、本平台常见
///   安装目录里查找。
///
/// 阻塞调用（几次 stat），上层按需用 `spawn_blocking` 包裹。
pub fn resolve_executable(configured: &str, extra_dirs: &[PathBuf]) -> AppResult<PathBuf> {
    let configured = configured.trim();
    if configured.is_empty() {
        return Err(AppError::InvalidInput(
            "executable path is empty".to_string(),
        ));
    }
    if is_explicit_path(configured) {
        let path = PathBuf::from(configured);
        return if is_executable(&path) {
            Ok(path)
        } else {
            Err(AppError::Platform(format!(
                "`{configured}` does not exist or is not executable"
            )))
        };
    }

    let path_dirs = path_dirs();
    find_in_dirs(configured, &path_dirs)
        .or_else(|| find_in_dirs(configured, extra_dirs))
        .or_else(|| find_in_dirs(configured, &tool_dirs()))
        .ok_or_else(|| {
            AppError::Platform(format!(
                "`{configured}` was not found in PATH or the standard install locations"
            ))
        })
}

/// 显式路径（含分隔符，或 Windows 盘符前缀）还是裸名。
fn is_explicit_path(configured: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        configured.contains(['/', '\\']) || configured.contains(':')
    }

    #[cfg(not(target_os = "windows"))]
    {
        configured.contains('/')
    }
}

/// 进程 PATH 里的目录。空项按「当前目录」解释，跳过以免误执行 cwd 里的同名文件。
fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|value| {
            std::env::split_paths(&value)
                .filter(|dir| !dir.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// 本平台 CLI 工具的常见安装目录：GUI 进程的 PATH 里没有，但工具常装在这里。
#[cfg(target_os = "macos")]
fn tool_dirs() -> Vec<PathBuf> {
    // Apple Silicon Homebrew / Intel Homebrew 与手动安装 / MacPorts。
    ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

#[cfg(target_os = "windows")]
fn tool_dirs() -> Vec<PathBuf> {
    // Chocolatey / Scoop 的 shim 目录。
    let mut dirs = Vec::new();
    if let Some(program_data) = std::env::var_os("ProgramData") {
        dirs.push(PathBuf::from(program_data).join("chocolatey").join("bin"));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        dirs.push(PathBuf::from(profile).join("scoop").join("shims"));
    }
    dirs
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn tool_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// 依次在各目录下找 `name`（Windows 上按 `PATHEXT` 补扩展名）。
fn find_in_dirs(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().find_map(|dir| {
        candidate_names(name)
            .iter()
            .map(|candidate| dir.join(candidate))
            .find(|path| is_executable(path))
    })
}

/// `name` 在目录里的候选文件名。
#[cfg(target_os = "windows")]
fn candidate_names(name: &str) -> Vec<String> {
    // 已带扩展名就不再补（`tools\custom.exe` 之类）。
    if Path::new(name).extension().is_some() {
        return vec![name.to_string()];
    }
    let default_exts = ".COM;.EXE;.BAT;.CMD";
    let pathtext = std::env::var("PATHEXT").unwrap_or_else(|_| default_exts.to_string());
    let mut names: Vec<String> = pathtext
        .split(';')
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| format!("{name}{ext}"))
        .collect();
    if !names
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
    {
        names.push(name.to_string());
    }
    names
}

#[cfg(not(target_os = "windows"))]
fn candidate_names(name: &str) -> Vec<String> {
    vec![name.to_string()]
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 每个用例独占的临时目录。
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lingua-exec-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_name(name: &str) -> String {
        format!("lingua-exec-test-{name}")
    }

    /// 放一个假的可执行文件，返回它的路径。
    fn fake_tool(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(tool_name(name));
        std::fs::write(&path, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn bare_name_found_in_extra_dirs() {
        let dir = scratch_dir("extra");
        let tool = fake_tool(&dir, "extra");
        let resolved = resolve_executable(&tool_name("extra"), std::slice::from_ref(&dir)).unwrap();
        assert_eq!(resolved, tool);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explicit_path_is_taken_verbatim() {
        let dir = scratch_dir("explicit");
        let tool = fake_tool(&dir, "explicit");
        let resolved = resolve_executable(tool.to_str().unwrap(), &[]).unwrap();
        assert_eq!(resolved, tool);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_tool_reports_platform_error() {
        let error = resolve_executable(&tool_name("no-such-tool"), &[]).unwrap_err();
        assert!(matches!(error, AppError::Platform(_)), "{error:?}");
        assert!(
            error.to_string().contains(&tool_name("no-such-tool")),
            "{error}"
        );
    }

    #[test]
    fn empty_configuration_is_invalid_input() {
        let error = resolve_executable("   ", &[]).unwrap_err();
        assert!(matches!(error, AppError::InvalidInput(_)), "{error:?}");
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_file_is_rejected() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch_dir("noexec");
        let path = dir.join(tool_name("noexec"));
        std::fs::write(&path, b"not a program").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error =
            resolve_executable(&tool_name("noexec"), std::slice::from_ref(&dir)).unwrap_err();
        assert!(matches!(error, AppError::Platform(_)), "{error:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真机自检（默认忽略）：Homebrew 装的 tesseract 在「没有 Homebrew 的 PATH」
    /// 下仍能被找到 —— 复现 GUI 进程的启动环境。
    /// `cargo test -p lingua-platform resolves_homebrew_without_path -- --ignored --nocapture`
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "依赖本机的 Homebrew tesseract"]
    fn resolves_homebrew_without_path() {
        let tesseract = PathBuf::from("/opt/homebrew/bin/tesseract");
        let intel = PathBuf::from("/usr/local/bin/tesseract");
        if !is_executable(&tesseract) && !is_executable(&intel) {
            return;
        }
        // PATH 查找空手而归（等价于 GUI 进程的 PATH）时，由常见安装目录兜底。
        assert!(find_in_dirs("tesseract", &[]).is_none());
        let resolved = find_in_dirs("tesseract", &tool_dirs()).unwrap();
        assert!(resolved.ends_with("tesseract"), "{}", resolved.display());
        assert!(is_executable(&resolved), "{}", resolved.display());
    }
}
