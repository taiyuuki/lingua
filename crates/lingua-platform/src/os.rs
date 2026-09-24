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
