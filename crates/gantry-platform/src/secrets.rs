use std::sync::Arc;

use keyring_core::{CredentialStore, Error as KeyringError};

use crate::{PlatformError, os};

/// API keys and, later, ATS passwords. Never stored in TOML or SQLite (§12).
pub trait SecretStore: std::fmt::Debug + Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, PlatformError>;
    fn set(&self, key: &str, value: &str) -> Result<(), PlatformError>;
    /// Removing a key that does not exist is not an error.
    fn delete(&self, key: &str) -> Result<(), PlatformError>;
}

/// Service name every Gantry secret is filed under in the OS keyring.
const SERVICE: &str = "gantry";

/// Secret Service on Linux, Credential Manager on Windows, Keychain on macOS.
#[derive(Debug)]
pub struct OsSecretStore {
    store: Arc<CredentialStore>,
}

impl OsSecretStore {
    pub fn open() -> Result<Self, PlatformError> {
        Ok(Self {
            store: os::credential_store()?,
        })
    }

    fn entry(&self, key: &str) -> Result<keyring_core::Entry, PlatformError> {
        Ok(self.store.build(SERVICE, key, None)?)
    }
}

impl SecretStore for OsSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>, PlatformError> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), PlatformError> {
        Ok(self.entry(key)?.set_password(value)?)
    }

    fn delete(&self, key: &str) -> Result<(), PlatformError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
