use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::config::{CredentialVaultBackend, UserCredentialVaultConfig};
use crate::error::DaemonError;

const VAULT_FILE_VERSION: u32 = 1;
const VAULT_CIPHER: &str = "aes-256-gcm";
const VAULT_KDF: &str = "argon2id";
const DEFAULT_ARGON2_MEMORY_KIB: u32 = 64 * 1024;
const DEFAULT_ARGON2_ITERATIONS: u32 = 3;
const DEFAULT_ARGON2_PARALLELISM: u32 = 1;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

pub trait CredentialVaultStore: Send + Sync + std::fmt::Debug {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError>;
    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError>;
    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultUnlockLease {
    Operation,
    TtlMinutes(u64),
    KernelShutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaVaultUnlockStatus {
    pub path: PathBuf,
    pub unlocked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ArrobaEncryptedCredentialVaultStore {
    path: PathBuf,
    kdf_profile: VaultKdfProfile,
}

impl ArrobaEncryptedCredentialVaultStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: normalize_vault_path(path.into()),
            kdf_profile: VaultKdfProfile::default(),
        }
    }

    #[cfg(test)]
    fn with_kdf_profile(path: impl Into<PathBuf>, kdf_profile: VaultKdfProfile) -> Self {
        Self {
            path: normalize_vault_path(path.into()),
            kdf_profile,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl CredentialVaultStore for ArrobaEncryptedCredentialVaultStore {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
        let vault_key = unlocked_vault_key(&self.path)?;
        let plaintext = read_vault_plaintext(&self.path, &vault_key)?;
        plaintext
            .secrets
            .get(service)
            .and_then(|service_secrets| service_secrets.get(key))
            .cloned()
            .ok_or_else(|| secret_error(format!("credential `{key}` not found in Arroba vault")))
    }

    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        let vault_key = unlocked_vault_key(&self.path)?;
        let mut plaintext = if self.path.exists() {
            read_vault_plaintext(&self.path, &vault_key)?
        } else {
            VaultPlaintext::default()
        };
        plaintext
            .secrets
            .entry(service.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());
        write_vault_plaintext(&self.path, &vault_key, &plaintext, &self.kdf_profile)
    }

    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
        let vault_key = unlocked_vault_key(&self.path)?;
        let mut plaintext = if self.path.exists() {
            read_vault_plaintext(&self.path, &vault_key)?
        } else {
            VaultPlaintext::default()
        };
        if let Some(service_secrets) = plaintext.secrets.get_mut(service) {
            service_secrets.remove(key);
            if service_secrets.is_empty() {
                plaintext.secrets.remove(service);
            }
        }
        write_vault_plaintext(&self.path, &vault_key, &plaintext, &self.kdf_profile)
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

pub(super) fn vault_store_for_config(
    config: &UserCredentialVaultConfig,
) -> Result<Arc<dyn CredentialVaultStore>, DaemonError> {
    match config.backend {
        CredentialVaultBackend::ArrobaEncrypted => Ok(Arc::new(
            ArrobaEncryptedCredentialVaultStore::new(config.path.clone()),
        )),
        CredentialVaultBackend::ProcessMemory => {
            if process_memory_vault_backend_allowed() {
                Ok(Arc::new(ProcessMemoryCredentialVaultStore))
            } else {
                Err(secret_error(
                    "credential_vault.backend=process_memory is volatile and is only allowed inside Arroba slices or with ARROBA_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT=1".to_string(),
                ))
            }
        }
    }
}

pub fn unlock_arroba_encrypted_vault(
    path: impl AsRef<Path>,
    passphrase: &str,
    lease: VaultUnlockLease,
) -> Result<ArrobaVaultUnlockStatus, DaemonError> {
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    validate_passphrase(passphrase)?;
    let now_ms = crate::session::unix_epoch_ms();
    let expires_at_ms = match lease {
        VaultUnlockLease::Operation => Some(now_ms + 5 * 60_000),
        VaultUnlockLease::TtlMinutes(minutes) => Some(now_ms + minutes.saturating_mul(60_000)),
        VaultUnlockLease::KernelShutdown => None,
    };
    let key = if path.exists() {
        let file = read_vault_file(&path)?;
        let key = derive_key(passphrase, &file.kdf)?;
        decrypt_vault_payload(&file, &key)?;
        key
    } else {
        let kdf = VaultKdfProfile::default().new_kdf_config();
        let key = derive_key(passphrase, &kdf)?;
        write_vault_file(&path, &key, &VaultPlaintext::default(), kdf)?;
        key
    };
    unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Arroba vault unlock state poisoned: {error}")))?
        .insert(path.clone(), UnlockedVault { key, expires_at_ms });
    Ok(ArrobaVaultUnlockStatus {
        path,
        unlocked: true,
        expires_at_ms,
    })
}

pub fn lock_arroba_encrypted_vault(path: impl AsRef<Path>) -> Result<(), DaemonError> {
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Arroba vault unlock state poisoned: {error}")))?
        .remove(&path);
    Ok(())
}

pub fn extend_arroba_encrypted_vault(
    path: impl AsRef<Path>,
    lease: VaultUnlockLease,
) -> Result<ArrobaVaultUnlockStatus, DaemonError> {
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    let mut unlocked = unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Arroba vault unlock state poisoned: {error}")))?;
    let vault = unlocked
        .get_mut(&path)
        .ok_or_else(|| vault_locked_error(&path))?;
    let now_ms = crate::session::unix_epoch_ms();
    vault.expires_at_ms = match lease {
        VaultUnlockLease::Operation => Some(now_ms),
        VaultUnlockLease::TtlMinutes(minutes) => Some(now_ms + minutes.saturating_mul(60_000)),
        VaultUnlockLease::KernelShutdown => None,
    };
    Ok(ArrobaVaultUnlockStatus {
        path,
        unlocked: true,
        expires_at_ms: vault.expires_at_ms,
    })
}

pub fn arroba_encrypted_vault_status(
    path: impl AsRef<Path>,
) -> Result<ArrobaVaultUnlockStatus, DaemonError> {
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    let mut unlocked = unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Arroba vault unlock state poisoned: {error}")))?;
    let now_ms = crate::session::unix_epoch_ms();
    let (is_unlocked, expires_at_ms) = match unlocked.get(&path) {
        Some(vault) if !vault.is_expired(now_ms) => (true, vault.expires_at_ms),
        Some(_) => {
            unlocked.remove(&path);
            (false, None)
        }
        None => (false, None),
    };
    Ok(ArrobaVaultUnlockStatus {
        path,
        unlocked: is_unlocked,
        expires_at_ms,
    })
}

pub fn clear_all_arroba_encrypted_vault_unlocks() -> Result<(), DaemonError> {
    unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Arroba vault unlock state poisoned: {error}")))?
        .clear();
    Ok(())
}

pub fn is_arroba_vault_locked_error(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::LocalTransport { operation, .. } if *operation == "credential_vault_locked"
    )
}

fn unlocked_vault_key(path: &Path) -> Result<[u8; KEY_LEN], DaemonError> {
    let path = normalize_vault_path(path.to_path_buf());
    let now_ms = crate::session::unix_epoch_ms();
    let mut unlocked = unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Arroba vault unlock state poisoned: {error}")))?;
    match unlocked.get(&path) {
        Some(vault) if !vault.is_expired(now_ms) => Ok(vault.key),
        Some(_) => {
            unlocked.remove(&path);
            Err(vault_locked_error(&path))
        }
        None => Err(vault_locked_error(&path)),
    }
}

fn read_vault_plaintext(path: &Path, key: &[u8; KEY_LEN]) -> Result<VaultPlaintext, DaemonError> {
    let file = read_vault_file(path)?;
    decrypt_vault_payload(&file, key)
}

fn write_vault_plaintext(
    path: &Path,
    key: &[u8; KEY_LEN],
    plaintext: &VaultPlaintext,
    kdf_profile: &VaultKdfProfile,
) -> Result<(), DaemonError> {
    let kdf = if path.exists() {
        read_vault_file(path)?.kdf
    } else {
        kdf_profile.new_kdf_config()
    };
    write_vault_file(path, key, plaintext, kdf)
}

fn read_vault_file(path: &Path) -> Result<EncryptedVaultFile, DaemonError> {
    let bytes = fs::read(path).map_err(|error| {
        secret_error(format!(
            "failed to read Arroba vault `{}`: {error}",
            path.display()
        ))
    })?;
    let file = serde_json::from_slice::<EncryptedVaultFile>(&bytes).map_err(|error| {
        secret_error(format!(
            "failed to parse Arroba vault `{}`: {error}",
            path.display()
        ))
    })?;
    validate_vault_file(&file)?;
    Ok(file)
}

fn write_vault_file(
    path: &Path,
    key: &[u8; KEY_LEN],
    plaintext: &VaultPlaintext,
    kdf: VaultKdfConfig,
) -> Result<(), DaemonError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            secret_error(format!(
                "failed to create Arroba vault directory `{}`: {error}",
                parent.display()
            ))
        })?;
    }
    let nonce = random_bytes::<NONCE_LEN>();
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|error| {
        secret_error(format!("failed to initialize Arroba vault cipher: {error}"))
    })?;
    let plaintext_bytes = serde_json::to_vec(plaintext).map_err(|error| {
        secret_error(format!(
            "failed to serialize Arroba vault plaintext: {error}"
        ))
    })?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext_bytes.as_slice())
        .map_err(|error| secret_error(format!("failed to encrypt Arroba vault: {error}")))?;
    let file = EncryptedVaultFile {
        version: VAULT_FILE_VERSION,
        kdf,
        cipher: VAULT_CIPHER.to_string(),
        nonce: base64_encode(&nonce),
        ciphertext: base64_encode(&ciphertext),
    };
    let serialized = serde_json::to_vec_pretty(&file)
        .map_err(|error| secret_error(format!("failed to serialize Arroba vault: {error}")))?;
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, serialized).map_err(|error| {
        secret_error(format!(
            "failed to write Arroba vault temp file `{}`: {error}",
            tmp_path.display()
        ))
    })?;
    fs::rename(&tmp_path, path).map_err(|error| {
        secret_error(format!(
            "failed to replace Arroba vault `{}`: {error}",
            path.display()
        ))
    })?;
    Ok(())
}

fn decrypt_vault_payload(
    file: &EncryptedVaultFile,
    key: &[u8; KEY_LEN],
) -> Result<VaultPlaintext, DaemonError> {
    validate_vault_file(file)?;
    let nonce = base64_decode_fixed::<NONCE_LEN>(&file.nonce, "nonce")?;
    let ciphertext = base64_decode(&file.ciphertext, "ciphertext")?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|error| {
        secret_error(format!("failed to initialize Arroba vault cipher: {error}"))
    })?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_slice())
        .map_err(|_| secret_error("failed to unlock Arroba vault; passphrase may be incorrect or the vault is corrupted".to_string()))?;
    serde_json::from_slice::<VaultPlaintext>(&plaintext)
        .map_err(|error| secret_error(format!("failed to decode Arroba vault plaintext: {error}")))
}

fn validate_vault_file(file: &EncryptedVaultFile) -> Result<(), DaemonError> {
    if file.version != VAULT_FILE_VERSION {
        return Err(secret_error(format!(
            "unsupported Arroba vault version {}",
            file.version
        )));
    }
    if file.cipher != VAULT_CIPHER {
        return Err(secret_error(format!(
            "unsupported Arroba vault cipher `{}`",
            file.cipher
        )));
    }
    if file.kdf.algorithm != VAULT_KDF {
        return Err(secret_error(format!(
            "unsupported Arroba vault KDF `{}`",
            file.kdf.algorithm
        )));
    }
    Ok(())
}

fn derive_key(passphrase: &str, kdf: &VaultKdfConfig) -> Result<[u8; KEY_LEN], DaemonError> {
    let salt = base64_decode(&kdf.salt, "salt")?;
    let params = Params::new(
        kdf.memory_kib,
        kdf.iterations,
        kdf.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|error| secret_error(format!("invalid Arroba vault KDF params: {error}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; KEY_LEN];
    argon2
        .hash_password_into(passphrase.as_bytes(), &salt, &mut key)
        .map_err(|error| secret_error(format!("failed to derive Arroba vault key: {error}")))?;
    Ok(key)
}

fn base64_decode(value: &str, label: &'static str) -> Result<Vec<u8>, DaemonError> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| secret_error(format!("invalid Arroba vault {label}: {error}")))
}

fn base64_decode_fixed<const N: usize>(
    value: &str,
    label: &'static str,
) -> Result<[u8; N], DaemonError> {
    let bytes = base64_decode(value, label)?;
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        secret_error(format!(
            "invalid Arroba vault {label} length: expected {N}, got {}",
            bytes.len()
        ))
    })
}

fn base64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

fn validate_passphrase(passphrase: &str) -> Result<(), DaemonError> {
    if passphrase.is_empty() {
        return Err(secret_error(
            "Arroba vault passphrase must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn process_memory_vault_backend_allowed() -> bool {
    std::env::var("ARROBA_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        || std::env::var_os("ARROBA_SLICE_MACHINE_ID").is_some()
}

fn process_memory_vault() -> &'static Mutex<BTreeMap<(String, String), String>> {
    static VAULT: OnceLock<Mutex<BTreeMap<(String, String), String>>> = OnceLock::new();
    VAULT.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn unlocked_vaults() -> &'static Mutex<BTreeMap<PathBuf, UnlockedVault>> {
    static VAULTS: OnceLock<Mutex<BTreeMap<PathBuf, UnlockedVault>>> = OnceLock::new();
    VAULTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn normalize_vault_path(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    if raw == "~" {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            return home;
        }
    }
    if let Some(suffix) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            return home.join(suffix);
        }
    }
    path
}

fn vault_locked_error(path: &Path) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "credential_vault_locked",
        message: format!("Arroba vault `{}` is locked", path.display()),
    }
}

fn secret_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "credential_vault",
        message,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedVaultFile {
    version: u32,
    kdf: VaultKdfConfig,
    cipher: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VaultKdfConfig {
    algorithm: String,
    salt: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

#[derive(Debug, Clone)]
struct VaultKdfProfile {
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

impl Default for VaultKdfProfile {
    fn default() -> Self {
        Self {
            memory_kib: DEFAULT_ARGON2_MEMORY_KIB,
            iterations: DEFAULT_ARGON2_ITERATIONS,
            parallelism: DEFAULT_ARGON2_PARALLELISM,
        }
    }
}

impl VaultKdfProfile {
    fn new_kdf_config(&self) -> VaultKdfConfig {
        VaultKdfConfig {
            algorithm: VAULT_KDF.to_string(),
            salt: base64_encode(&random_bytes::<SALT_LEN>()),
            memory_kib: self.memory_kib,
            iterations: self.iterations,
            parallelism: self.parallelism,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct VaultPlaintext {
    #[serde(default)]
    secrets: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone)]
struct UnlockedVault {
    key: [u8; KEY_LEN],
    expires_at_ms: Option<u64>,
}

impl UnlockedVault {
    fn is_expired(&self, now_ms: u64) -> bool {
        self.expires_at_ms
            .map(|expires_at_ms| expires_at_ms <= now_ms)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store(path: PathBuf) -> ArrobaEncryptedCredentialVaultStore {
        ArrobaEncryptedCredentialVaultStore::with_kdf_profile(
            path,
            VaultKdfProfile {
                memory_kib: 1024,
                iterations: 1,
                parallelism: 1,
            },
        )
    }

    #[test]
    fn encrypted_vault_round_trips_after_unlock() {
        let root = std::env::temp_dir().join(format!(
            "arroba-encrypted-vault-test-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let path = root.join("vault.json");
        let store = test_store(path.clone());
        unlock_arroba_encrypted_vault(
            &path,
            "correct horse battery staple",
            VaultUnlockLease::KernelShutdown,
        )
        .expect("vault should unlock");

        store
            .set_secret("arroba-test", "github-token", "secret-value")
            .expect("secret should store");
        assert_eq!(
            store
                .get_secret("arroba-test", "github-token")
                .expect("secret should read"),
            "secret-value"
        );
        let raw = fs::read_to_string(&path).expect("vault file should exist");
        assert!(!raw.contains("secret-value"));

        lock_arroba_encrypted_vault(&path).expect("vault should lock");
        assert!(is_arroba_vault_locked_error(
            &store
                .get_secret("arroba-test", "github-token")
                .expect_err("locked vault should not read")
        ));

        unlock_arroba_encrypted_vault(
            &path,
            "correct horse battery staple",
            VaultUnlockLease::KernelShutdown,
        )
        .expect("vault should unlock again");
        assert_eq!(
            store
                .get_secret("arroba-test", "github-token")
                .expect("secret should read after unlock"),
            "secret-value"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn encrypted_vault_rejects_wrong_passphrase() {
        let root = std::env::temp_dir().join(format!(
            "arroba-encrypted-vault-wrong-pass-test-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let path = root.join("vault.json");
        let store = test_store(path.clone());
        unlock_arroba_encrypted_vault(&path, "right-passphrase", VaultUnlockLease::KernelShutdown)
            .expect("vault should unlock");
        store
            .set_secret("arroba-test", "github-token", "secret-value")
            .expect("secret should store");
        lock_arroba_encrypted_vault(&path).expect("vault should lock");

        let error = unlock_arroba_encrypted_vault(
            &path,
            "wrong-passphrase",
            VaultUnlockLease::KernelShutdown,
        )
        .expect_err("wrong passphrase should fail");
        assert!(format!("{error}").contains("failed to unlock Arroba vault"));
        let _ = fs::remove_dir_all(root);
    }
}
