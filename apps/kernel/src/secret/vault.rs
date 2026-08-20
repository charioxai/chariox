use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::config::{CredentialVaultBackend, UserCredentialVaultConfig};
use crate::error::DaemonError;
use crate::runtime::terminal_pairings::public_key_thumbprint;
use crate::transport::relay_crypto;

const VAULT_FILE_VERSION: u32 = 1;
const VAULT_CIPHER: &str = "aes-256-gcm";
const VAULT_KDF: &str = "argon2id";
const DEFAULT_ARGON2_MEMORY_KIB: u32 = 64 * 1024;
const DEFAULT_ARGON2_ITERATIONS: u32 = 3;
const DEFAULT_ARGON2_PARALLELISM: u32 = 1;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TRANSFERRED_VAULT_SCHEMA_VERSION: u32 = 1;
const TRANSFERRED_VAULT_KEY_PURPOSE: &[u8] = b"managed-context-vault-key-v1";
const STORED_VAULT_KEY_PURPOSE: &[u8] = b"managed-context-vault-key-at-rest-v1";
const MAX_TRANSFERRED_VAULT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRANSFERRED_VAULT_ENVELOPE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferredVaultSnapshot {
    pub schema_version: u32,
    pub context_id: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub vault_sha256: String,
    pub vault_size_bytes: u64,
    pub vault_file_base64: String,
    pub sealed_unlock_key: chariox_relay::protocol::EncryptedRelayPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferredVaultSourceBinding {
    pub context_id: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
}

impl std::fmt::Debug for TransferredVaultSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransferredVaultSnapshot")
            .field("schema_version", &self.schema_version)
            .field("context_id", &self.context_id)
            .field("source_kernel_id", &self.source_kernel_id)
            .field("source_key_thumbprint", &self.source_key_thumbprint)
            .field("target_kernel_id", &self.target_kernel_id)
            .field("target_key_thumbprint", &self.target_key_thumbprint)
            .field("vault_sha256", &self.vault_sha256)
            .field("vault_size_bytes", &self.vault_size_bytes)
            .field("vault_file_base64", &"[redacted encrypted vault]")
            .field("sealed_unlock_key", &"[redacted sealed key]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TransferredVaultKeyEnvelope {
    schema_version: u32,
    context_id: String,
    source_kernel_id: String,
    source_key_thumbprint: String,
    target_kernel_id: String,
    target_key_thumbprint: String,
    vault_sha256: String,
    target_sealed_unlock_key: chariox_relay::protocol::EncryptedRelayPayload,
}

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
pub struct CharioxVaultUnlockStatus {
    pub path: PathBuf,
    pub unlocked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CharioxEncryptedCredentialVaultStore {
    path: PathBuf,
    kdf_profile: VaultKdfProfile,
}

impl CharioxEncryptedCredentialVaultStore {
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

impl CredentialVaultStore for CharioxEncryptedCredentialVaultStore {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
        let vault_key = unlocked_vault_key(&self.path)?;
        let plaintext = read_vault_plaintext(&self.path, vault_key.as_ref())?;
        plaintext
            .secrets
            .get(service)
            .and_then(|service_secrets| service_secrets.get(key))
            .cloned()
            .ok_or_else(|| secret_error(format!("credential `{key}` not found in Chariox vault")))
    }

    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        let vault_key = unlocked_vault_key(&self.path)?;
        let mut plaintext = if self.path.exists() {
            read_vault_plaintext(&self.path, vault_key.as_ref())?
        } else {
            VaultPlaintext::default()
        };
        plaintext
            .secrets
            .entry(service.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());
        write_vault_plaintext(
            &self.path,
            vault_key.as_ref(),
            &plaintext,
            &self.kdf_profile,
        )
    }

    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
        let vault_key = unlocked_vault_key(&self.path)?;
        let mut plaintext = if self.path.exists() {
            read_vault_plaintext(&self.path, vault_key.as_ref())?
        } else {
            VaultPlaintext::default()
        };
        if let Some(service_secrets) = plaintext.secrets.get_mut(service) {
            service_secrets.remove(key);
            if service_secrets.is_empty() {
                plaintext.secrets.remove(service);
            }
        }
        write_vault_plaintext(
            &self.path,
            vault_key.as_ref(),
            &plaintext,
            &self.kdf_profile,
        )
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
            .map(|value| value.to_string())
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
            .insert(
                (service.to_string(), key.to_string()),
                Zeroizing::new(value.to_string()),
            );
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
        CredentialVaultBackend::CharioxEncrypted => Ok(Arc::new(
            CharioxEncryptedCredentialVaultStore::new(config.path.clone()),
        )),
        CredentialVaultBackend::ProcessMemory => {
            if process_memory_vault_backend_allowed() {
                Ok(Arc::new(ProcessMemoryCredentialVaultStore))
            } else {
                Err(secret_error(
                    "credential_vault.backend=process_memory is volatile and is only allowed inside Chariox slices or with CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT=1".to_string(),
                ))
            }
        }
    }
}

pub fn unlock_chariox_encrypted_vault(
    path: impl AsRef<Path>,
    passphrase: &str,
    lease: VaultUnlockLease,
) -> Result<CharioxVaultUnlockStatus, DaemonError> {
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
        decrypt_vault_payload(&file, key.as_ref())?;
        key
    } else {
        let kdf = VaultKdfProfile::default().new_kdf_config();
        let key = derive_key(passphrase, &kdf)?;
        write_vault_file(&path, key.as_ref(), &VaultPlaintext::default(), kdf)?;
        key
    };
    unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Chariox vault unlock state poisoned: {error}")))?
        .insert(path.clone(), UnlockedVault { key, expires_at_ms });
    Ok(CharioxVaultUnlockStatus {
        path,
        unlocked: true,
        expires_at_ms,
    })
}

pub fn lock_chariox_encrypted_vault(path: impl AsRef<Path>) -> Result<(), DaemonError> {
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Chariox vault unlock state poisoned: {error}")))?
        .remove(&path);
    Ok(())
}

pub fn extend_chariox_encrypted_vault(
    path: impl AsRef<Path>,
    lease: VaultUnlockLease,
) -> Result<CharioxVaultUnlockStatus, DaemonError> {
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    let mut unlocked = unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Chariox vault unlock state poisoned: {error}")))?;
    let vault = unlocked
        .get_mut(&path)
        .ok_or_else(|| vault_locked_error(&path))?;
    let now_ms = crate::session::unix_epoch_ms();
    vault.expires_at_ms = match lease {
        VaultUnlockLease::Operation => Some(now_ms),
        VaultUnlockLease::TtlMinutes(minutes) => Some(now_ms + minutes.saturating_mul(60_000)),
        VaultUnlockLease::KernelShutdown => None,
    };
    Ok(CharioxVaultUnlockStatus {
        path,
        unlocked: true,
        expires_at_ms: vault.expires_at_ms,
    })
}

pub fn chariox_encrypted_vault_status(
    path: impl AsRef<Path>,
) -> Result<CharioxVaultUnlockStatus, DaemonError> {
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    let mut unlocked = unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Chariox vault unlock state poisoned: {error}")))?;
    let now_ms = crate::session::unix_epoch_ms();
    let (is_unlocked, expires_at_ms) = match unlocked.get(&path) {
        Some(vault) if !vault.is_expired(now_ms) => (true, vault.expires_at_ms),
        Some(_) => {
            unlocked.remove(&path);
            (false, None)
        }
        None => (false, None),
    };
    Ok(CharioxVaultUnlockStatus {
        path,
        unlocked: is_unlocked,
        expires_at_ms,
    })
}

pub fn clear_all_chariox_encrypted_vault_unlocks() -> Result<(), DaemonError> {
    unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Chariox vault unlock state poisoned: {error}")))?
        .clear();
    Ok(())
}

pub fn export_transferred_vault_snapshot(
    path: impl AsRef<Path>,
    context_id: &str,
    source_kernel_id: &str,
    source_private_key: &str,
    target_kernel_id: &str,
    target_public_key: &str,
) -> Result<TransferredVaultSnapshot, DaemonError> {
    validate_transfer_binding(context_id, "context id")?;
    validate_transfer_binding(source_kernel_id, "source kernel id")?;
    validate_transfer_binding(target_kernel_id, "target kernel id")?;
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    let vault_bytes = read_bounded_regular_file(&path, MAX_TRANSFERRED_VAULT_BYTES)?;
    let vault_file = serde_json::from_slice::<EncryptedVaultFile>(&vault_bytes)
        .map_err(|error| secret_error(format!("failed to parse Chariox vault: {error}")))?;
    validate_vault_file(&vault_file)?;
    let vault_key = unlocked_vault_key(&path)?;
    decrypt_vault_payload(&vault_file, vault_key.as_ref())?;

    let source_public_key = relay_crypto::public_key_from_private_key_base64(source_private_key)?;
    let source_key_thumbprint = public_key_thumbprint(&source_public_key);
    let target_key_thumbprint = public_key_thumbprint(target_public_key);
    let vault_sha256 = sha256_hex(&vault_bytes);
    let aad = transferred_vault_aad(
        context_id,
        source_kernel_id,
        &source_key_thumbprint,
        target_kernel_id,
        &target_key_thumbprint,
        &vault_sha256,
    );
    let sealed_unlock_key = relay_crypto::encrypt_payload_for_peer_bound(
        source_private_key,
        target_public_key,
        TRANSFERRED_VAULT_KEY_PURPOSE,
        &aad,
        vault_key.as_ref(),
    )?;
    Ok(TransferredVaultSnapshot {
        schema_version: TRANSFERRED_VAULT_SCHEMA_VERSION,
        context_id: context_id.to_string(),
        source_kernel_id: source_kernel_id.to_string(),
        source_key_thumbprint,
        target_kernel_id: target_kernel_id.to_string(),
        target_key_thumbprint,
        vault_sha256,
        vault_size_bytes: vault_bytes.len() as u64,
        vault_file_base64: base64_encode(&vault_bytes),
        sealed_unlock_key,
    })
}

pub fn install_transferred_vault_snapshot(
    path: impl AsRef<Path>,
    snapshot: &TransferredVaultSnapshot,
    expected_source: &TransferredVaultSourceBinding,
    target_kernel_id: &str,
    target_private_key: &str,
) -> Result<(), DaemonError> {
    validate_transferred_vault_snapshot(
        snapshot,
        Some(expected_source),
        target_kernel_id,
        target_private_key,
    )?;
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    let vault_bytes = decode_transferred_vault_bytes(snapshot)?;
    let key = unseal_transferred_vault_key(
        snapshot.schema_version,
        &snapshot.context_id,
        &snapshot.source_kernel_id,
        &snapshot.source_key_thumbprint,
        &snapshot.target_kernel_id,
        &snapshot.target_key_thumbprint,
        &snapshot.vault_sha256,
        &snapshot.sealed_unlock_key,
        target_private_key,
        TRANSFERRED_VAULT_KEY_PURPOSE,
    )?;
    validate_transferred_vault_plaintext(&vault_bytes, key.as_ref())?;
    let target_public_key = relay_crypto::public_key_from_private_key_base64(target_private_key)?;
    let target_sealed_unlock_key = relay_crypto::encrypt_payload_for_peer_bound(
        target_private_key,
        &target_public_key,
        STORED_VAULT_KEY_PURPOSE,
        &transferred_vault_aad(
            &snapshot.context_id,
            &snapshot.source_kernel_id,
            &snapshot.source_key_thumbprint,
            &snapshot.target_kernel_id,
            &snapshot.target_key_thumbprint,
            &snapshot.vault_sha256,
        ),
        key.as_ref(),
    )?;
    let envelope = TransferredVaultKeyEnvelope {
        schema_version: snapshot.schema_version,
        context_id: snapshot.context_id.clone(),
        source_kernel_id: snapshot.source_kernel_id.clone(),
        source_key_thumbprint: snapshot.source_key_thumbprint.clone(),
        target_kernel_id: snapshot.target_kernel_id.clone(),
        target_key_thumbprint: snapshot.target_key_thumbprint.clone(),
        vault_sha256: snapshot.vault_sha256.clone(),
        target_sealed_unlock_key,
    };
    let envelope_bytes = serde_json::to_vec(&envelope).map_err(|error| {
        secret_error(format!(
            "failed to serialize transferred Vault key envelope: {error}"
        ))
    })?;
    if envelope_bytes.len() as u64 > MAX_TRANSFERRED_VAULT_ENVELOPE_BYTES {
        return Err(secret_error(
            "transferred Vault key envelope exceeds its size limit".to_string(),
        ));
    }
    let envelope_path = transferred_vault_envelope_path(&path, target_kernel_id);
    cleanup_private_staging(&path, target_kernel_id)?;
    cleanup_private_staging(&envelope_path, target_kernel_id)?;
    ensure_vault_destination_compatible(&path, &vault_bytes, &snapshot.vault_sha256)?;
    ensure_envelope_destination_compatible(&envelope_path, &envelope_bytes)?;
    install_vault_bytes_no_clobber(
        &path,
        &vault_bytes,
        &snapshot.vault_sha256,
        target_kernel_id,
    )?;
    install_envelope_no_clobber(&envelope_path, &envelope_bytes, target_kernel_id)?;
    remember_transferred_vault_key(path, key)
}

pub fn restore_transferred_vault_unlock(
    path: impl AsRef<Path>,
    target_kernel_id: &str,
    target_private_key: &str,
) -> Result<bool, DaemonError> {
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    let envelope_path = transferred_vault_envelope_path(&path, target_kernel_id);
    cleanup_private_staging(&path, target_kernel_id)?;
    cleanup_private_staging(&envelope_path, target_kernel_id)?;
    if !envelope_path.exists() {
        return Ok(false);
    }
    let envelope_bytes =
        read_bounded_regular_file(&envelope_path, MAX_TRANSFERRED_VAULT_ENVELOPE_BYTES)?;
    let envelope = serde_json::from_slice::<TransferredVaultKeyEnvelope>(&envelope_bytes).map_err(
        |error| {
            secret_error(format!(
                "failed to parse transferred Vault key envelope: {error}"
            ))
        },
    )?;
    validate_transfer_envelope(&envelope, target_kernel_id, target_private_key)?;
    let vault_bytes = read_bounded_regular_file(&path, MAX_TRANSFERRED_VAULT_BYTES)?;
    let key = unseal_transferred_vault_key(
        envelope.schema_version,
        &envelope.context_id,
        &envelope.source_kernel_id,
        &envelope.source_key_thumbprint,
        &envelope.target_kernel_id,
        &envelope.target_key_thumbprint,
        &envelope.vault_sha256,
        &envelope.target_sealed_unlock_key,
        target_private_key,
        STORED_VAULT_KEY_PURPOSE,
    )?;
    validate_transferred_vault_plaintext(&vault_bytes, key.as_ref())?;
    remember_transferred_vault_key(path, key)?;
    Ok(true)
}

pub fn is_chariox_vault_locked_error(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::LocalTransport { operation, .. } if *operation == "credential_vault_locked"
    )
}

fn unlocked_vault_key(path: &Path) -> Result<Zeroizing<[u8; KEY_LEN]>, DaemonError> {
    let path = normalize_vault_path(path.to_path_buf());
    let now_ms = crate::session::unix_epoch_ms();
    let mut unlocked = unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Chariox vault unlock state poisoned: {error}")))?;
    match unlocked.get(&path) {
        Some(vault) if !vault.is_expired(now_ms) => Ok(vault.key.clone()),
        Some(_) => {
            unlocked.remove(&path);
            Err(vault_locked_error(&path))
        }
        None => Err(vault_locked_error(&path)),
    }
}

fn read_vault_plaintext(path: &Path, key: &[u8]) -> Result<VaultPlaintext, DaemonError> {
    let file = read_vault_file(path)?;
    decrypt_vault_payload(&file, key)
}

fn write_vault_plaintext(
    path: &Path,
    key: &[u8],
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
            "failed to read Chariox vault `{}`: {error}",
            path.display()
        ))
    })?;
    let file = serde_json::from_slice::<EncryptedVaultFile>(&bytes).map_err(|error| {
        secret_error(format!(
            "failed to parse Chariox vault `{}`: {error}",
            path.display()
        ))
    })?;
    validate_vault_file(&file)?;
    Ok(file)
}

fn write_vault_file(
    path: &Path,
    key: &[u8],
    plaintext: &VaultPlaintext,
    kdf: VaultKdfConfig,
) -> Result<(), DaemonError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            secret_error(format!(
                "failed to create Chariox vault directory `{}`: {error}",
                parent.display()
            ))
        })?;
    }
    let nonce = random_bytes::<NONCE_LEN>();
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|error| {
        secret_error(format!(
            "failed to initialize Chariox vault cipher: {error}"
        ))
    })?;
    let plaintext_bytes = serde_json::to_vec(plaintext).map_err(|error| {
        secret_error(format!(
            "failed to serialize Chariox vault plaintext: {error}"
        ))
    })?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext_bytes.as_slice())
        .map_err(|error| secret_error(format!("failed to encrypt Chariox vault: {error}")))?;
    let file = EncryptedVaultFile {
        version: VAULT_FILE_VERSION,
        kdf,
        cipher: VAULT_CIPHER.to_string(),
        nonce: base64_encode(&nonce),
        ciphertext: base64_encode(&ciphertext),
    };
    let serialized = serde_json::to_vec_pretty(&file)
        .map_err(|error| secret_error(format!("failed to serialize Chariox vault: {error}")))?;
    let tmp_path = vault_temp_path(path);
    let mut tmp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .map_err(|error| {
            secret_error(format!(
                "failed to create Chariox vault temp file `{}`: {error}",
                tmp_path.display()
            ))
        })?;
    set_vault_file_permissions(&tmp_file)?;
    tmp_file.write_all(&serialized).map_err(|error| {
        secret_error(format!(
            "failed to write Chariox vault temp file `{}`: {error}",
            tmp_path.display()
        ))
    })?;
    tmp_file.sync_all().map_err(|error| {
        secret_error(format!(
            "failed to sync Chariox vault temp file `{}`: {error}",
            tmp_path.display()
        ))
    })?;
    drop(tmp_file);
    fs::rename(&tmp_path, path).map_err(|error| {
        secret_error(format!(
            "failed to replace Chariox vault `{}`: {error}",
            path.display()
        ))
    })?;
    sync_vault_parent_dir(path)?;
    Ok(())
}

fn decrypt_vault_payload(
    file: &EncryptedVaultFile,
    key: &[u8],
) -> Result<VaultPlaintext, DaemonError> {
    validate_vault_file(file)?;
    let nonce = base64_decode_fixed::<NONCE_LEN>(&file.nonce, "nonce")?;
    let ciphertext = base64_decode(&file.ciphertext, "ciphertext")?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|error| {
        secret_error(format!(
            "failed to initialize Chariox vault cipher: {error}"
        ))
    })?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_slice())
        .map_err(|_| secret_error("failed to unlock Chariox vault; passphrase may be incorrect or the vault is corrupted".to_string()))?;
    serde_json::from_slice::<VaultPlaintext>(&plaintext)
        .map_err(|error| secret_error(format!("failed to decode Chariox vault plaintext: {error}")))
}

fn validate_vault_file(file: &EncryptedVaultFile) -> Result<(), DaemonError> {
    if file.version != VAULT_FILE_VERSION {
        return Err(secret_error(format!(
            "unsupported Chariox vault version {}",
            file.version
        )));
    }
    if file.cipher != VAULT_CIPHER {
        return Err(secret_error(format!(
            "unsupported Chariox vault cipher `{}`",
            file.cipher
        )));
    }
    if file.kdf.algorithm != VAULT_KDF {
        return Err(secret_error(format!(
            "unsupported Chariox vault KDF `{}`",
            file.kdf.algorithm
        )));
    }
    Ok(())
}

fn derive_key(
    passphrase: &str,
    kdf: &VaultKdfConfig,
) -> Result<Zeroizing<[u8; KEY_LEN]>, DaemonError> {
    let salt = base64_decode(&kdf.salt, "salt")?;
    let params = Params::new(
        kdf.memory_kib,
        kdf.iterations,
        kdf.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|error| secret_error(format!("invalid Chariox vault KDF params: {error}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0_u8; KEY_LEN]);
    argon2
        .hash_password_into(passphrase.as_bytes(), &salt, key.as_mut())
        .map_err(|error| secret_error(format!("failed to derive Chariox vault key: {error}")))?;
    Ok(key)
}

fn validate_transferred_vault_snapshot(
    snapshot: &TransferredVaultSnapshot,
    expected_source: Option<&TransferredVaultSourceBinding>,
    target_kernel_id: &str,
    target_private_key: &str,
) -> Result<(), DaemonError> {
    if snapshot.schema_version != TRANSFERRED_VAULT_SCHEMA_VERSION {
        return Err(secret_error(format!(
            "unsupported transferred Vault snapshot version {}",
            snapshot.schema_version
        )));
    }
    validate_transfer_binding(&snapshot.context_id, "context id")?;
    validate_transfer_binding(&snapshot.source_kernel_id, "source kernel id")?;
    validate_transfer_binding(&snapshot.target_kernel_id, "target kernel id")?;
    if let Some(expected_source) = expected_source {
        validate_transfer_binding(&expected_source.context_id, "expected context id")?;
        validate_transfer_binding(
            &expected_source.source_kernel_id,
            "expected source kernel id",
        )?;
        validate_sha256(
            &expected_source.source_key_thumbprint,
            "expected source key thumbprint",
        )?;
        if snapshot.context_id != expected_source.context_id
            || snapshot.source_kernel_id != expected_source.source_kernel_id
            || snapshot.source_key_thumbprint != expected_source.source_key_thumbprint
        {
            return Err(secret_error(
                "transferred Vault source or context binding does not match".to_string(),
            ));
        }
    }
    if snapshot.target_kernel_id != target_kernel_id {
        return Err(secret_error(
            "transferred Vault target kernel does not match".to_string(),
        ));
    }
    validate_sha256(&snapshot.source_key_thumbprint, "source key thumbprint")?;
    validate_sha256(&snapshot.target_key_thumbprint, "target key thumbprint")?;
    validate_sha256(&snapshot.vault_sha256, "Vault digest")?;
    let target_public_key = relay_crypto::public_key_from_private_key_base64(target_private_key)?;
    if public_key_thumbprint(&target_public_key) != snapshot.target_key_thumbprint {
        return Err(secret_error(
            "transferred Vault target key does not match".to_string(),
        ));
    }
    if public_key_thumbprint(&snapshot.sealed_unlock_key.sender_public_key)
        != snapshot.source_key_thumbprint
    {
        return Err(secret_error(
            "transferred Vault source key does not match its sealed key".to_string(),
        ));
    }
    if snapshot.vault_size_bytes == 0 || snapshot.vault_size_bytes > MAX_TRANSFERRED_VAULT_BYTES {
        return Err(secret_error(
            "transferred Vault size is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_transfer_envelope(
    envelope: &TransferredVaultKeyEnvelope,
    target_kernel_id: &str,
    target_private_key: &str,
) -> Result<(), DaemonError> {
    if envelope.schema_version != TRANSFERRED_VAULT_SCHEMA_VERSION {
        return Err(secret_error(format!(
            "unsupported transferred Vault key envelope version {}",
            envelope.schema_version
        )));
    }
    validate_transfer_binding(&envelope.context_id, "context id")?;
    validate_transfer_binding(&envelope.source_kernel_id, "source kernel id")?;
    validate_transfer_binding(&envelope.target_kernel_id, "target kernel id")?;
    validate_sha256(&envelope.source_key_thumbprint, "source key thumbprint")?;
    validate_sha256(&envelope.target_key_thumbprint, "target key thumbprint")?;
    validate_sha256(&envelope.vault_sha256, "Vault digest")?;
    if envelope.target_kernel_id != target_kernel_id {
        return Err(secret_error(
            "stored transferred Vault target kernel does not match".to_string(),
        ));
    }
    let target_public_key = relay_crypto::public_key_from_private_key_base64(target_private_key)?;
    if public_key_thumbprint(&target_public_key) != envelope.target_key_thumbprint
        || public_key_thumbprint(&envelope.target_sealed_unlock_key.sender_public_key)
            != envelope.target_key_thumbprint
    {
        return Err(secret_error(
            "stored transferred Vault target sealing key does not match".to_string(),
        ));
    }
    Ok(())
}

fn decode_transferred_vault_bytes(
    snapshot: &TransferredVaultSnapshot,
) -> Result<Vec<u8>, DaemonError> {
    let maximum_base64_bytes = MAX_TRANSFERRED_VAULT_BYTES
        .saturating_add(2)
        .saturating_div(3)
        .saturating_mul(4);
    if snapshot.vault_file_base64.len() as u64 > maximum_base64_bytes {
        return Err(secret_error(
            "transferred Vault encoding exceeds its size limit".to_string(),
        ));
    }
    let bytes = base64_decode(&snapshot.vault_file_base64, "transferred Vault file")?;
    if bytes.len() as u64 != snapshot.vault_size_bytes
        || bytes.len() as u64 > MAX_TRANSFERRED_VAULT_BYTES
    {
        return Err(secret_error(
            "transferred Vault bytes do not match the declared size".to_string(),
        ));
    }
    if sha256_hex(&bytes) != snapshot.vault_sha256 {
        return Err(secret_error(
            "transferred Vault bytes do not match the declared digest".to_string(),
        ));
    }
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
fn unseal_transferred_vault_key(
    schema_version: u32,
    context_id: &str,
    source_kernel_id: &str,
    source_key_thumbprint: &str,
    target_kernel_id: &str,
    target_key_thumbprint: &str,
    vault_sha256: &str,
    sealed_unlock_key: &chariox_relay::protocol::EncryptedRelayPayload,
    target_private_key: &str,
    purpose: &[u8],
) -> Result<Zeroizing<[u8; KEY_LEN]>, DaemonError> {
    if schema_version != TRANSFERRED_VAULT_SCHEMA_VERSION {
        return Err(secret_error(
            "unsupported transferred Vault key envelope version".to_string(),
        ));
    }
    let aad = transferred_vault_aad(
        context_id,
        source_kernel_id,
        source_key_thumbprint,
        target_kernel_id,
        target_key_thumbprint,
        vault_sha256,
    );
    let decrypted = relay_crypto::decrypt_payload_for_private_key_bound(
        target_private_key,
        sealed_unlock_key,
        purpose,
        &aad,
    )?;
    let plaintext = Zeroizing::new(decrypted.plaintext);
    let key: [u8; KEY_LEN] = plaintext.as_slice().try_into().map_err(|_| {
        secret_error("transferred Vault unlock key has an invalid length".to_string())
    })?;
    Ok(Zeroizing::new(key))
}

fn transferred_vault_aad(
    context_id: &str,
    source_kernel_id: &str,
    source_key_thumbprint: &str,
    target_kernel_id: &str,
    target_key_thumbprint: &str,
    vault_sha256: &str,
) -> Vec<u8> {
    [
        b"chariox-managed-context-vault-v1".as_slice(),
        context_id.as_bytes(),
        source_kernel_id.as_bytes(),
        source_key_thumbprint.as_bytes(),
        target_kernel_id.as_bytes(),
        target_key_thumbprint.as_bytes(),
        vault_sha256.as_bytes(),
    ]
    .join(&0)
}

fn validate_transferred_vault_plaintext(vault_bytes: &[u8], key: &[u8]) -> Result<(), DaemonError> {
    let file = serde_json::from_slice::<EncryptedVaultFile>(vault_bytes)
        .map_err(|error| secret_error(format!("failed to parse transferred Vault: {error}")))?;
    decrypt_vault_payload(&file, key).map(|_| ())
}

fn install_vault_bytes_no_clobber(
    path: &Path,
    vault_bytes: &[u8],
    expected_sha256: &str,
    target_kernel_id: &str,
) -> Result<(), DaemonError> {
    if ensure_vault_destination_compatible(path, vault_bytes, expected_sha256)? {
        return Ok(());
    }
    write_private_file_no_clobber(path, vault_bytes, target_kernel_id)
}

fn install_envelope_no_clobber(
    path: &Path,
    envelope_bytes: &[u8],
    target_kernel_id: &str,
) -> Result<(), DaemonError> {
    if ensure_envelope_destination_compatible(path, envelope_bytes)? {
        return Ok(());
    }
    write_private_file_no_clobber(path, envelope_bytes, target_kernel_id)
}

fn ensure_vault_destination_compatible(
    path: &Path,
    vault_bytes: &[u8],
    expected_sha256: &str,
) -> Result<bool, DaemonError> {
    if !path.exists() {
        return Ok(false);
    }
    let existing = read_bounded_regular_file(path, MAX_TRANSFERRED_VAULT_BYTES)?;
    if sha256_hex(&existing) == expected_sha256 && existing == vault_bytes {
        return Ok(true);
    }
    Err(secret_error(
        "refusing to replace an existing target Vault with transferred context".to_string(),
    ))
}

fn ensure_envelope_destination_compatible(
    path: &Path,
    envelope_bytes: &[u8],
) -> Result<bool, DaemonError> {
    if !path.exists() {
        return Ok(false);
    }
    let existing = read_bounded_regular_file(path, MAX_TRANSFERRED_VAULT_ENVELOPE_BYTES)?;
    if existing == envelope_bytes {
        return Ok(true);
    }
    Err(secret_error(
        "refusing to replace an existing transferred Vault key envelope".to_string(),
    ))
}

fn write_private_file_no_clobber(
    path: &Path,
    bytes: &[u8],
    target_kernel_id: &str,
) -> Result<(), DaemonError> {
    let parent = path
        .parent()
        .ok_or_else(|| secret_error("transferred Vault destination has no parent".to_string()))?;
    fs::create_dir_all(parent).map_err(|error| {
        secret_error(format!(
            "failed to create transferred Vault destination: {error}"
        ))
    })?;
    let temporary = private_staging_path(path, target_kernel_id)?;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|error| {
            secret_error(format!(
                "failed to stage transferred Vault material: {error}"
            ))
        })?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                secret_error(format!(
                    "failed to stage transferred Vault material: {error}"
                ))
            })?;
        fs::hard_link(&temporary, path).map_err(|error| {
            secret_error(format!(
                "failed to publish transferred Vault material: {error}"
            ))
        })?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                let _ = fs::remove_file(path);
                secret_error(format!(
                    "failed to sync transferred Vault material: {error}"
                ))
            })?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn cleanup_private_staging(path: &Path, target_kernel_id: &str) -> Result<(), DaemonError> {
    let temporary = private_staging_path(path, target_kernel_id)?;
    let metadata = match fs::symlink_metadata(&temporary) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(secret_error(format!(
                "failed to inspect transferred Vault staging: {error}"
            )))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(secret_error(
            "transferred Vault staging must be a regular file".to_string(),
        ));
    }
    fs::remove_file(&temporary).map_err(|error| {
        secret_error(format!(
            "failed to remove transferred Vault staging: {error}"
        ))
    })
}

fn private_staging_path(path: &Path, target_kernel_id: &str) -> Result<PathBuf, DaemonError> {
    let parent = path
        .parent()
        .ok_or_else(|| secret_error("transferred Vault destination has no parent".to_string()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| secret_error("transferred Vault destination name is invalid".to_string()))?;
    let target_namespace = sha256_hex(target_kernel_id.as_bytes());
    Ok(parent.join(format!(
        ".{file_name}.managed-context-{target_namespace}.tmp"
    )))
}

fn remember_transferred_vault_key(
    path: PathBuf,
    key: Zeroizing<[u8; KEY_LEN]>,
) -> Result<(), DaemonError> {
    unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Chariox vault unlock state poisoned: {error}")))?
        .insert(
            path,
            UnlockedVault {
                key,
                expires_at_ms: None,
            },
        );
    Ok(())
}

fn transferred_vault_envelope_path(path: &Path, target_kernel_id: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("vault.json");
    let target_namespace = sha256_hex(target_kernel_id.as_bytes());
    path.with_file_name(format!(
        "{file_name}.managed-context-key-{target_namespace}.json"
    ))
}

fn read_bounded_regular_file(path: &Path, maximum_bytes: u64) -> Result<Vec<u8>, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|error| secret_error(format!("failed to open Chariox Vault material: {error}")))?;
    let metadata = file.metadata().map_err(|error| {
        secret_error(format!("failed to inspect Chariox Vault material: {error}"))
    })?;
    if !metadata.is_file() || metadata.len() > maximum_bytes {
        return Err(secret_error(
            "Chariox Vault material must be a bounded regular file".to_string(),
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| secret_error(format!("failed to read Chariox Vault material: {error}")))?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(secret_error(
            "Chariox Vault material exceeds its size limit".to_string(),
        ));
    }
    Ok(bytes)
}

fn validate_transfer_binding(value: &str, label: &str) -> Result<(), DaemonError> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(secret_error(format!(
            "transferred Vault {label} is invalid"
        )));
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), DaemonError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(secret_error(format!(
            "transferred Vault {label} is invalid"
        )));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn vault_temp_path(path: &Path) -> PathBuf {
    let mut suffix = [0_u8; 8];
    rand::thread_rng().fill_bytes(&mut suffix);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("vault.json");
    path.with_file_name(format!(
        ".{file_name}.{}.tmp",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(suffix)
    ))
}

fn sync_vault_parent_dir(path: &Path) -> Result<(), DaemonError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| {
            secret_error(format!(
                "failed to sync Chariox vault directory `{}`: {error}",
                parent.display()
            ))
        })
}

#[cfg(unix)]
fn set_vault_file_permissions(file: &File) -> Result<(), DaemonError> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| secret_error(format!("failed to set Chariox vault permissions: {error}")))
}

#[cfg(not(unix))]
fn set_vault_file_permissions(_file: &File) -> Result<(), DaemonError> {
    Ok(())
}

fn base64_decode(value: &str, label: &'static str) -> Result<Vec<u8>, DaemonError> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| secret_error(format!("invalid Chariox vault {label}: {error}")))
}

fn base64_decode_fixed<const N: usize>(
    value: &str,
    label: &'static str,
) -> Result<[u8; N], DaemonError> {
    let bytes = base64_decode(value, label)?;
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        secret_error(format!(
            "invalid Chariox vault {label} length: expected {N}, got {}",
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
            "Chariox vault passphrase must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn process_memory_vault_backend_allowed() -> bool {
    std::env::var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        || std::env::var_os("CHARIOX_SLICE_MACHINE_ID").is_some()
}

fn process_memory_vault() -> &'static Mutex<BTreeMap<(String, String), Zeroizing<String>>> {
    static VAULT: OnceLock<Mutex<BTreeMap<(String, String), Zeroizing<String>>>> = OnceLock::new();
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

fn vault_locked_error(_path: &Path) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "credential_vault_locked",
        message: "Chariox vault is locked".to_string(),
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
    key: Zeroizing<[u8; KEY_LEN]>,
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

    fn test_store(path: PathBuf) -> CharioxEncryptedCredentialVaultStore {
        CharioxEncryptedCredentialVaultStore::with_kdf_profile(
            path,
            VaultKdfProfile {
                memory_kib: 1024,
                iterations: 1,
                parallelism: 1,
            },
        )
    }

    fn source_binding(snapshot: &TransferredVaultSnapshot) -> TransferredVaultSourceBinding {
        TransferredVaultSourceBinding {
            context_id: snapshot.context_id.clone(),
            source_kernel_id: snapshot.source_kernel_id.clone(),
            source_key_thumbprint: snapshot.source_key_thumbprint.clone(),
        }
    }

    fn boot_config(
        root: &Path,
        vault_path: &Path,
        daemon_id: &str,
        relay_private_key: &str,
    ) -> crate::config::DaemonConfig {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.daemon_id = daemon_id.to_string();
        config.relay_private_key = relay_private_key.to_string();
        config.relay_public_key =
            relay_crypto::public_key_from_private_key_base64(relay_private_key)
                .expect("test relay public key should derive");
        config.session_history_root = root.join(format!("{daemon_id}-sessions"));
        config.user_config.history.operational.path = Some(
            root.join(format!("{daemon_id}-operational.db"))
                .display()
                .to_string(),
        );
        config.user_config.artifacts.operational.root = Some(
            root.join(format!("{daemon_id}-artifacts"))
                .display()
                .to_string(),
        );
        config.user_config.artifacts.operational.index_path = Some(
            root.join(format!("{daemon_id}-artifacts.db"))
                .display()
                .to_string(),
        );
        config.user_config.state.path = Some(
            root.join(format!("{daemon_id}-state"))
                .join("state.db")
                .display()
                .to_string(),
        );
        config.user_config.credential_vault.path = vault_path.display().to_string();
        config
    }

    #[test]
    fn encrypted_vault_round_trips_after_unlock() {
        let root = std::env::temp_dir().join(format!(
            "chariox-encrypted-vault-test-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let path = root.join("vault.json");
        let store = test_store(path.clone());
        unlock_chariox_encrypted_vault(
            &path,
            "correct horse battery staple",
            VaultUnlockLease::KernelShutdown,
        )
        .expect("vault should unlock");

        store
            .set_secret("chariox-test", "github-token", "secret-value")
            .expect("secret should store");
        assert_eq!(
            store
                .get_secret("chariox-test", "github-token")
                .expect("secret should read"),
            "secret-value"
        );
        let raw = fs::read_to_string(&path).expect("vault file should exist");
        assert!(!raw.contains("secret-value"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path)
                .expect("vault metadata should read")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        lock_chariox_encrypted_vault(&path).expect("vault should lock");
        let locked_error = store
            .get_secret("chariox-test", "github-token")
            .expect_err("locked vault should not read");
        assert!(is_chariox_vault_locked_error(&locked_error));
        assert!(!locked_error
            .to_string()
            .contains(&path.display().to_string()));

        unlock_chariox_encrypted_vault(
            &path,
            "correct horse battery staple",
            VaultUnlockLease::KernelShutdown,
        )
        .expect("vault should unlock again");
        assert_eq!(
            store
                .get_secret("chariox-test", "github-token")
                .expect("secret should read after unlock"),
            "secret-value"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn encrypted_vault_rejects_wrong_passphrase() {
        let root = std::env::temp_dir().join(format!(
            "chariox-encrypted-vault-wrong-pass-test-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let path = root.join("vault.json");
        let store = test_store(path.clone());
        unlock_chariox_encrypted_vault(&path, "right-passphrase", VaultUnlockLease::KernelShutdown)
            .expect("vault should unlock");
        store
            .set_secret("chariox-test", "github-token", "secret-value")
            .expect("secret should store");
        lock_chariox_encrypted_vault(&path).expect("vault should lock");

        let error = unlock_chariox_encrypted_vault(
            &path,
            "wrong-passphrase",
            VaultUnlockLease::KernelShutdown,
        )
        .expect_err("wrong passphrase should fail");
        assert!(format!("{error}").contains("failed to unlock Chariox vault"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn transferred_vault_is_target_bound_and_reopens_after_restart() {
        let root = std::env::temp_dir().join(format!(
            "chariox-transferred-vault-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let source_path = root.join("source-vault.json");
        let target_path = root.join("target-vault.json");
        let source_store = test_store(source_path.clone());
        unlock_chariox_encrypted_vault(
            &source_path,
            "source-passphrase",
            VaultUnlockLease::KernelShutdown,
        )
        .expect("source vault should unlock");
        source_store
            .set_secret("chariox-test", "token", "vault-secret-canary")
            .expect("source secret should store");

        let source_private = relay_crypto::generate_private_key_base64();
        let target_private = relay_crypto::generate_private_key_base64();
        let target_public = relay_crypto::public_key_from_private_key_base64(&target_private)
            .expect("target public key should derive");
        let snapshot = export_transferred_vault_snapshot(
            &source_path,
            "context-one",
            "source-kernel",
            &source_private,
            "target-kernel",
            &target_public,
        )
        .expect("vault should export");
        let serialized = serde_json::to_string(&snapshot).expect("snapshot should serialize");
        assert!(!serialized.contains("vault-secret-canary"));
        assert!(!format!("{snapshot:?}").contains(&snapshot.vault_file_base64));

        let wrong_target_private = relay_crypto::generate_private_key_base64();
        let wrong_target_error = install_transferred_vault_snapshot(
            root.join("wrong-target-vault.json"),
            &snapshot,
            &source_binding(&snapshot),
            "target-kernel",
            &wrong_target_private,
        )
        .expect_err("wrong target key should reject");
        assert!(wrong_target_error
            .to_string()
            .contains("target key does not match"));

        lock_chariox_encrypted_vault(&source_path).expect("source unlock should clear");
        install_transferred_vault_snapshot(
            &target_path,
            &snapshot,
            &source_binding(&snapshot),
            "target-kernel",
            &target_private,
        )
        .expect("target vault should install");
        let target_store = test_store(target_path.clone());
        assert_eq!(
            target_store
                .get_secret("chariox-test", "token")
                .expect("installed target vault should read"),
            "vault-secret-canary"
        );

        target_store
            .set_secret("chariox-test", "rotated", "new-secret")
            .expect("transferred Vault should remain mutable");
        let vault_staging =
            private_staging_path(&target_path, "target-kernel").expect("vault staging path");
        let envelope_staging = private_staging_path(
            &transferred_vault_envelope_path(&target_path, "target-kernel"),
            "target-kernel",
        )
        .expect("envelope staging path");
        fs::write(&vault_staging, b"stale vault staging").expect("seed vault staging");
        fs::write(&envelope_staging, b"stale envelope staging").expect("seed envelope staging");
        lock_chariox_encrypted_vault(&target_path).expect("simulate kernel shutdown");
        let target_config = boot_config(
            &root.join("target-boot"),
            &target_path,
            "target-kernel",
            &target_private,
        );
        crate::app::DaemonApp::bootstrap(target_config)
            .expect("target kernel should bootstrap from the persisted envelope");
        assert!(!vault_staging.exists());
        assert!(!envelope_staging.exists());
        assert_eq!(
            target_store
                .get_secret("chariox-test", "token")
                .expect("restored target vault should read"),
            "vault-secret-canary"
        );
        assert_eq!(
            target_store
                .get_secret("chariox-test", "rotated")
                .expect("mutated target Vault should survive bootstrap"),
            "new-secret"
        );

        lock_chariox_encrypted_vault(&target_path).expect("clear target unlock");
        let other_private = relay_crypto::generate_private_key_base64();
        let other_config = boot_config(
            &root.join("other-boot"),
            &target_path,
            "other-kernel",
            &other_private,
        );
        crate::app::DaemonApp::bootstrap(other_config)
            .expect("another kernel sharing the Vault path should ignore another target envelope");
        let _ = lock_chariox_encrypted_vault(&target_path);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn transferred_vault_rejects_tampered_binding_and_existing_target() {
        let root = std::env::temp_dir().join(format!(
            "chariox-transferred-vault-tamper-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let source_path = root.join("source-vault.json");
        let source_store = test_store(source_path.clone());
        unlock_chariox_encrypted_vault(
            &source_path,
            "source-passphrase",
            VaultUnlockLease::KernelShutdown,
        )
        .expect("source vault should unlock");
        source_store
            .set_secret("chariox-test", "token", "secret")
            .expect("source secret should store");
        let source_private = relay_crypto::generate_private_key_base64();
        let target_private = relay_crypto::generate_private_key_base64();
        let target_public = relay_crypto::public_key_from_private_key_base64(&target_private)
            .expect("target public key should derive");
        let snapshot = export_transferred_vault_snapshot(
            &source_path,
            "context-one",
            "source-kernel",
            &source_private,
            "target-kernel",
            &target_public,
        )
        .expect("vault should export");

        let mut tampered = snapshot.clone();
        tampered.context_id = "context-two".to_string();
        assert!(install_transferred_vault_snapshot(
            root.join("tampered-vault.json"),
            &tampered,
            &source_binding(&snapshot),
            "target-kernel",
            &target_private,
        )
        .is_err());

        let attacker_private = relay_crypto::generate_private_key_base64();
        let attacker_snapshot = export_transferred_vault_snapshot(
            &source_path,
            "attacker-context",
            "attacker-kernel",
            &attacker_private,
            "target-kernel",
            &target_public,
        )
        .expect("self-consistent attacker snapshot should export");
        assert!(install_transferred_vault_snapshot(
            root.join("attacker-vault.json"),
            &attacker_snapshot,
            &source_binding(&snapshot),
            "target-kernel",
            &target_private,
        )
        .expect_err("authenticated source binding should reject another source")
        .to_string()
        .contains("source or context binding does not match"));

        let forged_path = root.join("forged-on-disk-vault.json");
        fs::write(
            &forged_path,
            decode_transferred_vault_bytes(&attacker_snapshot)
                .expect("attacker Vault bytes should decode"),
        )
        .expect("forged Vault should write");
        let forged_envelope = TransferredVaultKeyEnvelope {
            schema_version: attacker_snapshot.schema_version,
            context_id: attacker_snapshot.context_id.clone(),
            source_kernel_id: attacker_snapshot.source_kernel_id.clone(),
            source_key_thumbprint: attacker_snapshot.source_key_thumbprint.clone(),
            target_kernel_id: attacker_snapshot.target_kernel_id.clone(),
            target_key_thumbprint: attacker_snapshot.target_key_thumbprint.clone(),
            vault_sha256: attacker_snapshot.vault_sha256.clone(),
            target_sealed_unlock_key: attacker_snapshot.sealed_unlock_key.clone(),
        };
        fs::write(
            transferred_vault_envelope_path(&forged_path, "target-kernel"),
            serde_json::to_vec(&forged_envelope).expect("forged envelope should serialize"),
        )
        .expect("forged envelope should write");
        let forged_boot = crate::app::DaemonApp::bootstrap(boot_config(
            &root.join("forged-boot"),
            &forged_path,
            "target-kernel",
            &target_private,
        ));
        let forged_error = match forged_boot {
            Ok(_) => panic!("forged on-disk Vault envelope should not bootstrap"),
            Err(error) => error,
        };
        assert!(forged_error
            .to_string()
            .contains("target sealing key does not match"));

        let occupied = root.join("occupied-vault.json");
        fs::create_dir_all(&root).expect("test root should create");
        fs::write(&occupied, b"not the transferred vault").expect("occupied vault should write");
        assert!(install_transferred_vault_snapshot(
            &occupied,
            &snapshot,
            &source_binding(&snapshot),
            "target-kernel",
            &target_private,
        )
        .expect_err("occupied target should reject")
        .to_string()
        .contains("refusing to replace"));
        let _ = lock_chariox_encrypted_vault(&source_path);
        let _ = fs::remove_dir_all(root);
    }
}
