use zeroize::Zeroizing;

use super::runtime_vault_unlock_state::{expand_vault_path, vault_unlock_request_lock};
use super::{ClaudeSetupTokenVaultPrompt, KernelRuntimeState};
use crate::config::{CredentialVaultBackend, CredentialVaultUnlockPolicy, DaemonConfig};
use crate::error::DaemonError;
use crate::secret::VaultUnlockLease;

#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime) enum ClaudeSetupTokenStoreOutcome {
    Stored,
    VaultPassphraseRequired(ClaudeSetupTokenVaultPrompt),
    /// The passphrase did not unlock the vault; nothing was stored.
    VaultUnlockFailed(String),
}

impl KernelRuntimeState {
    /// Stores a captured `claude setup-token` credential through the same path
    /// as `provider setup-token`. A locked encrypted vault is unlocked only with
    /// a passphrase entered in the provider login workflow itself, never
    /// through an agent runtime interaction.
    pub(in crate::runtime) async fn store_claude_setup_token(
        &self,
        owner_user_id: &str,
        account_profile: &str,
        token: Zeroizing<String>,
        passphrase: Option<Zeroizing<String>>,
    ) -> Result<ClaudeSetupTokenStoreOutcome, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        // Launches unlock and relock the same vault; serialize with them so
        // this operation-scoped unlock never relocks in the middle of one.
        let vault_path = expand_vault_path(&config.user_config.credential_vault.path);
        let _unlock_request = vault_unlock_request_lock(&vault_path).lock_owned().await;
        let owner_user_id = owner_user_id.to_string();
        let account_profile = account_profile.to_string();
        tokio::task::spawn_blocking(move || {
            store_claude_setup_token(
                &config,
                &owner_user_id,
                &account_profile,
                &token,
                passphrase.as_ref().map(|value| value.as_str()),
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "store Claude setup token",
            message: error.to_string(),
        })?
    }
}

fn store_claude_setup_token(
    config: &DaemonConfig,
    owner_user_id: &str,
    account_profile: &str,
    token: &str,
    passphrase: Option<&str>,
) -> Result<ClaudeSetupTokenStoreOutcome, DaemonError> {
    let vault = &config.user_config.credential_vault;
    let mut lock_after_store = None;
    if vault.backend == CredentialVaultBackend::CharioxEncrypted {
        let path = expand_vault_path(&vault.path);
        let always_prompt = vault.unlock_policy == CredentialVaultUnlockPolicy::Always;
        if always_prompt || !crate::secret::chariox_encrypted_vault_status(&path)?.unlocked {
            let Some(passphrase) = passphrase else {
                return Ok(ClaudeSetupTokenStoreOutcome::VaultPassphraseRequired(
                    if path.exists() {
                        ClaudeSetupTokenVaultPrompt::Unlock
                    } else {
                        ClaudeSetupTokenVaultPrompt::Create
                    },
                ));
            };
            let keep_unlocked = vault.unlock_policy == CredentialVaultUnlockPolicy::KernelInit;
            if let Err(error) = crate::secret::unlock_chariox_encrypted_vault(
                &path,
                passphrase,
                if keep_unlocked {
                    VaultUnlockLease::KernelShutdown
                } else {
                    VaultUnlockLease::Operation
                },
            ) {
                return Ok(ClaudeSetupTokenStoreOutcome::VaultUnlockFailed(
                    error.to_string(),
                ));
            }
            if !keep_unlocked {
                lock_after_store = Some(path);
            }
        }
    }
    let stored = crate::provider::store_provider_account_credential(
        config,
        owner_user_id,
        "claude",
        account_profile,
        token,
        true,
    );
    if let Some(path) = lock_after_store {
        let _ = crate::secret::lock_chariox_encrypted_vault(&path);
        let _ = crate::secret::clear_vault_secret_process_cache();
    }
    stored.map(|_| ClaudeSetupTokenStoreOutcome::Stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "sk-ant-oat01-TESTONLY-vault-store-aaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    struct HomeGuard {
        root: std::path::PathBuf,
        _lock: crate::env_lock::EnvGuard,
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            std::env::remove_var("CHARIOX_HOME");
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn isolated_home(name: &str) -> HomeGuard {
        let lock = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-claude-setup-token-{name}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("CHARIOX_HOME", &root);
        HomeGuard { root, _lock: lock }
    }

    fn encrypted_config(root: &std::path::Path) -> DaemonConfig {
        let mut config = DaemonConfig::for_tests();
        config.user_config.credential_vault.backend = CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path =
            root.join("vault.enc").to_string_lossy().to_string();
        config.user_config.credential_vault.unlock_policy = CredentialVaultUnlockPolicy::Ttl;
        config
    }

    fn resolved_token(config: &DaemonConfig) -> Vec<(String, String)> {
        crate::provider::resolve_provider_account_credentials(config, "local", "claude", "work")
            .expect("stored credential should resolve")
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn locked_vault_requires_a_workflow_passphrase_and_relocks_after_storing() {
        let home = isolated_home("locked");
        let config = encrypted_config(&home.root);
        let vault_path = expand_vault_path(&config.user_config.credential_vault.path);

        assert_eq!(
            store_claude_setup_token(&config, "local", "work", TOKEN, None).unwrap(),
            ClaudeSetupTokenStoreOutcome::VaultPassphraseRequired(
                ClaudeSetupTokenVaultPrompt::Create
            )
        );
        assert_eq!(
            store_claude_setup_token(&config, "local", "work", TOKEN, Some("vault-pass")).unwrap(),
            ClaudeSetupTokenStoreOutcome::Stored
        );
        assert!(
            !crate::secret::chariox_encrypted_vault_status(&vault_path)
                .unwrap()
                .unlocked,
            "an operation-scoped unlock must lock again after storing"
        );
        assert_eq!(
            store_claude_setup_token(&config, "local", "work", TOKEN, None).unwrap(),
            ClaudeSetupTokenStoreOutcome::VaultPassphraseRequired(
                ClaudeSetupTokenVaultPrompt::Unlock
            )
        );
        assert!(matches!(
            store_claude_setup_token(&config, "local", "work", TOKEN, Some("wrong-pass")).unwrap(),
            ClaudeSetupTokenStoreOutcome::VaultUnlockFailed(_)
        ));

        crate::secret::unlock_chariox_encrypted_vault(
            &vault_path,
            "vault-pass",
            VaultUnlockLease::Operation,
        )
        .unwrap();
        assert_eq!(
            resolved_token(&config),
            vec![(
                crate::provider::CLAUDE_OAUTH_TOKEN_ENV.to_string(),
                TOKEN.to_string()
            )]
        );
        crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
        let _ = crate::secret::clear_vault_secret_process_cache();
    }

    #[test]
    fn a_new_setup_token_replaces_the_profile_credential() {
        let home = isolated_home("replace");
        std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
        let mut config = DaemonConfig::for_tests();
        config.user_config.credential_vault.backend = CredentialVaultBackend::ProcessMemory;

        store_claude_setup_token(
            &config,
            "local",
            "work",
            "sk-ant-oat01-TESTONLY-first",
            None,
        )
        .unwrap();
        assert_eq!(
            store_claude_setup_token(&config, "local", "work", TOKEN, None).unwrap(),
            ClaudeSetupTokenStoreOutcome::Stored
        );
        assert_eq!(
            resolved_token(&config),
            vec![(
                crate::provider::CLAUDE_OAUTH_TOKEN_ENV.to_string(),
                TOKEN.to_string()
            )]
        );
        std::env::remove_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT");
        drop(home);
    }
}
