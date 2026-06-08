use std::collections::BTreeMap;
use std::sync::Arc;

use crate::config::CredentialVaultBackend;
use crate::error::DaemonError;

pub trait CredentialVaultStore: Send + Sync + std::fmt::Debug {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError>;
    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError>;
    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError>;
}

#[derive(Debug, Default)]
pub struct PlatformKeychainCredentialVaultStore;

impl CredentialVaultStore for PlatformKeychainCredentialVaultStore {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
        keyring_entry(service, key)?
            .get_password()
            .map_err(|error| vault_error("get", key, error))
    }

    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        keyring_entry(service, key)?
            .set_password(value)
            .map_err(|error| vault_error("set", key, error))
    }

    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
        keyring_entry(service, key)?
            .delete_credential()
            .map_err(|error| vault_error("delete", key, error))
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Default)]
pub struct LinuxKeyutilsCredentialVaultStore;

#[cfg(target_os = "linux")]
impl CredentialVaultStore for LinuxKeyutilsCredentialVaultStore {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
        keyutils_entry(service, key)?
            .get_password()
            .map_err(|error| vault_error("get", key, error))
    }

    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        keyutils_entry(service, key)?
            .set_password(value)
            .map_err(|error| vault_error("set", key, error))
    }

    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
        keyutils_entry(service, key)?
            .delete_credential()
            .map_err(|error| vault_error("delete", key, error))
    }
}

#[derive(Debug, Default)]
pub struct ProcessMemoryCredentialVaultStore;

impl CredentialVaultStore for ProcessMemoryCredentialVaultStore {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
        process_memory_vault()
            .lock()
            .map_err(|error| secret_error(format!("process memory vault lock poisoned: {error}")))?
            .get(&(service.to_string(), key.to_string()))
            .cloned()
            .ok_or_else(|| {
                secret_error(format!(
                    "credential `{key}` not found in process memory vault"
                ))
            })
    }

    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        process_memory_vault()
            .lock()
            .map_err(|error| secret_error(format!("process memory vault lock poisoned: {error}")))?
            .insert((service.to_string(), key.to_string()), value.to_string());
        Ok(())
    }

    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
        process_memory_vault()
            .lock()
            .map_err(|error| secret_error(format!("process memory vault lock poisoned: {error}")))?
            .remove(&(service.to_string(), key.to_string()));
        Ok(())
    }
}

pub(super) fn vault_store_for_backend(
    backend: CredentialVaultBackend,
) -> Result<Arc<dyn CredentialVaultStore>, DaemonError> {
    match backend {
        CredentialVaultBackend::OsKeychain => Ok(Arc::new(PlatformKeychainCredentialVaultStore)),
        CredentialVaultBackend::ProcessMemory => {
            if process_memory_vault_backend_allowed() {
                Ok(Arc::new(ProcessMemoryCredentialVaultStore))
            } else {
                Err(secret_error(
                    "credential_vault.backend=process_memory is volatile and is only allowed inside Arroba slices or with ARROBA_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT=1".to_string(),
                ))
            }
        }
        CredentialVaultBackend::LinuxKeyutils => {
            #[cfg(target_os = "linux")]
            {
                Ok(Arc::new(LinuxKeyutilsCredentialVaultStore))
            }
            #[cfg(not(target_os = "linux"))]
            {
                Err(secret_error(
                    "credential_vault.backend=linux_keyutils is only supported on Linux"
                        .to_string(),
                ))
            }
        }
    }
}

fn process_memory_vault_backend_allowed() -> bool {
    std::env::var("ARROBA_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        || std::env::var_os("ARROBA_SLICE_MACHINE_ID").is_some()
}

fn process_memory_vault() -> &'static std::sync::Mutex<BTreeMap<(String, String), String>> {
    static VAULT: std::sync::OnceLock<std::sync::Mutex<BTreeMap<(String, String), String>>> =
        std::sync::OnceLock::new();
    VAULT.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

fn keyring_entry(service: &str, key: &str) -> Result<keyring::Entry, DaemonError> {
    keyring::Entry::new(service, key).map_err(|error| vault_error("open", key, error))
}

#[cfg(target_os = "linux")]
fn keyutils_entry(service: &str, key: &str) -> Result<keyring::Entry, DaemonError> {
    let credential = keyring::keyutils::KeyutilsCredential::new_with_target(None, service, key)
        .map_err(|error| vault_error("open", key, error))?;
    Ok(keyring::Entry::new_with_credential(Box::new(credential)))
}

fn vault_error(operation: &'static str, key: &str, error: keyring::Error) -> DaemonError {
    secret_error(format!(
        "failed to {operation} credential `{key}` in {}: {error}",
        platform_keychain_backend_name()
    ))
}

fn platform_keychain_backend_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macOS Keychain"
    }
    #[cfg(target_os = "windows")]
    {
        "Windows Credential Manager"
    }
    #[cfg(target_os = "linux")]
    {
        "Linux keyutils/Secret Service"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "platform keychain"
    }
}

fn secret_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "credential_vault",
        message,
    }
}
