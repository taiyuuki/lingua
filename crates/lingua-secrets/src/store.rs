//! [`SecretStore`] trait 与 keyring 实现。

use lingua_core::{AppError, AppResult};

/// 机密的逻辑引用：`service + account` 唯一定位一条凭据。
///
/// 约定：Provider 的 API Key 存为 `service = "com.taiyuuki.lingua"`、
/// `account = "provider/{provider_id}"`，引用字符串（`provider/default`）
/// 可以安全地入库，明文只存在于密钥库。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRef {
    pub account: String,
}

impl SecretRef {
    pub fn provider(provider_id: &str) -> Self {
        Self {
            account: format!("provider/{provider_id}"),
        }
    }
}

/// 机密存储抽象。实现必须是线程安全的（全局共享）。
pub trait SecretStore: Send + Sync {
    /// 写入（或覆盖）机密。
    fn set_secret(&self, reference: &SecretRef, secret: &str) -> AppResult<()>;

    /// 读取机密；不存在返回 `None`。
    fn get_secret(&self, reference: &SecretRef) -> AppResult<Option<String>>;

    /// 删除机密；返回是否真的删除了。
    fn delete_secret(&self, reference: &SecretRef) -> AppResult<bool>;
}

/// 基于 `keyring` crate 的实现。服务名固定为应用 identifier，
/// 这样系统密钥管理器里能一眼认出条目归属。
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, reference: &SecretRef) -> AppResult<keyring::Entry> {
        keyring::Entry::new(&self.service, &reference.account)
            .map_err(|error| AppError::Authentication(format!("keyring unavailable: {error}")))
    }
}

impl Default for KeyringSecretStore {
    fn default() -> Self {
        Self::new("com.taiyuuki.lingua")
    }
}

impl SecretStore for KeyringSecretStore {
    fn set_secret(&self, reference: &SecretRef, secret: &str) -> AppResult<()> {
        self.entry(reference)?
            .set_password(secret)
            .map_err(|error| AppError::Authentication(format!("failed to store secret: {error}")))
    }

    fn get_secret(&self, reference: &SecretRef) -> AppResult<Option<String>> {
        match self.entry(reference)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(AppError::Authentication(format!(
                "failed to read secret: {error}"
            ))),
        }
    }

    fn delete_secret(&self, reference: &SecretRef) -> AppResult<bool> {
        match self.entry(reference)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(error) => Err(AppError::Authentication(format!(
                "failed to delete secret: {error}"
            ))),
        }
    }
}

// keychain 交互依赖真实 OS 密钥库，不适合在 `cargo test` 里无条件运行；
// 手动验证：cargo test -p lingua-secrets -- --ignored
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    #[ignore = "touches the real OS keychain; run manually"]
    fn keychain_roundtrip() {
        let store = KeyringSecretStore::new("com.taiyuuki.lingua.test");
        let reference = SecretRef::provider("test-entry");
        store.delete_secret(&reference).unwrap();

        assert_eq!(store.get_secret(&reference).unwrap(), None);
        store.set_secret(&reference, "sk-test-123").unwrap();
        assert_eq!(
            store.get_secret(&reference).unwrap(),
            Some("sk-test-123".into())
        );
        assert!(store.delete_secret(&reference).unwrap());
        assert_eq!(store.get_secret(&reference).unwrap(), None);
    }
}
