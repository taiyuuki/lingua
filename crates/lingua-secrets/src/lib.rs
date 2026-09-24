//! OS 密钥库抽象。
//!
//! 硬性约束（来自需求）：**API Key 等机密不允许明文写入 SQLite**。
//! 本 crate 是唯一允许接触机密明文的模块：
//!
//! * macOS  → Keychain（keyring `apple-native` store）
//! * Windows → Credential Manager（`windows-native` store）
//! * Linux  → Secret Service（gnome-keyring / KWallet，`zbus` 实现；
//!   需要桌面会话里有 keyring 守护进程，无头环境会失败）
//!
//! keyring v4 默认特性（v1 兼容层）在三大平台各自使用原生凭据库，
//! 因此这里不挑平台特性，直接用默认。

pub mod store;

pub use store::{KeyringSecretStore, SecretRef, SecretStore};
