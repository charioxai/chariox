//! MP-08: Project values enter only ephemeral launch bindings, shared by every placement.
use super::*;
use crate::{config::DaemonConfig, error::DaemonError};
use std::{collections::BTreeMap, path::Path};

pub(crate) fn project_launch_environment(
    config: &DaemonConfig,
    project_id: &str,
    workspace_id: &str,
    directory: &Path,
) -> Result<crate::provider::ProviderCredentialEnvironment, DaemonError> {
    let mut environment = crate::provider::ProviderCredentialEnvironment::default();
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let Some(state) = store.load(project_id)? else {
        return Ok(environment);
    };
    let mut manifest = state.manifest;
    manifest
        .entries
        .retain(|entry| entry.workspace_id == workspace_id);
    let roots = BTreeMap::from([(workspace_id.to_string(), directory.to_path_buf())]);
    let names = manifest
        .entries
        .iter()
        .filter_map(|entry| match &entry.locator {
            ProjectEnvironmentLocator::WorkspaceEnvironment { name } => Some(name),
            _ => None,
        });
    // Select only names already authorized by this Project's manifest, never dump ambient env.
    let inherited = names
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.clone(), zeroize::Zeroizing::new(value)))
        })
        .collect();
    let vault = crate::secret::project_environment_vault(config)?;
    let resolved = resolve_project_environment(
        &manifest,
        &roots,
        &BTreeMap::from([(workspace_id.into(), inherited)]),
        vault.as_ref(),
    )?;
    for (name, value) in project_environment_launch_bindings(&manifest, &resolved, workspace_id)? {
        environment.insert(name, value);
    }
    Ok(environment)
}

// MP-08 / MP-10: Provider-neutral rules are bounded and never sourced from provider homes.
pub(crate) fn read_user_rules_file(root: &Path) -> Result<String, DaemonError> {
    let rules = super::resolver::read_environment_file(root, "user-rules.md")?;
    if rules.len() > 64 * 1024 || rules.contains('\0') {
        return Err(super::resolver::environment_error("invalid user rules"));
    }
    Ok(rules.to_string())
}
pub(crate) fn project_missing_inputs(
    config: &DaemonConfig,
    project: &str,
    workspace: &str,
) -> Result<Vec<String>, DaemonError> {
    Ok(
        ProjectEnvironmentStore::new(&config.private_runtime_state_root())
            .load(project)?
            .map(|state| {
                state
                    .manifest
                    .entries
                    .into_iter()
                    .filter(|entry| {
                        entry.workspace_id == workspace
                            && (entry.excluded
                                || entry.status != ProjectEnvironmentEntryStatus::Found)
                    })
                    .map(|entry| entry.name)
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// MP-08 / MP-10 / MP-11: Provider reuse observes both selected metadata and
/// freshly resolved values. The process-keyed marker stays runtime-only, so
/// low-entropy secrets cannot be guessed from an exported hash or persistence.
pub(crate) fn project_environment_launch_revision(
    manifest: &ProjectEnvironmentManifest,
    environment: &crate::provider::ProviderCredentialEnvironment,
) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::sync::OnceLock;
    static PROCESS_KEY: OnceLock<[u8; 32]> = OnceLock::new();
    let mut mac = Hmac::<Sha256>::new_from_slice(PROCESS_KEY.get_or_init(rand::random))
        .expect("fixed HMAC key length");
    let mut update = |bytes: &[u8]| {
        mac.update(&(bytes.len() as u64).to_le_bytes());
        mac.update(bytes);
    };
    update(&serde_json::to_vec(manifest).expect("manifest encodes"));
    for (name, value) in environment.iter() {
        update(name.as_bytes());
        update(value.as_bytes());
    }
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// MP-08 / MP-10 / MP-11: One resolver for ordinary prompt activation and runtime launches.
pub(crate) fn attach_project_provider_environment(
    config: &DaemonConfig,
    session: &crate::session::RuntimeSession,
    agent: Option<&crate::agent::AgentInstance>,
    mut request: crate::provider::LaunchProviderRequest,
) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
    if let Some(directory) = request.working_directory.as_deref() {
        // A metadata utility executes in a neutral directory outside the Project.
        let is_agent_worktree = agent
            .and_then(|agent| agent.worktree_id().map(std::path::PathBuf::from))
            .is_some_and(|worktree| worktree == directory);
        if directory == std::path::Path::new(session.worktree_id()) || is_agent_worktree {
            let project_environment_state =
                crate::project_environment::ProjectEnvironmentStore::new(
                    &config.private_runtime_state_root(),
                )
                .load(session.project_id())?;
            // MP-08 / MP-10 / MP-11: imported workspaces use mounted paths;
            // a leased session keeps its synthetic workspace identity.
            let directory_workspace = directory.to_string_lossy();
            let workspace_id = project_environment_state
                .as_ref()
                .filter(|state| {
                    let selected = |workspace: &str| {
                        state
                            .manifest
                            .entries
                            .iter()
                            .any(|entry| entry.workspace_id == workspace)
                            || state
                                .manifest
                                .private_files
                                .iter()
                                .any(|file| file.workspace_id == workspace)
                    };
                    !selected(session.workspace_id()) && selected(directory_workspace.as_ref())
                })
                .map_or(session.workspace_id(), |_| directory_workspace.as_ref());
            let environment = crate::project_environment::project_launch_environment(
                config,
                session.project_id(),
                workspace_id,
                directory,
            )?;
            let previously_bound = request.project_environment_revision.is_some();
            request.project_environment_revision =
                project_environment_state.as_ref().map(|state| {
                    crate::project_environment::project_environment_launch_revision(
                        &state.manifest,
                        &environment,
                    )
                });
            let missing = crate::project_environment::project_missing_inputs(
                config,
                session.project_id(),
                workspace_id,
            )?;
            if !missing.is_empty() {
                request.provider_account_env.insert(
                    "CHARIOX_PROJECT_MISSING_INPUTS".into(),
                    serde_json::to_string(&missing).expect("missing names encode"),
                );
            }
            for (name, value) in environment.iter() {
                if crate::account_profile::provider_auth_env_vars(&request.provider).contains(&name)
                    || (!previously_bound
                        && request
                            .provider_credential_env
                            .iter()
                            .any(|(existing, _)| existing == name))
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "Project environment",
                        message: "Project input conflicts with provider credential binding".into(),
                    });
                }
                request
                    .provider_credential_env
                    .insert(name, zeroize::Zeroizing::new(value.to_string()));
            }
        }
    }
    Ok(request)
}

#[cfg(test)]
mod revision_tests {
    use super::*;
    #[test]
    fn mp08_mp10_mp11_provider_reuse_changes_with_decisions_and_live_values() {
        let mut manifest = ProjectEnvironmentManifest {
            schema_version: 1,
            project_id: "project".into(),
            evidence_digest: "evidence".into(),
            entries: vec![],
            private_files: vec![],
            toolchain_hints: vec![],
            package_hints: vec![],
            service_hints: vec![],
        };
        let mut environment = crate::provider::ProviderCredentialEnvironment::default();
        environment.insert("INPUT", zeroize::Zeroizing::new("synthetic-one".into()));
        let first = project_environment_launch_revision(&manifest, &environment);
        assert_eq!(
            first,
            project_environment_launch_revision(&manifest, &environment)
        );
        environment.insert("INPUT", zeroize::Zeroizing::new("synthetic-two".into()));
        assert_ne!(
            first,
            project_environment_launch_revision(&manifest, &environment)
        );
        let second = project_environment_launch_revision(&manifest, &environment);
        manifest.private_files.push(ProjectPrivateFileDecision {
            workspace_id: "workspace".into(),
            path: "notes.md".into(),
            bring: false,
            secret_looking: false,
            reason: "Personal notes".into(),
        });
        let third = project_environment_launch_revision(&manifest, &environment);
        assert_ne!(second, third);
        manifest.private_files[0].bring = true;
        assert_ne!(
            third,
            project_environment_launch_revision(&manifest, &environment)
        );
        assert!(!format!("{environment:?}").contains("synthetic-two"));
    }
}
