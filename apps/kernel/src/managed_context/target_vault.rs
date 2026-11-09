//! A transfer that carries Vault information starts only when the receiving kernel's Vault is open.
use crate::config::{CredentialVaultBackend, DaemonConfig};
use crate::error::DaemonError;

/// The reason class clients read when the receiving Vault is closed.
pub(crate) const TARGET_VAULT_LOCKED: &str = "target_vault_locked";

pub(crate) fn target_vault_is_open(config: &DaemonConfig) -> Result<bool, DaemonError> {
    let vault = &config.user_config.credential_vault;
    match vault.backend {
        CredentialVaultBackend::CharioxEncrypted => {
            Ok(crate::secret::chariox_encrypted_vault_status(&vault.path)?.unlocked)
        }
        CredentialVaultBackend::ProcessMemory => Ok(true),
    }
}

/// Refuses the whole import while the receiving Vault is closed; nothing has been written yet.
pub(crate) fn require_open_target_vault(config: &DaemonConfig) -> Result<(), DaemonError> {
    if target_vault_is_open(config)? {
        Ok(())
    } else {
        Err(target_vault_locked())
    }
}

pub(crate) fn target_vault_locked() -> DaemonError {
    DaemonError::ManagedContext {
        code: TARGET_VAULT_LOCKED,
        operation: "import managed context",
        message: "the receiving kernel's Vault is locked; unlock it and try again".into(),
        retryable: true,
    }
}

/// Importing named Secret entries writes them into this kernel's own Vault: refuse the whole
/// import, before any entry lands, while that Vault is closed. An Update names nothing; its
/// selection comes from the source's record, so `run_import` makes the same refusal once it has it.
pub(crate) fn require_open_vault_for_secret_entries(
    config: &DaemonConfig,
    entries: Option<&[crate::local::EnvironmentEntryRef]>,
) -> Result<(), DaemonError> {
    let names_a_secret = entries.is_some_and(|entries| {
        entries
            .iter()
            .any(|entry| entry.kind == crate::local::EnvironmentEntryKind::Secret)
    });
    if names_a_secret && !target_vault_is_open(config)? {
        return Err(secret_import_refused());
    }
    Ok(())
}

pub(crate) fn secret_import_refused() -> DaemonError {
    DaemonError::VaultUnlock {
        code: TARGET_VAULT_LOCKED,
        message: "this kernel's Vault is locked; unlock it and try again".into(),
        retryable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{EnvironmentEntryKind, EnvironmentEntryRef};

    const PASSPHRASE: &str = "target-vault-test-passphrase";

    fn kernel(label: &str) -> (DaemonConfig, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "target-vault-{label}-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut config = DaemonConfig::new(label, label, "owner");
        config.user_config.credential_vault.backend = CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path =
            root.join("vault.json").to_string_lossy().into_owned();
        (config, root)
    }

    fn entry(kind: EnvironmentEntryKind, name: &str) -> EnvironmentEntryRef {
        EnvironmentEntryRef {
            kind,
            name: name.into(),
        }
    }

    #[test]
    fn a_selected_secret_is_refused_while_the_receiving_vault_is_closed_and_allowed_after_unlock() {
        let (config, root) = kernel("env-secret");
        let entries = [
            entry(EnvironmentEntryKind::File, ".env.example"),
            entry(EnvironmentEntryKind::Secret, "API_KEY"),
        ];

        let error = require_open_vault_for_secret_entries(&config, Some(&entries))
            .expect_err("the Vault is not open");
        assert!(
            matches!(
                &error,
                DaemonError::VaultUnlock {
                    code: "target_vault_locked",
                    retryable: true,
                    ..
                }
            ),
            "{error:?}"
        );

        crate::secret::unlock_chariox_encrypted_vault(
            &config.user_config.credential_vault.path,
            PASSPHRASE,
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        require_open_vault_for_secret_entries(&config, Some(&entries))
            .expect("the same request starts once the Vault is open");
        crate::secret::lock_chariox_encrypted_vault(&config.user_config.credential_vault.path)
            .unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_import_without_a_secret_does_not_need_the_vault() {
        let (config, root) = kernel("env-files");
        let entries = [
            entry(EnvironmentEntryKind::File, ".env.example"),
            entry(EnvironmentEntryKind::Mcp, "docs"),
        ];
        require_open_vault_for_secret_entries(&config, Some(&entries)).unwrap();
        require_open_vault_for_secret_entries(&config, None).unwrap();
        let _ = std::fs::remove_dir_all(root);
    }
}
