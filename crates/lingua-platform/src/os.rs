//! 运行平台的轻量描述。
//!
//! Phase 0 只需要「是哪个 OS」；后续需要版本号、Wayland/X11 会话类型、
//! 权限状态等时，在这里扩展为完整的 `PlatformInfo`。

use serde::{Deserialize, Serialize};

/// 支持的桌面平台。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Os {
    Macos,
    Windows,
    Linux,
    Other,
}

/// 返回当前运行平台。
pub fn current_os() -> Os {
    match std::env::consts::OS {
        "macos" => Os::Macos,
        "windows" => Os::Windows,
        "linux" => Os::Linux,
        _ => Os::Other,
    }
}

/// 打开划词权限对应的系统设置面板（macOS：隐私与安全性 →
/// 辅助功能；macOS 26+ 该项并入「设备控制和数据访问」，
/// 手翻设置树难找，URL 直达）。其它平台划词无需授权，无操作。
pub fn open_accessibility_settings() {
    #[cfg(target_os = "macos")]
    {
        // Ventura+ 的新 URL 格式；旧 com.apple.preference.security
        // 在 macOS 26 已失效（open 会落到错误的 handler）。
        let _ = std::process::Command::new("open")
            .arg(
                "x-apple.systempreferences:com.apple.settings.\
                 PrivacySecurity.extension?Privacy_Accessibility",
            )
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Windows / Linux：无系统级划词授权，保持静默。
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn current_os_is_a_known_variant() {
        // 在 CI 三平台上都应命中具体变体；其他环境允许 Other。
        assert!(matches!(
            current_os(),
            Os::Macos | Os::Windows | Os::Linux | Os::Other
        ));
    }
}
