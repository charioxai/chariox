//! MP-08 / MP-10 / MP-11: owner-only value management. No runtime tool or public
//! projection calls this API. The encrypted owner catalogue retains opaque uses;
//! the agent-use schema is owned by the shared Vault integration, not this UI.
use super::*;
use crate::local::{
    OwnerVaultActivity, OwnerVaultEntry, OwnerVaultRequest, OwnerVaultResult, VaultPassphrase,
};
use crate::runtime::command::KernelCommand;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
struct Catalogue {
    entries: Vec<SavedEntry>,
    activity: std::collections::BTreeMap<String, Vec<OwnerVaultActivity>>,
}

#[derive(Serialize, Deserialize)]
struct SavedEntry {
    entry: OwnerVaultEntry,
    value: VaultPassphrase,
}

impl Catalogue {
    fn record(&mut self, id: &str, event: &str) {
        self.activity
            .entry(id.into())
            .or_default()
            .push(OwnerVaultActivity {
                at_ms: crate::session::unix_epoch_ms(),
                event: event.into(),
            });
    }

    fn save(&self, service: &crate::secret::RuntimeSecretService) -> Result<(), DaemonError> {
        let serialized = zeroize::Zeroizing::new(
            serde_json::to_string(self)
                .map_err(|_| failure("Could not save the Vault activity. Try again."))?,
        );
        service.save_owner_vault_catalogue(Some(&serialized))
    }
}

fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "vault_owner",
        message: message.into(),
    }
}

fn validate(name: &str, kind: &str, usage: &str, value: &str) -> Result<(), DaemonError> {
    if name.trim().is_empty() || name.len() > 200 || usage.trim().is_empty() || usage.len() > 2048 {
        return Err(failure("Enter a name and one use for this secret."));
    }
    if ![
        "Password",
        "API key",
        "2FA code",
        "Command secret",
        "File secret",
    ]
    .contains(&kind)
    {
        return Err(failure("Choose a secret kind."));
    }
    if value.is_empty() || value.len() > 65536 {
        return Err(failure("Enter a value up to 64 KiB."));
    }
    Ok(())
}

impl KernelRuntimeState {
    pub(crate) async fn owner_vault(
        &self,
        command: &KernelCommand,
        request: OwnerVaultRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        // Check here as well as router admission: /sudo, KA and host forwarding
        // never turn an agent into the owner's client.
        let owner = self.owner_client_user_id(command)?;
        self.require_unlocked_vault()?;
        let _guard = self.vault_observation_mutation_guard().await;
        self.require_unlocked_vault()?;
        let service = self.home_runtime_secret_service()?;
        let previous_catalogue = service.owner_vault_catalogue()?;
        let mut catalogue: Catalogue = match previous_catalogue.as_ref() {
            Some(value) => serde_json::from_str(value.as_str()).map_err(|_| failure("The Vault catalogue could not be read. Restore its backup before changing values."))?,
            None => Catalogue::default(),
        };
        let result = match request {
            OwnerVaultRequest::List {} => OwnerVaultResult::List {
                entries: catalogue
                    .entries
                    .iter()
                    .map(|saved| saved.entry.clone())
                    .collect(),
            },
            OwnerVaultRequest::Create {
                name,
                kind,
                usage,
                value,
            } => create_owner_value(&service, &mut catalogue, owner, name, kind, usage, value)?,
            OwnerVaultRequest::Generate { name, kind, usage } => {
                if kind != "Password" {
                    return Err(failure("Generate is available for passwords. Paste the service's value for other kinds."));
                }
                let value = super::sudo::generate_password(32, true);
                create_owner_value(
                    &service,
                    &mut catalogue,
                    owner,
                    name,
                    kind,
                    usage,
                    VaultPassphrase::new(value),
                )?
            }
            OwnerVaultRequest::Activity { id } => {
                let mut events = catalogue.activity.get(&id).cloned().unwrap_or_default();
                if !catalogue.entries.iter().any(|saved| saved.entry.id == id) {
                    let entry = crate::credential::CharioxCredentialRegistry::user()?
                        .get(&id)?
                        .ok_or_else(|| failure("This entry is no longer in the Vault."))?;
                    if let Some(metadata) = entry.metadata {
                        if let Some(at_ms) = metadata.created_at_ms {
                            events.insert(
                                0,
                                OwnerVaultActivity {
                                    at_ms,
                                    event: "Created".into(),
                                },
                            );
                        }
                        if let Some(at_ms) = metadata
                            .updated_at_ms
                            .filter(|time| Some(*time) != metadata.created_at_ms)
                        {
                            events.push(OwnerVaultActivity {
                                at_ms,
                                event: "Updated".into(),
                            });
                        }
                    }
                    events.sort_by_key(|event| event.at_ms);
                }
                OwnerVaultResult::Activity { id, events }
            }
            OwnerVaultRequest::Reveal { id } => {
                let value = match catalogue.entries.iter().find(|saved| saved.entry.id == id) {
                    Some(saved) => saved.value.clone(),
                    None => {
                        let key = registered_vault_key(&id)?;
                        let value = service.vault_secret_for_dictation(&key)?.ok_or_else(|| {
                            failure("This entry has no saved value. Replace it first.")
                        })?;
                        VaultPassphrase::new(value.as_str())
                    }
                };
                // Commit the audit before returning any value, only to this caller.
                catalogue.record(&id, "Viewed by owner");
                catalogue.save(&service)?;
                OwnerVaultResult::Revealed { id, value }
            }
            OwnerVaultRequest::Replace { id, value } => {
                if value.expose_secret().is_empty() || value.expose_secret().len() > 65536 {
                    return Err(failure("Enter a value up to 64 KiB."));
                }
                if let Some(saved) = catalogue
                    .entries
                    .iter_mut()
                    .find(|saved| saved.entry.id == id)
                {
                    saved.value = value;
                    saved.entry.updated_at_ms = crate::session::unix_epoch_ms();
                } else {
                    let registry = crate::credential::CharioxCredentialRegistry::user()?;
                    let entry = registry
                        .get(&id)?
                        .ok_or_else(|| failure("This entry is no longer in the Vault."))?;
                    let key = registered_vault_key(&id)?;
                    self.revoke_vault_observation_values(&key).await?;
                    catalogue.record(&id, "Replaced by owner");
                    commit_registered(
                        &service,
                        &registry,
                        entry,
                        &catalogue,
                        previous_catalogue.as_deref().map(String::as_str),
                        Some(value.expose_secret()),
                    )?;
                    return Ok(LocalDaemonResponse::OwnerVault {
                        result: OwnerVaultResult::Saved { id },
                    });
                }
                catalogue.record(&id, "Replaced by owner");
                catalogue.save(&service)?;
                OwnerVaultResult::Saved { id }
            }
            OwnerVaultRequest::Delete { id } => {
                if let Some(index) = catalogue
                    .entries
                    .iter()
                    .position(|saved| saved.entry.id == id)
                {
                    catalogue.entries.remove(index);
                } else {
                    let registry = crate::credential::CharioxCredentialRegistry::user()?;
                    let entry = registry
                        .get(&id)?
                        .ok_or_else(|| failure("This entry is no longer in the Vault."))?;
                    if let crate::config::UserCredentialSourceConfig::Vault { key } = &entry.source
                    {
                        if registry.other_handle_using_vault_key(key, &id)?.is_some() {
                            return Err(failure(
                                "Another entry uses this value. Remove that entry first.",
                            ));
                        }
                        self.revoke_vault_observation_values(key).await?;
                    }
                    catalogue.activity.remove(&id);
                    commit_registered(
                        &service,
                        &registry,
                        entry,
                        &catalogue,
                        previous_catalogue.as_deref().map(String::as_str),
                        None,
                    )?;
                    return Ok(LocalDaemonResponse::OwnerVault {
                        result: OwnerVaultResult::Deleted { id },
                    });
                }
                catalogue.activity.remove(&id);
                catalogue.save(&service)?;
                OwnerVaultResult::Deleted { id }
            }
        };
        Ok(LocalDaemonResponse::OwnerVault { result })
    }
}

fn create_owner_value(
    service: &crate::secret::RuntimeSecretService,
    catalogue: &mut Catalogue,
    owner: String,
    name: String,
    kind: String,
    usage: String,
    value: VaultPassphrase,
) -> Result<OwnerVaultResult, DaemonError> {
    validate(&name, &kind, &usage, value.expose_secret())?;
    let id = format!("owner-{:032x}", rand::random::<u128>());
    let now = crate::session::unix_epoch_ms();
    catalogue.entries.push(SavedEntry {
        entry: OwnerVaultEntry {
            id: id.clone(),
            name: name.trim().into(),
            kind,
            usage: usage.trim().into(),
            created_by: owner,
            created_at_ms: now,
            updated_at_ms: now,
        },
        value,
    });
    catalogue.record(&id, "Created by owner");
    catalogue.save(service)?;
    Ok(OwnerVaultResult::Saved { id })
}

fn registered_vault_key(id: &str) -> Result<String, DaemonError> {
    let entry = crate::credential::CharioxCredentialRegistry::user()?
        .get(id)?
        .ok_or_else(|| failure("This entry is no longer in the Vault."))?;
    match entry.source {
        crate::config::UserCredentialSourceConfig::Vault { key } => Ok(key),
        _ => Err(failure("This entry does not store its value in the Vault.")),
    }
}

/// The legacy registry is a separate file. A failed commit restores all three
/// pieces; an unsuccessful rollback is an explicit recovery result, never success.
fn commit_registered(
    service: &crate::secret::RuntimeSecretService,
    registry: &crate::credential::CharioxCredentialRegistry,
    entry: crate::config::UserCredentialConfig,
    catalogue: &Catalogue,
    previous_catalogue: Option<&str>,
    replacement: Option<&str>,
) -> Result<(), DaemonError> {
    let key = match &entry.source {
        crate::config::UserCredentialSourceConfig::Vault { key } => Some(key.as_str()),
        _ => None,
    };
    let previous_value = key
        .map(|key| service.vault_secret_for_dictation(key))
        .transpose()?
        .flatten();
    let commit = (|| {
        if let Some(key) = key {
            match replacement {
                Some(value) => service.set_vault_secret(key, value)?,
                None => service.delete_vault_secret(key)?,
            }
        }
        if replacement.is_none() {
            registry.remove(&entry.id)?;
        }
        catalogue.save(service)
    })();
    if let Err(error) = commit {
        let rollback_value = match key {
            Some(key) => match previous_value {
                Some(value) => service.set_vault_secret(key, value.as_str()),
                None => service.delete_vault_secret(key),
            },
            None => Ok(()),
        };
        let rollback_entry = if replacement.is_none() {
            registry.upsert(entry).map(|_| ())
        } else {
            Ok(())
        };
        let rollback_catalogue = service.save_owner_vault_catalogue(previous_catalogue);
        if rollback_value.is_err() || rollback_entry.is_err() || rollback_catalogue.is_err() {
            return Err(DaemonError::MachineRequest { code: "vault_owner_recovery_required".into(), message: "The Vault change could not be rolled back. Restore the Vault backup before retrying.".into() });
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
