//! A new Vault on another kernel that opens with the passphrase of this one.
//!
//! The Vault key is the passphrase run through the KDF parameters and salt stored in the Vault file. The
//! source seals its unlocked key and those parameters to the new kernel's relay key; the new kernel writes
//! its own empty Vault under that key. Each kernel keeps its own file, and the passphrase never leaves the
//! kernel where it was typed.
use super::*;

const SEED_PURPOSE: &[u8] = b"vault-key-seed-v1";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VaultKeySeed {
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    kdf: VaultKdfConfig,
    sealed_key: chariox_relay::protocol::EncryptedRelayPayload,
}

impl std::fmt::Debug for VaultKeySeed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VaultKeySeed")
            .field("source_kernel_id", &self.source_kernel_id)
            .field("target_kernel_id", &self.target_kernel_id)
            .field("sealed_key", &"[redacted sealed key]")
            .finish()
    }
}

fn seed_aad(
    source_kernel_id: &str,
    source_key_thumbprint: &str,
    target_kernel_id: &str,
    target_key_thumbprint: &str,
    kdf: &VaultKdfConfig,
) -> Result<Vec<u8>, DaemonError> {
    let kdf = serde_json::to_vec(kdf)
        .map_err(|error| secret_error(format!("failed to encode Vault KDF: {error}")))?;
    Ok([
        b"chariox-vault-key-seed-v1".as_slice(),
        source_kernel_id.as_bytes(),
        source_key_thumbprint.as_bytes(),
        target_kernel_id.as_bytes(),
        target_key_thumbprint.as_bytes(),
        &kdf,
    ]
    .join(&0))
}

/// Seals the unlocked key of the Vault at `path`, with its KDF parameters, to `target_public_key`.
pub fn export_vault_key_seed(
    path: impl AsRef<Path>,
    source_kernel_id: &str,
    source_private_key: &str,
    target_kernel_id: &str,
    target_public_key: &str,
) -> Result<VaultKeySeed, DaemonError> {
    validate_transfer_binding(source_kernel_id, "source kernel id")?;
    validate_transfer_binding(target_kernel_id, "target kernel id")?;
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    let _read = vault_read_lock()?;
    let key = unlocked_vault_key(&path)?;
    let file = read_vault_file(&path)?;
    validate_vault_file(&file)?;
    decrypt_vault_payload(&file, key.as_ref())?;
    let source_key_thumbprint = public_key_thumbprint(
        &relay_crypto::public_key_from_private_key_base64(source_private_key)?,
    );
    let target_key_thumbprint = public_key_thumbprint(target_public_key);
    let aad = seed_aad(
        source_kernel_id,
        &source_key_thumbprint,
        target_kernel_id,
        &target_key_thumbprint,
        &file.kdf,
    )?;
    let sealed_key = relay_crypto::encrypt_payload_for_peer_bound(
        source_private_key,
        target_public_key,
        SEED_PURPOSE,
        &aad,
        key.as_ref(),
    )?;
    Ok(VaultKeySeed {
        source_kernel_id: source_kernel_id.to_string(),
        source_key_thumbprint,
        target_kernel_id: target_kernel_id.to_string(),
        target_key_thumbprint,
        kdf: file.kdf,
        sealed_key,
    })
}

/// Writes a new, empty Vault at `path` under the sealed key and leaves it unlocked for `lease`. An existing
/// Vault is never replaced.
pub fn create_vault_from_key_seed(
    path: impl AsRef<Path>,
    seed: &VaultKeySeed,
    target_kernel_id: &str,
    target_private_key: &str,
    lease: VaultUnlockLease,
) -> Result<CharioxVaultUnlockStatus, DaemonError> {
    let path = normalize_vault_path(path.as_ref().to_path_buf());
    let target_key_thumbprint = public_key_thumbprint(
        &relay_crypto::public_key_from_private_key_base64(target_private_key)?,
    );
    if seed.target_kernel_id != target_kernel_id
        || seed.target_key_thumbprint != target_key_thumbprint
        || public_key_thumbprint(&seed.sealed_key.sender_public_key) != seed.source_key_thumbprint
    {
        return Err(secret_error(
            "the Vault key was not sealed by that kernel for this one".to_string(),
        ));
    }
    let probe = EncryptedVaultFile {
        version: VAULT_FILE_VERSION,
        kdf: seed.kdf.clone(),
        cipher: VAULT_CIPHER.to_string(),
        nonce: base64_encode(&[0; NONCE_LEN]),
        ciphertext: base64_encode(&[0; AUTH_TAG_LEN]),
    };
    validate_vault_file(&probe)?;
    let aad = seed_aad(
        &seed.source_kernel_id,
        &seed.source_key_thumbprint,
        &seed.target_kernel_id,
        &seed.target_key_thumbprint,
        &seed.kdf,
    )?;
    let decrypted = relay_crypto::decrypt_payload_for_private_key_bound(
        target_private_key,
        &seed.sealed_key,
        SEED_PURPOSE,
        &aad,
    )?;
    let key: [u8; KEY_LEN] = decrypted
        .plaintext
        .as_slice()
        .try_into()
        .map_err(|_| secret_error("the sealed Vault key has an invalid length".to_string()))?;
    let key = Zeroizing::new(key);
    let _write = vault_write_lock()?;
    if path.try_exists().map_err(|error| {
        secret_error(format!(
            "failed to inspect Chariox vault `{}`: {error}",
            path.display()
        ))
    })? {
        return Err(secret_error(
            "this kernel already has a Vault; it is not replaced".to_string(),
        ));
    }
    write_vault_file(
        &path,
        key.as_ref(),
        &VaultPlaintext::default(),
        seed.kdf.clone(),
    )?;
    let expires_at_ms = unlock_expiry_ms(lease, crate::session::unix_epoch_ms());
    unlocked_vaults()
        .lock()
        .map_err(|error| secret_error(format!("Chariox vault unlock state poisoned: {error}")))?
        .insert(
            path.clone(),
            UnlockedVault {
                key,
                expires_at_ms,
                single_use: lease == VaultUnlockLease::SingleUse,
            },
        );
    Ok(CharioxVaultUnlockStatus {
        awaiting_passphrase: false,
        path,
        unlocked: true,
        expires_at_ms,
        single_use: lease == VaultUnlockLease::SingleUse,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light_vault(path: &Path, passphrase: &str) {
        create_chariox_encrypted_vault_for_test(path, passphrase).expect("source vault");
        unlock_chariox_encrypted_vault(path, passphrase, VaultUnlockLease::KernelShutdown)
            .expect("unlock source");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "chariox-key-seed-{name}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn a_new_vault_opens_with_the_passphrase_of_the_source_and_holds_nothing_of_it() {
        let dir = scratch("roundtrip");
        let (source, target) = (dir.join("source.json"), dir.join("target.json"));
        light_vault(&source, "correct horse");
        crate::secret::set_chariox_encrypted_vault_secret_for_test(
            source.clone(),
            "svc",
            "key",
            "source secret",
        )
        .expect("source secret");
        let source_private = relay_crypto::generate_private_key_base64();
        let target_private = relay_crypto::generate_private_key_base64();
        let target_public =
            relay_crypto::public_key_from_private_key_base64(&target_private).unwrap();

        let seed =
            export_vault_key_seed(&source, "source", &source_private, "target", &target_public)
                .expect("seal");
        let status = create_vault_from_key_seed(
            &target,
            &seed,
            "target",
            &target_private,
            VaultUnlockLease::KernelShutdown,
        )
        .expect("create");
        assert!(status.unlocked);
        let store = CharioxEncryptedCredentialVaultStore::new(target.clone());
        assert!(store.find_secret("svc", "key").unwrap().is_none());
        store.set_secret("svc", "own", "target secret").unwrap();

        lock_chariox_encrypted_vault(&target).unwrap();
        assert!(unlock_existing_chariox_encrypted_vault(
            &target,
            "wrong",
            VaultUnlockLease::Operation
        )
        .unwrap()
        .is_none());
        assert!(unlock_existing_chariox_encrypted_vault(
            &target,
            "correct horse",
            VaultUnlockLease::Operation
        )
        .unwrap()
        .is_some());
        assert_eq!(store.get_secret("svc", "own").unwrap(), "target secret");
        assert!(CharioxEncryptedCredentialVaultStore::new(source.clone())
            .find_secret("svc", "own")
            .unwrap()
            .is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_locked_source_seals_nothing() {
        let dir = scratch("locked");
        let source = dir.join("source.json");
        light_vault(&source, "correct horse");
        lock_chariox_encrypted_vault(&source).unwrap();
        let target_private = relay_crypto::generate_private_key_base64();
        let target_public =
            relay_crypto::public_key_from_private_key_base64(&target_private).unwrap();
        let error = export_vault_key_seed(
            &source,
            "source",
            &relay_crypto::generate_private_key_base64(),
            "target",
            &target_public,
        )
        .expect_err("locked");
        assert!(is_chariox_vault_locked_error(&error));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_seed_for_another_kernel_creates_nothing() {
        let dir = scratch("misdirected");
        let source = dir.join("source.json");
        light_vault(&source, "correct horse");
        let source_private = relay_crypto::generate_private_key_base64();
        let target_private = relay_crypto::generate_private_key_base64();
        let target_public =
            relay_crypto::public_key_from_private_key_base64(&target_private).unwrap();
        let seed =
            export_vault_key_seed(&source, "source", &source_private, "target", &target_public)
                .expect("seal");
        let other = relay_crypto::generate_private_key_base64();
        for (kernel, private) in [
            ("target", other.as_str()),
            ("other-target", target_private.as_str()),
        ] {
            let path = dir.join(format!("{kernel}.json"));
            create_vault_from_key_seed(&path, &seed, kernel, private, VaultUnlockLease::Operation)
                .expect_err("refused");
            assert!(!path.exists());
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_existing_vault_is_not_replaced() {
        let dir = scratch("existing");
        let (source, target) = (dir.join("source.json"), dir.join("target.json"));
        light_vault(&source, "correct horse");
        create_chariox_encrypted_vault_for_test(&target, "another").unwrap();
        let before = fs::read(&target).unwrap();
        let source_private = relay_crypto::generate_private_key_base64();
        let target_private = relay_crypto::generate_private_key_base64();
        let target_public =
            relay_crypto::public_key_from_private_key_base64(&target_private).unwrap();
        let seed =
            export_vault_key_seed(&source, "source", &source_private, "target", &target_public)
                .expect("seal");
        create_vault_from_key_seed(
            &target,
            &seed,
            "target",
            &target_private,
            VaultUnlockLease::Operation,
        )
        .expect_err("refused");
        assert_eq!(fs::read(&target).unwrap(), before);
        let _ = fs::remove_dir_all(dir);
    }
}
