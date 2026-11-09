//! MP-08 / MP-09 / MP-10 / MP-11: owner controls on the canonical Vault registry.
use super::*;
use crate::config::{
    UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialMetadataConfig,
    UserCredentialSourceConfig, UserCredentialUse, VaultUseBinding, VaultUseTarget, VaultValueKind,
};
use crate::local::{OwnerVaultActivity, OwnerVaultRequest, OwnerVaultResult, VaultPassphrase};
use crate::runtime::command::KernelCommand;

fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "vault_owner",
        message: message.into(),
    }
}

/// The value to store: a payment card canonically, with its summary in the entry;
/// every other kind as typed.
fn owner_value(
    entry: &mut UserCredentialConfig,
    value: &str,
) -> Result<zeroize::Zeroizing<String>, DaemonError> {
    if value.is_empty() || value.len() > 65536 {
        return Err(failure("Enter a value up to 64 KiB."));
    }
    let metadata = entry.metadata.get_or_insert_default();
    if metadata
        .use_binding
        .as_ref()
        .is_none_or(|binding| binding.value_kind != VaultValueKind::PaymentCard)
    {
        return Ok(zeroize::Zeroizing::new(value.to_owned()));
    }
    let card = crate::config::PaymentCard::parse(value, crate::session::unix_epoch_ms())?;
    metadata.payment_card = Some(card.summary());
    Ok(card.canonical())
}

fn owner_credential(
    owner: String,
    name: String,
    kind: &str,
    use_binding: VaultUseBinding,
) -> Result<UserCredentialConfig, DaemonError> {
    if name.trim().is_empty() || name.len() > 200 {
        return Err(failure("Enter a name up to 200 characters."));
    }
    use_binding.target.validate()?;
    let created_by_kind = match kind {
        "Password" => "owner_password",
        "API key" => "owner_api_key",
        "2FA code" => "owner_totp",
        "Command secret" => "owner_command",
        "File secret" => "owner_file",
        "Payment card" => "owner_payment_card",
        _ => return Err(failure("Choose a secret kind.")),
    };
    if (kind == "Payment card") != (use_binding.value_kind == VaultValueKind::PaymentCard) {
        return Err(failure("Choose the matching kind for this use."));
    }
    let (allowed_hosts, allowed_uses, injection) = match (&use_binding.target, kind) {
        (VaultUseTarget::Site { origin }, "Password" | "API key" | "Payment card") => (
            vec![origin.clone()],
            vec![UserCredentialUse::Browser],
            UserCredentialInjectionConfig::Browser,
        ),
        (VaultUseTarget::Totp { origin }, "2FA code") => (
            vec![origin.clone()],
            vec![UserCredentialUse::Browser],
            UserCredentialInjectionConfig::Totp(crate::config::TotpConfig::default()),
        ),
        (
            VaultUseTarget::CommandEnvironment { origin, .. }
            | VaultUseTarget::CommandStdin { origin },
            "Command secret",
        ) => (
            vec![origin.clone()],
            vec![UserCredentialUse::Env],
            UserCredentialInjectionConfig::Env,
        ),
        (VaultUseTarget::File { .. }, "File secret") => (
            vec![],
            vec![UserCredentialUse::File],
            UserCredentialInjectionConfig::File,
        ),
        _ => return Err(failure("Choose the matching kind for this use.")),
    };
    let id = format!("owner-{:032x}", rand::random::<u128>());
    let now = crate::session::unix_epoch_ms();
    Ok(UserCredentialConfig {
        id: id.clone(),
        description: Some(name.trim().into()),
        source: UserCredentialSourceConfig::Vault { key: id.clone() },
        allowed_hosts,
        allowed_uses,
        injection,
        metadata: Some(UserCredentialMetadataConfig {
            use_binding: Some(use_binding),
            created_by_kind: Some(created_by_kind.into()),
            created_by_id: Some(owner),
            vault_key: Some(id),
            created_at_ms: Some(now),
            updated_at_ms: Some(now),
            ..Default::default()
        }),
    })
}

impl KernelRuntimeState {
    pub(crate) async fn owner_vault(
        &self,
        command: &KernelCommand,
        request: OwnerVaultRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        // KA, sudo and host forwarding cannot turn an agent into an owner client.
        let owner = self.owner_client_user_id(command)?;
        self.require_unlocked_vault()?;
        let _guard = self.vault_observation_mutation_guard().await;
        self.require_unlocked_vault()?;
        let service = self.home_runtime_secret_service()?;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        let result = match request {
            OwnerVaultRequest::Create {
                name,
                kind,
                use_binding,
                value,
            } => {
                let mut entry = owner_credential(owner, name, &kind, use_binding)?;
                let value = owner_value(&mut entry, value.expose_secret())?;
                let id = entry.id.clone();
                commit_entry(
                    &service,
                    &registry,
                    None,
                    entry,
                    Some(value.as_str()),
                    "Created by owner",
                )?;
                OwnerVaultResult::Saved { id }
            }
            OwnerVaultRequest::Generate {
                name,
                kind,
                use_binding,
            } => {
                if kind != "Password" {
                    return Err(failure("Generate is available for passwords. Paste the service's value for other kinds."));
                }
                let entry = owner_credential(owner, name, &kind, use_binding)?;
                let id = entry.id.clone();
                let value =
                    zeroize::Zeroizing::new(super::vault_generate::generate_password(32, true));
                commit_entry(
                    &service,
                    &registry,
                    None,
                    entry,
                    Some(value.as_str()),
                    "Created by owner",
                )?;
                OwnerVaultResult::Saved { id }
            }
            OwnerVaultRequest::Activity { id } => {
                let entry = registered_entry(&registry, &id)?;
                let mut events = service.vault_activity(&id)?;
                if !events
                    .iter()
                    .any(|event| event.event.starts_with("Created"))
                {
                    if let Some(metadata) = entry.metadata {
                        if let Some(at_ms) = metadata.created_at_ms {
                            events.insert(
                                0,
                                OwnerVaultActivity {
                                    at_ms,
                                    event: format!(
                                        "Created by {}",
                                        metadata.created_by_id.as_deref().unwrap_or("agent")
                                    ),
                                },
                            );
                        }
                    }
                }
                OwnerVaultResult::Activity { id, events }
            }
            OwnerVaultRequest::Reveal { id } => {
                let entry = registered_entry(&registry, &id)?;
                let key = vault_key(&entry)?;
                let value = service
                    .vault_secret_for_dictation(key)?
                    .ok_or_else(|| failure("This entry has no saved value. Replace it first."))?;
                service.record_vault_activity(&id, "Viewed by owner")?;
                OwnerVaultResult::Revealed {
                    id,
                    value: VaultPassphrase::new(value.as_str()),
                }
            }
            OwnerVaultRequest::Replace { id, value } => {
                let previous = registered_entry(&registry, &id)?;
                let mut entry = previous.clone();
                let value = owner_value(&mut entry, value.expose_secret())?;
                self.revoke_vault_observation_values(vault_key(&previous)?)
                    .await?;
                entry.metadata.get_or_insert_default().updated_at_ms =
                    Some(crate::session::unix_epoch_ms());
                commit_entry(
                    &service,
                    &registry,
                    Some(previous),
                    entry,
                    Some(value.as_str()),
                    "Replaced by owner",
                )?;
                self.forget_vault_payment_grants(&id);
                OwnerVaultResult::Saved { id }
            }
            OwnerVaultRequest::Delete { id } => {
                let entry = registered_entry(&registry, &id)?;
                let key = vault_key(&entry)?;
                if registry.other_handle_using_vault_key(key, &id)?.is_some() {
                    return Err(failure(
                        "Another entry uses this value. Remove that entry first.",
                    ));
                }
                self.revoke_vault_observation_values(key).await?;
                commit_entry(
                    &service,
                    &registry,
                    Some(entry.clone()),
                    entry,
                    None,
                    "Deleted by owner",
                )?;
                self.forget_vault_payment_grants(&id);
                OwnerVaultResult::Deleted { id }
            }
        };
        Ok(LocalDaemonResponse::OwnerVault { result })
    }
}

fn registered_entry(
    registry: &crate::credential::CharioxCredentialRegistry,
    id: &str,
) -> Result<UserCredentialConfig, DaemonError> {
    registry
        .get(id)?
        .ok_or_else(|| failure("This entry is no longer in the Vault."))
}
fn vault_key(entry: &UserCredentialConfig) -> Result<&str, DaemonError> {
    match &entry.source {
        UserCredentialSourceConfig::Vault { key } => Ok(key),
        _ => Err(failure("This entry does not store its value in the Vault.")),
    }
}

/// Registry, encrypted value and Activity commit together or restore their previous state.
pub(in crate::runtime::state) fn commit_entry(
    service: &crate::secret::RuntimeSecretService,
    registry: &crate::credential::CharioxCredentialRegistry,
    previous: Option<UserCredentialConfig>,
    entry: UserCredentialConfig,
    replacement: Option<&str>,
    event: &str,
) -> Result<(), DaemonError> {
    let normalized = replacement
        .map(|value| crate::secret::normalize_vault_credential_input(entry.clone(), value))
        .transpose()?;
    let (entry, replacement) = match &normalized {
        Some((entry, value)) => (entry.clone(), Some(value.as_str())),
        None => (entry, replacement),
    };
    registry.validate_upsert(&entry)?;
    let key = vault_key(&entry)?;
    let previous_value = service.vault_secret_for_dictation(key)?;
    let commit = (|| {
        match replacement {
            Some(value) => {
                service.set_vault_secret(key, value)?;
                registry.upsert(entry.clone())?;
            }
            None => {
                service.delete_vault_secret(key)?;
                registry.remove(&entry.id)?;
            }
        }
        service.record_vault_activity(&entry.id, event)
    })();
    if let Err(error) = commit {
        let rollback_value = match previous_value {
            Some(value) => service.set_vault_secret(key, value.as_str()),
            None => service.delete_vault_secret(key),
        };
        let rollback_entry = match previous {
            Some(previous) => registry.upsert(previous).map(|_| ()),
            None => {
                if registry.get(&entry.id)?.is_some() {
                    registry.remove(&entry.id).map(|_| ())
                } else {
                    Ok(())
                }
            }
        };
        if rollback_value.is_err() || rollback_entry.is_err() {
            return Err(DaemonError::MachineRequest { code: "vault_owner_recovery_required".into(), message: "The Vault change could not be rolled back. Restore the Vault backup before retrying.".into() });
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
