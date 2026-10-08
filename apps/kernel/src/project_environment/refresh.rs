//! MP-08: Export-time refresh is local to the exporting kernel, never synchronization.
use super::resolver::environment_error;
use super::*;
use crate::error::DaemonError;
use crate::secret::CredentialVaultStore;
use std::collections::BTreeMap;
use std::path::PathBuf;
use zeroize::Zeroizing;

#[derive(Debug, Clone)]
pub struct ProjectEnvironmentDiscoveryRequest {
    pub project_id: String,
    pub previous_manifest: Option<ProjectEnvironmentManifest>,
    pub evidence_digest: String,
    pub changed_paths: BTreeMap<String, Vec<String>>,
}

pub struct PreparedProjectEnvironmentExport {
    pub state: StoredProjectEnvironment,
    pub resolved: ResolvedProjectEnvironment,
    pub newly_missing: Vec<ProjectEnvironmentEntry>,
    pub discovery_ran: bool,
}
impl std::fmt::Debug for PreparedProjectEnvironmentExport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedProjectEnvironmentExport")
            .field("state", &self.state)
            .field("resolved", &self.resolved)
            .field("newly_missing", &self.newly_missing)
            .field("discovery_ran", &self.discovery_ran)
            .finish()
    }
}

/// The runtime supplies the official-provider discovery callback, never a client-authored manifest.
/// Evidence must be regenerated from the current complete index on every export, including new files.
/// The callback is never called for an unchanged digest; values are always resolved again.
pub fn prepare_project_environment_export(
    store: &ProjectEnvironmentStore,
    project_id: &str,
    evidence: ProjectEnvironmentEvidence,
    workspaces: &BTreeMap<String, PathBuf>,
    workspace_environment: &BTreeMap<String, BTreeMap<String, Zeroizing<String>>>,
    vault: &dyn CredentialVaultStore,
    discover: impl FnOnce(
        ProjectEnvironmentDiscoveryRequest,
    ) -> Result<ProjectEnvironmentManifest, DaemonError>,
) -> Result<PreparedProjectEnvironmentExport, DaemonError> {
    let _refresh_lock = store.lock(project_id)?;
    let previous = store.load(project_id)?;
    let evidence_digest = evidence.digest();
    let discovery_ran = previous
        .as_ref()
        .is_none_or(|state| state.manifest.evidence_digest != evidence_digest);
    let mut manifest = if discovery_ran {
        let previous_evidence = previous
            .as_ref()
            .map(|state| state.evidence.clone())
            .unwrap_or_default();
        let discovered = discover(ProjectEnvironmentDiscoveryRequest {
            project_id: project_id.into(),
            previous_manifest: previous.as_ref().map(|state| state.manifest.clone()),
            evidence_digest: evidence_digest.clone(),
            changed_paths: evidence.changed_paths(&previous_evidence),
        })?;
        discovered.validate().map_err(environment_error)?;
        if discovered.project_id != project_id || discovered.evidence_digest != evidence_digest {
            return Err(environment_error(
                "discovery Project or evidence binding mismatch",
            ));
        }
        discovered
    } else {
        previous
            .as_ref()
            .expect("unchanged evidence has a manifest")
            .manifest
            .clone()
    };
    let resolved =
        resolve_project_environment(&manifest, workspaces, workspace_environment, vault)?;
    for entry in &mut manifest.entries {
        entry.status = resolved
            .unresolved
            .iter()
            .find(|unresolved| {
                unresolved.workspace_id == entry.workspace_id && unresolved.name == entry.name
            })
            .map_or(ProjectEnvironmentEntryStatus::Found, |unresolved| {
                unresolved.status
            });
    }
    let reported_missing = previous
        .as_ref()
        .map(|state| state.reported_missing.clone())
        .unwrap_or_default();
    let newly_missing = resolved
        .unresolved
        .iter()
        .filter(|entry| {
            !reported_missing.contains(&(entry.workspace_id.clone(), entry.name.clone()))
        })
        .cloned()
        .collect();
    let state = StoredProjectEnvironment {
        source: previous.as_ref().and_then(|state| state.source.clone()),
        manifest,
        evidence,
        reported_missing,
        reviewed_manifest: None,
        last_review: None,
    };
    store.save(&state)?;
    Ok(PreparedProjectEnvironmentExport {
        state,
        resolved,
        newly_missing,
        discovery_ran,
    })
}

/// Store user input directly in the Project Vault namespace and make the Vault its locator.
/// Future exports resolve that kernel-owned value without asking again.
pub fn supply_project_environment_inputs(
    state: &mut StoredProjectEnvironment,
    inputs: BTreeMap<(String, String), Zeroizing<String>>,
    vault: &dyn CredentialVaultStore,
) -> Result<(), DaemonError> {
    let service = project_environment_vault_service(&state.manifest.project_id);
    for ((workspace, name), value) in &inputs {
        if value.is_empty()
            || value.len() > 1024 * 1024
            || value.contains('\0')
            || !state.manifest.entries.iter().any(|entry| {
                &entry.workspace_id == workspace
                    && &entry.name == name
                    && entry.status != ProjectEnvironmentEntryStatus::Found
            })
        {
            return Err(environment_error(
                "input does not match an unresolved Project environment entry",
            ));
        }
    }
    for entry in &mut state.manifest.entries {
        let identity = (entry.workspace_id.clone(), entry.name.clone());
        if let Some(value) = inputs.get(&identity) {
            vault.set_secret(&service, &project_environment_vault_key(entry), value)?;
            entry.locator = ProjectEnvironmentLocator::Vault {
                service: service.clone(),
                key: project_environment_vault_key(entry),
            };
            state.reported_missing.insert(identity);
            entry.status = ProjectEnvironmentEntryStatus::Found;
        }
    }
    Ok(())
}

/// MP-08: Delete only this Project's captured values, never an externally referenced Vault entry.
pub fn remove_project_environment(
    store: &ProjectEnvironmentStore,
    project_id: &str,
    vault: &dyn CredentialVaultStore,
) -> Result<(), DaemonError> {
    let _lock = store.lock(project_id)?;
    if let Some(state) = store.load(project_id)? {
        let service = project_environment_vault_service(project_id);
        for entry in &state.manifest.entries {
            vault.delete_secret(&service, &project_environment_vault_key(entry))?;
        }
        super::forget_project_file_rules(&state.manifest);
    }
    store.remove(project_id)?;
    Ok(())
}
