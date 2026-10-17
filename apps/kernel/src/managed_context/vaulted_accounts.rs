//! MP-08 / MP-10 / MP-11: a Claude account whose only launch credential is a
//! Vault setup token reaches a managed kernel through the Vault. The source
//! seals just the selected items to the target kernel; the target restores
//! them into its own Vault. Values never leave a Vault or a launch environment.
use serde::{Deserialize, Serialize};

use crate::account_profile::{
    ManagedContextProviderAccountReceipt, ProviderAccountMaterialization,
    ProviderAccountProfileRegistry,
};
use crate::config::{
    CredentialVaultBackend, DaemonConfig, UserCredentialConfig, UserCredentialInjectionConfig,
    UserCredentialMetadataConfig, UserCredentialSourceConfig, UserCredentialUse,
    UserCredentialVaultConfig,
};
use crate::credential::CharioxCredentialRegistry;
use crate::error::DaemonError;
use crate::secret::{TransferredVaultSnapshot, TransferredVaultSourceBinding};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VaultedProviderCredential {
    pub provider: String,
    pub profile_id: String,
    /// Source credential handle; names the Vault item, never its value.
    pub credential_id: String,
    pub vault_service: String,
    pub vault_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_validated_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VaultedProviderAccounts {
    pub credentials: Vec<VaultedProviderCredential>,
    /// Absent when the Source kernel context already transfers the whole Vault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<TransferredVaultSnapshot>,
}

const CREATED_BY_KIND: &str = "managed_context_vault";

fn written_by(credential: &UserCredentialConfig, context_id: &str) -> bool {
    credential.metadata.as_ref().is_some_and(|metadata| {
        metadata.created_by_kind.as_deref() == Some(CREATED_BY_KIND)
            && metadata.created_by_id.as_deref() == Some(context_id)
    })
}

fn vaulted_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed context vaulted provider account",
        message: message.into(),
    }
}

fn encrypted_vault(vault: &UserCredentialVaultConfig) -> Result<(), DaemonError> {
    if vault.backend != CredentialVaultBackend::CharioxEncrypted {
        return Err(vaulted_error(
            "a Vault setup token transfers only from and to a Chariox encrypted Vault",
        ));
    }
    Ok(())
}

/// Source: the file-free replica of a Claude account and the Vault item behind
/// its setup token. Fails unless that item is a provider-only Vault credential.
pub(crate) fn export_vaulted_account(
    config: &DaemonConfig,
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<(ProviderAccountMaterialization, VaultedProviderCredential), DaemonError> {
    encrypted_vault(&config.user_config.credential_vault)?;
    let (materialization, last_validated_at_ms) =
        registry.export_managed_context_vaulted_replica(owner_user_id, provider, profile_id)?;
    let profile = &materialization.profile;
    let credential_id = crate::provider::provider_account_credential_id(
        owner_user_id,
        "claude",
        &profile.profile_id,
    );
    let vault_key = crate::credential::load_user_credentials()?
        .into_iter()
        .find(|credential| credential.id == credential_id)
        .and_then(|credential| match credential.source {
            UserCredentialSourceConfig::Vault { key }
                if credential.allowed_uses == [UserCredentialUse::Provider]
                    && matches!(
                        credential.injection,
                        UserCredentialInjectionConfig::Provider
                    ) =>
            {
                Some(key)
            }
            _ => None,
        })
        .ok_or_else(|| {
            vaulted_error(format!(
                "claude account profile `{}` has no Vault setup token",
                profile.profile_id
            ))
        })?;
    let credential = VaultedProviderCredential {
        provider: profile.provider.clone(),
        profile_id: profile.profile_id.clone(),
        credential_id,
        vault_service: config
            .user_config
            .credential_vault
            .service_name()
            .to_string(),
        vault_key,
        last_validated_at_ms,
    };
    Ok((materialization, credential))
}

/// Source: CREATE preflight. The token is read when the package is built, so
/// the Vault must be open now; no compute is rented otherwise.
pub(crate) fn ensure_source_vault_open(
    config: &DaemonConfig,
    profile_id: &str,
    credential: &VaultedProviderCredential,
) -> Result<(), DaemonError> {
    if !crate::secret::chariox_encrypted_vault_status(&config.user_config.credential_vault.path)?
        .unlocked
    {
        return Err(vaulted_error(format!(
            "the setup token of claude account `{profile_id}` is in the Chariox Vault, which is locked; unlock the Vault and retry"
        )));
    }
    crate::secret::check_selected_vault_item(
        &config.user_config.credential_vault,
        &credential.vault_service,
        &credential.vault_key,
    ).map_err(|_| vaulted_error(format!(
        "the setup token of claude account `{profile_id}` is missing from the selected Vault; restore it and retry"
    )))?;
    Ok(())
}

/// Source: seal the selected items into a new Vault for the target kernel,
/// unless the whole Vault already travels with the Source kernel context.
pub(crate) fn seal_vaulted_accounts(
    config: &DaemonConfig,
    credentials: Vec<VaultedProviderCredential>,
    whole_vault_transferred: bool,
    context_id: &str,
    target_kernel_id: &str,
    target_public_key: &str,
) -> Result<Option<VaultedProviderAccounts>, DaemonError> {
    if credentials.is_empty() {
        return Ok(None);
    }
    let vault = if whole_vault_transferred {
        None
    } else {
        let items = credentials
            .iter()
            .map(|credential| {
                (
                    credential.vault_service.clone(),
                    credential.vault_key.clone(),
                )
            })
            .collect::<Vec<_>>();
        Some(crate::secret::export_selected_vault_snapshot(
            &config.user_config.credential_vault.path,
            &items,
            context_id,
            &config.daemon_id,
            &config.relay_private_key,
            target_kernel_id,
            target_public_key,
        )?)
    };
    crate::logging::info_with_fields(
        "managed_context.vaulted_provider_credentials",
        "provider credentials sealed for the managed kernel",
        serde_json::json!({
            "context_id": context_id,
            "accounts": credentials
                .iter()
                .map(|credential| format!("{}/{}", credential.provider, credential.profile_id))
                .collect::<Vec<_>>(),
            "whole_vault_transferred": whole_vault_transferred,
        }),
    );
    Ok(Some(VaultedProviderAccounts { credentials, vault }))
}

/// Target: restore the sealed Vault and register one provider credential per
/// imported replica. Existing target Vaults and credentials are never replaced;
/// a failure removes what this import wrote.
pub(crate) fn install_vaulted_accounts(
    vaulted: &VaultedProviderAccounts,
    replicas: &[(
        VaultedProviderCredential,
        ManagedContextProviderAccountReceipt,
    )],
    owner_user_id: &str,
    vault: &UserCredentialVaultConfig,
    expected_source: &TransferredVaultSourceBinding,
    target_kernel_id: &str,
    target_private_key: &str,
) -> Result<(), DaemonError> {
    encrypted_vault(vault)?;
    let result = (|| {
        if let Some(snapshot) = &vaulted.vault {
            if vaulted
                .credentials
                .iter()
                .any(|credential| credential.vault_service != vault.service_name())
            {
                return Err(vaulted_error(
                    "transferred setup tokens use a different Vault service",
                ));
            }
            // A replay finds the Vault this binding already installed.
            if crate::secret::validate_installed_transferred_vault(
                &vault.path,
                expected_source,
                target_kernel_id,
                target_private_key,
            )
            .is_err()
            {
                crate::secret::install_transferred_vault_snapshot(
                    &vault.path,
                    snapshot,
                    expected_source,
                    target_kernel_id,
                    target_private_key,
                )?;
            }
        }
        let registry = CharioxCredentialRegistry::user()?;
        for (credential, receipt) in replicas {
            let id = crate::provider::provider_account_credential_id(
                owner_user_id,
                "claude",
                &receipt.profile_id,
            );
            // The Source kernel context registers the source handle itself.
            if vaulted.vault.is_none() && id == credential.credential_id {
                continue;
            }
            if let Some(existing) = registry.get(&id)? {
                // A replay of this import finds its own definition unchanged.
                if written_by(&existing, &expected_source.context_id)
                    && existing.source
                        == (UserCredentialSourceConfig::Vault {
                            key: credential.vault_key.clone(),
                        })
                {
                    continue;
                }
                return Err(vaulted_error("target provider credential already exists"));
            }
            let now_ms = crate::session::unix_epoch_ms();
            register_vaulted_credential(
                &registry,
                UserCredentialConfig {
                    id,
                    description: Some(format!("claude account profile {}", receipt.profile_id)),
                    source: UserCredentialSourceConfig::Vault {
                        key: credential.vault_key.clone(),
                    },
                    allowed_hosts: Vec::new(),
                    allowed_uses: vec![UserCredentialUse::Provider],
                    injection: UserCredentialInjectionConfig::Provider,
                    metadata: Some(UserCredentialMetadataConfig {
                        created_by_kind: Some(CREATED_BY_KIND.to_string()),
                        created_by_id: Some(expected_source.context_id.clone()),
                        session_id: None,
                        provider: Some("claude".to_string()),
                        provider_run_id: None,
                        vault_key: Some(credential.vault_key.clone()),
                        created_at_ms: Some(now_ms),
                        updated_at_ms: Some(now_ms),
                    }),
                },
            )?;
        }
        Ok(())
    })();
    let Err(error) = result else {
        return Ok(());
    };
    match rollback_vaulted_accounts(
        &expected_source.context_id,
        vault,
        vaulted.vault.is_some(),
        target_kernel_id,
        target_private_key,
    ) {
        Ok(()) => Err(error),
        Err(rollback_error) => Err(vaulted_error(format!(
            "{error}; roll back transferred setup tokens: {rollback_error}"
        ))),
    }
}

/// Removes the credential definitions and, when it carried one, the Vault that
/// the import of `context_id` installed. Anything else is left untouched.
pub(crate) fn rollback_vaulted_accounts(
    context_id: &str,
    vault: &UserCredentialVaultConfig,
    remove_vault: bool,
    target_kernel_id: &str,
    target_private_key: &str,
) -> Result<(), DaemonError> {
    let registry = CharioxCredentialRegistry::user()?;
    for credential in registry.list()? {
        if written_by(&credential, context_id) {
            remove_vaulted_credential(&registry, &credential)?;
        }
    }
    if remove_vault {
        crate::secret::remove_transferred_vault_for_context(
            &vault.path,
            context_id,
            target_kernel_id,
            target_private_key,
        )?;
    }
    Ok(())
}

fn register_vaulted_credential(
    registry: &CharioxCredentialRegistry,
    credential: UserCredentialConfig,
) -> Result<(), DaemonError> {
    if !registry.insert_if_absent(credential)? {
        return Err(vaulted_error("target provider credential already exists"));
    }
    Ok(())
}
fn remove_vaulted_credential(
    registry: &CharioxCredentialRegistry,
    expected: &UserCredentialConfig,
) -> Result<(), DaemonError> {
    registry.remove_if_matches(expected)?;
    Ok(())
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    fn definition(key: &str, imported: bool) -> UserCredentialConfig {
        UserCredentialConfig {
            id: "claude-import".into(),
            description: None,
            source: UserCredentialSourceConfig::Vault { key: key.into() },
            allowed_hosts: vec![],
            allowed_uses: vec![UserCredentialUse::Provider],
            injection: UserCredentialInjectionConfig::Provider,
            metadata: imported.then(|| UserCredentialMetadataConfig {
                created_by_kind: Some(CREATED_BY_KIND.into()),
                created_by_id: Some("context".into()),
                session_id: None,
                provider: Some("claude".into()),
                provider_run_id: None,
                vault_key: Some(key.into()),
                created_at_ms: Some(1),
                updated_at_ms: Some(1),
            }),
        }
    }
    #[test]
    fn mp11_import_preserves_registration_created_after_absence_check() {
        let root = crate::test_support::TestWorktree::new("vault-registration-race");
        let registry = CharioxCredentialRegistry::new(root.path().join("registry"));
        assert!(registry.get("claude-import").unwrap().is_none());
        let registered = definition("operator-owned", false);
        registry.upsert(registered.clone()).unwrap();
        assert!(register_vaulted_credential(&registry, definition("imported", true)).is_err());
        assert_eq!(registry.get("claude-import").unwrap(), Some(registered));
    }
    #[test]
    fn mp11_rollback_preserves_replacement_after_ownership_snapshot() {
        let root = crate::test_support::TestWorktree::new("vault-rollback-race");
        let registry = CharioxCredentialRegistry::new(root.path().join("registry"));
        registry.upsert(definition("imported", true)).unwrap();
        let snapshot = registry.list().unwrap().pop().unwrap();
        assert!(written_by(&snapshot, "context"));
        let registered = definition("operator-owned", false);
        registry.upsert(registered.clone()).unwrap();
        remove_vaulted_credential(&registry, &snapshot).unwrap();
        assert_eq!(registry.get("claude-import").unwrap(), Some(registered));
    }
}
