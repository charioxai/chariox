//! MP-08 / MP-10: Authenticated staged import used by M28 and managed workers.
use super::*;
use crate::{config::DaemonConfig, error::DaemonError, secret::CredentialVaultStore};
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
use zeroize::Zeroizing;

#[derive(Clone)]
pub(crate) struct ProjectEnvironmentImportAuthority {
    pub config: DaemonConfig,
    pub context_id: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub target_kernel_id: String,
}
// Temporary Vault never writes a source namespace into the target's durable Vault.
#[derive(Default)]
struct ImportVault(Mutex<BTreeMap<(String, String), Zeroizing<String>>>);
impl std::fmt::Debug for ImportVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ImportVault([redacted])")
    }
}
impl CredentialVaultStore for ImportVault {
    fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
        self.0
            .lock()
            .expect("import vault")
            .get(&(service.into(), key.into()))
            .map(|v| v.to_string())
            .ok_or_else(|| super::resolver::environment_error("import value unavailable"))
    }
    fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
        self.0
            .lock()
            .expect("import vault")
            .insert((service.into(), key.into()), Zeroizing::new(value.into()));
        Ok(())
    }
    fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
        self.0
            .lock()
            .expect("import vault")
            .remove(&(service.into(), key.into()));
        Ok(())
    }
}

pub(crate) struct PreparedProjectEnvironmentImport {
    authority: ProjectEnvironmentImportAuthority,
    state: StoredProjectEnvironment,
    values: ImportVault,
    materialization:
        Mutex<Option<super::materialization_transaction::ProjectEnvironmentMaterialization>>,
}
impl PreparedProjectEnvironmentImport {
    pub(crate) fn prepare(
        authority: &ProjectEnvironmentImportAuthority,
        layer: &crate::managed_context::development::DevelopmentProjectEnvironment,
        staged_workspaces: &BTreeMap<String, PathBuf>,
        workspace_mapping: &BTreeMap<String, String>,
    ) -> Result<Self, DaemonError> {
        Self::prepare_with_target(
            authority,
            layer,
            staged_workspaces,
            workspace_mapping,
            &layer.sealed.manifest.project_id,
            super::materialization_transaction::MaterializationTarget::StagedDevelopment,
        )
    }
    pub(crate) fn prepare_for_project(
        authority: &ProjectEnvironmentImportAuthority,
        layer: &crate::managed_context::development::DevelopmentProjectEnvironment,
        staged_workspaces: &BTreeMap<String, PathBuf>,
        workspace_mapping: &BTreeMap<String, String>,
        target_project_id: &str,
    ) -> Result<Self, DaemonError> {
        Self::prepare_with_target(
            authority,
            layer,
            staged_workspaces,
            workspace_mapping,
            target_project_id,
            super::materialization_transaction::MaterializationTarget::MountedSource,
        )
    }
    fn prepare_with_target(
        authority: &ProjectEnvironmentImportAuthority,
        layer: &crate::managed_context::development::DevelopmentProjectEnvironment,
        staged_workspaces: &BTreeMap<String, PathBuf>,
        workspace_mapping: &BTreeMap<String, String>,
        target_project_id: &str,
        target: super::materialization_transaction::MaterializationTarget,
    ) -> Result<Self, DaemonError> {
        let fail = super::resolver::environment_error;
        if authority.config.daemon_id != authority.target_kernel_id
            || layer.sealed.binding.context_id != authority.context_id
            || layer.sealed.binding.source_kernel_id != authority.source_kernel_id
            || layer.sealed.binding.target_kernel_id != authority.target_kernel_id
            || crate::runtime::terminal_pairings::public_key_thumbprint(
                &layer.sealed.values.sender_public_key,
            ) != authority.source_key_thumbprint
            || layer.evidence.digest() != layer.sealed.manifest.evidence_digest
            || layer
                .sealed
                .manifest
                .entries
                .iter()
                .any(|e| !staged_workspaces.contains_key(&e.workspace_id))
        {
            return Err(fail("Project environment import authority mismatch"));
        }
        let values = ImportVault::default();
        let resolved = unseal_project_environment(
            &layer.sealed,
            &layer.sealed.binding,
            &authority.config.relay_private_key,
            &layer.sealed.values.sender_public_key,
            &values,
        )?;
        let mut manifest = target_project_environment_manifest(
            &layer.sealed.manifest,
            workspace_mapping,
            &values,
        )?;
        let evidence = ProjectEnvironmentEvidence {
            private_inventory: layer
                .evidence
                .private_inventory
                .iter()
                .map(|(workspace, files)| {
                    workspace_mapping
                        .get(workspace)
                        .cloned()
                        .map(|workspace| (workspace, files.clone()))
                        .ok_or_else(|| fail("private inventory workspace is not selected"))
                })
                .collect::<Result<_, _>>()?,
            files: layer
                .evidence
                .files
                .iter()
                .map(|(workspace, files)| {
                    workspace_mapping
                        .get(workspace)
                        .cloned()
                        .map(|workspace| (workspace, files.clone()))
                        .ok_or_else(|| fail("Project evidence workspace is not selected"))
                })
                .collect::<Result<_, _>>()?,
        };
        let old_service = project_environment_vault_service(&manifest.project_id);
        manifest.project_id = target_project_id.to_string();
        let new_service = project_environment_vault_service(target_project_id);
        for entry in &mut manifest.entries {
            if entry.status != ProjectEnvironmentEntryStatus::Found {
                continue;
            }
            let key = project_environment_vault_key(entry);
            let value = Zeroizing::new(values.get_secret(&old_service, &key)?);
            values.set_secret(&new_service, &key, &value)?;
            if matches!(entry.locator, ProjectEnvironmentLocator::Vault { .. }) {
                entry.locator = ProjectEnvironmentLocator::Vault {
                    service: new_service.clone(),
                    key,
                };
            }
        }
        manifest.evidence_digest = evidence.digest();
        let state = StoredProjectEnvironment {
            source: None,
            reviewed_manifest: Some(manifest.clone()),
            manifest,
            evidence,
            reported_missing: Default::default(),
            last_review: None,
        };
        let materialization =
            super::materialization_transaction::ProjectEnvironmentMaterialization::prepare(
                &layer.sealed.manifest,
                &resolved,
                staged_workspaces,
                target,
            )?;
        Ok(Self {
            authority: authority.clone(),
            state,
            values,
            materialization: Mutex::new(Some(materialization)),
        })
    }
    pub(crate) fn publication_binding(
        &self,
        publication_id: &str,
        archive_sha256: &str,
    ) -> ProjectEnvironmentPublicationBinding {
        ProjectEnvironmentPublicationBinding {
            publication_id: publication_id.into(),
            archive_sha256: archive_sha256.into(),
            project_id: self.state.manifest.project_id.clone(),
            context_id: self.authority.context_id.clone(),
            target_kernel_id: self.authority.target_kernel_id.clone(),
        }
    }
    pub(crate) fn rollback(&self) -> Result<(), DaemonError> {
        let store =
            ProjectEnvironmentStore::new(&self.authority.config.private_runtime_state_root());
        let vault = crate::secret::project_environment_vault(&self.authority.config)?;
        remove_project_environment(&store, &self.state.manifest.project_id, vault.as_ref())
    }
    // Publication happens before this call. A failed commit leaves no published environment state.
    pub(crate) fn commit(&self) -> Result<(), DaemonError> {
        let store =
            ProjectEnvironmentStore::new(&self.authority.config.private_runtime_state_root());
        let _lock = store.lock(&self.state.manifest.project_id)?;
        if store.load(&self.state.manifest.project_id)?.is_some() {
            return Err(super::resolver::environment_error(
                "import must not overwrite an independent Project environment",
            ));
        }
        let vault = crate::secret::project_environment_vault(&self.authority.config)?;
        let service = project_environment_vault_service(&self.state.manifest.project_id);
        let mut written = Vec::new();
        let result = (|| {
            for entry in self
                .state
                .manifest
                .entries
                .iter()
                .filter(|entry| entry.status == ProjectEnvironmentEntryStatus::Found)
            {
                let key = project_environment_vault_key(entry);
                let value = Zeroizing::new(self.values.get_secret(&service, &key)?);
                // A fresh import owns its selected keys only; preexisting values are never replaced.
                if vault.get_secret(&service, &key).is_ok() {
                    return Err(super::resolver::environment_error(
                        "target Project input already exists",
                    ));
                }
                vault.set_secret(&service, &key, &value)?;
                written.push(key);
            }
            store.save(&self.state)
        })();
        if result.is_err() {
            // save() may have renamed the new manifest before a directory fsync failed.
            store.remove(&self.state.manifest.project_id)?;
            let mut rollback_error = None;
            for key in written {
                if let Err(error) = vault.delete_secret(&service, &key) {
                    rollback_error.get_or_insert(error);
                }
            }
            self.materialization
                .lock()
                .expect("Project materialization")
                .take();
            if let Some(error) = rollback_error {
                return Err(error);
            }
        } else if let Some(files) = self
            .materialization
            .lock()
            .expect("Project materialization")
            .take()
        {
            files.commit();
        }
        result
    }
}

// MP-08 / MP-10 / MP-11: Private completion evidence cannot contain values.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectEnvironmentPublicationBinding {
    pub publication_id: String,
    pub archive_sha256: String,
    pub project_id: String,
    pub context_id: String,
    pub target_kernel_id: String,
}
impl ProjectEnvironmentPublicationBinding {
    pub(crate) fn verify_target(
        &self,
        authority: &ProjectEnvironmentImportAuthority,
    ) -> Result<(), DaemonError> {
        let fail = super::resolver::environment_error;
        if self.context_id != authority.context_id
            || self.target_kernel_id != authority.target_kernel_id
        {
            return Err(fail("Project environment completion authority mismatch"));
        }
        let store = ProjectEnvironmentStore::new(&authority.config.private_runtime_state_root());
        let state = store
            .load(&self.project_id)?
            .ok_or_else(|| fail("published Project environment is missing"))?;
        let vault = crate::secret::project_environment_vault(&authority.config)?;
        let service = project_environment_vault_service(&self.project_id);
        for entry in state
            .manifest
            .entries
            .iter()
            .filter(|entry| entry.status == ProjectEnvironmentEntryStatus::Found)
        {
            // Only test availability; never print, persist or compare the value.
            let _value =
                Zeroizing::new(vault.get_secret(&service, &project_environment_vault_key(entry))?);
        }
        Ok(())
    }
}
