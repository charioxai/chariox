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
