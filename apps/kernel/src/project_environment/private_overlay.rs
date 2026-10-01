//! MP-08 / MP-10: Export and live sync consume the same kernel-owned file selection.
use super::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};

fn selections() -> &'static RwLock<BTreeMap<PathBuf, (String, Vec<String>, Vec<String>)>> {
    static SELECTIONS: OnceLock<RwLock<BTreeMap<PathBuf, (String, Vec<String>, Vec<String>)>>> =
        OnceLock::new();
    SELECTIONS.get_or_init(Default::default)
}
pub(crate) fn register_project_file_rules(
    manifest: &ProjectEnvironmentManifest,
    roots: &BTreeMap<String, PathBuf>,
) {
    let mut registry = selections()
        .write()
        .expect("Project private overlay registry");
    for (workspace, root) in roots {
        let rules = manifest
            .private_files
            .iter()
            .filter(|file| &file.workspace_id == workspace)
            .map(|file| format!("{}{}", if file.bring { "!" } else { "" }, file.path))
            .collect();
        let secrets = manifest
            .private_files
            .iter()
            .filter(|file| &file.workspace_id == workspace && file.secret_looking)
            .map(|file| file.path.clone())
            .collect();
        registry.insert(root.clone(), (manifest.project_id.clone(), rules, secrets));
    }
}
pub(crate) fn restore_project_file_rules(manifest: &ProjectEnvironmentManifest) {
    let roots = manifest
        .private_files
        .iter()
        .filter_map(|file| {
            let path = PathBuf::from(&file.workspace_id);
            path.is_absolute()
                .then(|| (file.workspace_id.clone(), path))
        })
        .collect();
    register_project_file_rules(manifest, &roots);
}
pub(crate) fn forget_project_file_rules(manifest: &ProjectEnvironmentManifest) {
    let mut registry = selections()
        .write()
        .expect("Project private overlay registry");
    registry.retain(|_, (project, _, _)| project != &manifest.project_id);
}
pub(crate) fn project_private_file_rules(root: &Path) -> Vec<String> {
    selections()
        .read()
        .expect("Project private overlay registry")
        .get(root)
        .map(|(_, rules, _)| rules.clone())
        .unwrap_or_default()
}

pub(crate) fn project_private_secret_files(root: &Path) -> Vec<String> {
    selections()
        .read()
        .expect("Project private overlay registry")
        .get(root)
        .map(|(_, _, secrets)| secrets.clone())
        .unwrap_or_default()
}

/// Referenced configuration files belong to the sealed environment layer, even
/// when the utility classifies their contents as non-secret. Never copy them a
/// second time in the plain overlay or allow a review flip to create a collision.
pub(crate) fn normalize_project_config_file_decisions(manifest: &mut ProjectEnvironmentManifest) {
    for file in &mut manifest.private_files {
        if manifest.entries.iter().any(|entry| {
            entry.workspace_id == file.workspace_id
                && entry.kind == ProjectEnvironmentEntryKind::ConfigFile
                && entry.name == file.path
        }) {
            file.bring = false;
            file.secret_looking = true;
            file.reason =
                "Referenced configuration; recreated through the sealed environment layer".into();
        }
    }
}

/// A review must never claim to bring a private file that is unavailable on the
/// exporting kernel. A later source fetch must settle before accepting that choice.
pub(crate) fn validate_project_private_files_present(
    manifest: &ProjectEnvironmentManifest,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<(), crate::error::DaemonError> {
    for file in manifest.private_files.iter().filter(|file| file.bring) {
        let root = roots.get(&file.workspace_id).ok_or_else(|| {
            super::resolver::environment_error("private file workspace is unavailable")
        })?;
        let _file = super::resolver::open_workspace_file(root, &file.path)?;
    }
    Ok(())
}
