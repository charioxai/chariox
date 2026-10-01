//! MP-08 / MP-10 / MP-11: Value-free review policy shared by TUI and Web.
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentReview {
    pub schema_version: u32,
    pub project_name: String,
    pub target_name: String,
    pub expanded: bool,
    pub unattended: bool,
    pub changed_only: bool,
    pub rows: Vec<ProjectEnvironmentReviewRow>,
    pub files: Vec<ProjectEnvironmentReviewFile>,
    pub inputs: Vec<ProjectEnvironmentReviewInput>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentReviewRow {
    pub label: String,
    pub summary: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentReviewFile {
    pub id: String,
    pub path: String,
    pub bring: bool,
    pub reason: String,
    pub changed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentReviewInput {
    pub id: String,
    pub name: String,
    pub workspace_id: String,
    pub missing: bool,
    pub secret: bool,
    pub source: String,
    pub changed: bool,
    pub uses: Vec<ProjectEnvironmentUse>,
}

pub fn project_environment_item_id(workspace: &str, name: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{workspace}\0{name}").as_bytes())
    )
}
pub fn project_environment_needs_review(state: &StoredProjectEnvironment) -> bool {
    state.reviewed_manifest.as_ref().is_none_or(|old| {
        old.entries != state.manifest.entries
            || old.private_files != state.manifest.private_files
            || old.toolchain_hints != state.manifest.toolchain_hints
            || old.package_hints != state.manifest.package_hints
            || old.service_hints != state.manifest.service_hints
    })
}

impl ProjectEnvironmentReview {
    pub fn build(
        state: &StoredProjectEnvironment,
        project: &str,
        target: &str,
        code: String,
        expanded: bool,
        unattended: bool,
    ) -> Self {
        let old = state.reviewed_manifest.as_ref();
        let manifest = &state.manifest;
        let brought = manifest.private_files.iter().filter(|f| f.bring).count();
        let selected: Vec<_> = manifest
            .entries
            .iter()
            .filter(|entry| !entry.excluded)
            .collect();
        let secret = selected
            .iter()
            .filter(|entry| entry.classification == ProjectEnvironmentClassification::Secret)
            .count();
        let mut sources: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for entry in &selected {
            sources.insert(locator_source(&entry.locator));
        }
        Self {
            schema_version: 1,
            project_name: project.into(),
            target_name: target.into(),
            expanded,
            unattended,
            changed_only: old.is_some(),
            rows: vec![
                ProjectEnvironmentReviewRow {
                    label: "Code".into(),
                    summary: code,
                },
                ProjectEnvironmentReviewRow {
                    label: "Environment".into(),
                    summary: format!(
                        "{} variables, {} secrets hidden · {}",
                        selected.len(),
                        secret,
                        sources.into_iter().collect::<Vec<_>>().join(", ")
                    ),
                },
                ProjectEnvironmentReviewRow {
                    label: "Files".into(),
                    summary: format!(
                        "bringing {} / leaving {}",
                        brought,
                        manifest.private_files.len() - brought
                    ),
                },
                ProjectEnvironmentReviewRow {
                    label: "Setup".into(),
                    summary: manifest
                        .toolchain_hints
                        .iter()
                        .chain(&manifest.package_hints)
                        .chain(&manifest.service_hints)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                },
            ],
            files: manifest
                .private_files
                .iter()
                .map(|file| ProjectEnvironmentReviewFile {
                    id: project_environment_item_id(&file.workspace_id, &file.path),
                    path: file.path.clone(),
                    bring: file.bring,
                    reason: file.reason.clone(),
                    changed: old.is_none_or(|old| !old.private_files.contains(file)),
                })
                .collect(),
            inputs: manifest
                .entries
                .iter()
                .map(|entry| ProjectEnvironmentReviewInput {
                    id: project_environment_item_id(&entry.workspace_id, &entry.name),
                    name: entry.name.clone(),
                    workspace_id: entry.workspace_id.clone(),
                    missing: !entry.excluded
                        && entry.status != ProjectEnvironmentEntryStatus::Found,
                    secret: entry.classification == ProjectEnvironmentClassification::Secret,
                    source: if entry.excluded {
                        "left behind".into()
                    } else {
                        locator_source(&entry.locator)
                    },
                    changed: old.is_none_or(|old| !old.entries.contains(entry)),
                    uses: entry.uses.clone(),
                })
                .collect(),
        }
    }
}
fn locator_source(locator: &ProjectEnvironmentLocator) -> String {
    match locator {
        ProjectEnvironmentLocator::EnvFile { path, .. }
        | ProjectEnvironmentLocator::ConfigFile { path } => path.clone(),
        ProjectEnvironmentLocator::WorkspaceEnvironment { .. } => "workspace shell".into(),
        ProjectEnvironmentLocator::Vault { .. } => "Project Vault".into(),
        ProjectEnvironmentLocator::Missing => "not found".into(),
    }
}

pub fn flip_project_environment_item(
    state: &mut StoredProjectEnvironment,
    id: &str,
    file: bool,
) -> Result<(), DaemonError> {
    let fail = super::resolver::environment_error;
    if file {
        if state.manifest.entries.iter().any(|entry| entry.kind == ProjectEnvironmentEntryKind::ConfigFile
            && project_environment_item_id(&entry.workspace_id, &entry.name) == id) {
            return Err(fail("referenced configuration uses the sealed environment layer"));
        }
        let decision = state
            .manifest
            .private_files
            .iter_mut()
            .find(|f| project_environment_item_id(&f.workspace_id, &f.path) == id)
            .ok_or_else(|| fail("unknown Project file decision"))?;
        if !decision.bring
            && (decision.secret_looking
                || secret_looking_project_path(&decision.path)
                || crate::workspace_live_sync_ignore::workspace_live_sync_force_excluded_path(
                    &decision.path,
                ))
        {
            return Err(fail(
                "secret and runtime files cannot enter the plain overlay",
            ));
        }
        decision.bring = !decision.bring;
        decision.reason = "Your export review choice".into();
    } else {
        let entry = state
            .manifest
            .entries
            .iter_mut()
            .find(|e| project_environment_item_id(&e.workspace_id, &e.name) == id)
            .ok_or_else(|| fail("unknown Project input decision"))?;
        entry.excluded = !entry.excluded;
    }
    Ok(())
}

/// Only changed/new decisions are editable by incremental discovery; saved explicit choices survive.
pub fn preserve_project_environment_choices(
    manifest: &mut ProjectEnvironmentManifest,
    previous: &ProjectEnvironmentManifest,
    changed: &std::collections::BTreeMap<String, Vec<String>>,
) {
    for file in &mut manifest.private_files {
        if changed
            .get(&file.workspace_id)
            .is_none_or(|paths| !paths.contains(&file.path))
        {
            if let Some(old) = previous
                .private_files
                .iter()
                .find(|old| old.workspace_id == file.workspace_id && old.path == file.path)
            {
                *file = old.clone();
            }
        }
    }
    for entry in &mut manifest.entries {
        if let Some(old) = previous
            .entries
            .iter()
            .find(|old| old.workspace_id == entry.workspace_id && old.name == entry.name)
        {
            entry.excluded = old.excluded;
        }
    }
}

pub fn accept_project_environment_review(
    state: &mut StoredProjectEnvironment,
    review: ProjectEnvironmentReview,
) {
    state.reported_missing.extend(
        state
            .manifest
            .entries
            .iter()
            .filter(|entry| entry.status != ProjectEnvironmentEntryStatus::Found)
            .map(|entry| (entry.workspace_id.clone(), entry.name.clone())),
    );
    state.reviewed_manifest = Some(state.manifest.clone());
    state.last_review = Some(review);
}
