//! MP-08 / MP-10 / MP-11: fresh value-free binding proof, outside the Project lock.
use super::*;
use crate::{config::DaemonConfig, error::DaemonError};
use std::{collections::BTreeMap, path::Path, time::Instant};

pub(crate) struct PreparedScopedBinding {
    authority: String,
    entries: Vec<ProjectEnvironmentEntry>,
}
fn authority(env: &ProjectEnvironment, r: &Requirement) -> String {
    metadata_digest(&(
        &env.local_project_id,
        &env.content_digest,
        folder_bindings_digest(env),
        r,
    ))
}
impl PreparedScopedBinding {
    pub(super) fn into_entries(
        self,
        env: &ProjectEnvironment,
        r: &Requirement,
    ) -> Result<Vec<ProjectEnvironmentEntry>, DaemonError> {
        if self.authority != authority(env, r) {
            return Err(environment_error("Scoped binding proof changed"));
        }
        Ok(self.entries)
    }
}
pub(crate) fn prepare_scoped_binding(
    config: &DaemonConfig,
    env: &ProjectEnvironment,
    r: &Requirement,
    deadline: Instant,
    mut cancelled: impl FnMut() -> bool,
) -> Result<PreparedScopedBinding, DaemonError> {
    let (name, locator, classification) = match &r.spec {
        RequirementSpec::Variables { name, locator } => (
            name,
            locator.clone(),
            ProjectEnvironmentClassification::NonSecret,
        ),
        RequirementSpec::Secrets {
            name,
            vault: Some(v),
        } => (
            name,
            ProjectEnvironmentLocator::Vault {
                service: v.service.clone(),
                key: v.key.clone(),
            },
            ProjectEnvironmentClassification::Secret,
        ),
        _ => {
            return Err(environment_error(
                "Scoped binding requires an authorized locator",
            ))
        }
    };
    if project_environment_protected_name(name)
        || (classification == ProjectEnvironmentClassification::NonSecret
            && (project_environment_public_name_protected(name)
                || project_environment_public_source_protected(&locator)))
    {
        return Err(environment_error("Protected launch binding"));
    }
    let mut entries = vec![];
    for folder in env.folders.iter().filter(|f| {
        r.scope == RequirementScope::Project
            || r.scope
                == (RequirementScope::Folder {
                    folder_id: f.folder_id.clone(),
                })
    }) {
        if folder.local_workspace_binding.is_empty() || cancelled() || Instant::now() >= deadline {
            return Err(environment_error("Scoped binding validation unavailable"));
        }
        let entry = ProjectEnvironmentEntry {
            name: name.clone(),
            workspace_id: folder.local_workspace_binding.clone(),
            kind: ProjectEnvironmentEntryKind::Variable,
            classification,
            excluded: false,
            uses: vec![],
            locator: locator.clone(),
            status: ProjectEnvironmentEntryStatus::Found,
        };
        match &locator {
            ProjectEnvironmentLocator::EnvFile { .. }
            | ProjectEnvironmentLocator::ConfigFile { .. } => {
                let facts = super::check_files::publication_binding(
                    config,
                    Path::new(&entry.workspace_id),
                    &locator,
                    deadline,
                    &mut cancelled,
                )?;
                if !facts.present || facts.needs_vault {
                    return Err(environment_error(
                        "Scoped public input is unavailable or requires Vault classification",
                    ));
                }
            }
            ProjectEnvironmentLocator::WorkspaceEnvironment { name } => {
                let facts = super::check_binding::summarize(
                    std::env::var(name).ok().map(zeroize::Zeroizing::new),
                );
                if !facts.present || facts.needs_vault {
                    return Err(environment_error(
                        "Scoped public input is unavailable or requires Vault classification",
                    ));
                }
            }
            ProjectEnvironmentLocator::Missing => {
                return Err(environment_error("Scoped binding is unavailable"))
            }
            ProjectEnvironmentLocator::Vault { .. } => {
                let evidence = ProjectEnvironmentEvidence::default();
                let manifest = ProjectEnvironmentManifest {
                    schema_version: 1,
                    project_id: env.local_project_id.clone(),
                    evidence_digest: evidence.digest(),
                    entries: vec![entry.clone()],
                    private_files: vec![],
                    toolchain_hints: vec![],
                    package_hints: vec![],
                    service_hints: vec![],
                };
                let vault = crate::secret::project_environment_vault(config)?;
                let resolved = resolve_project_environment(
                    &manifest,
                    &BTreeMap::from([(
                        entry.workspace_id.clone(),
                        entry.workspace_id.clone().into(),
                    )]),
                    &BTreeMap::new(),
                    vault.as_ref(),
                )?;
                if !resolved
                    .values
                    .contains_key(&(entry.workspace_id.clone(), name.clone()))
                {
                    return Err(environment_error("Scoped binding is unavailable"));
                }
            }
        }
        entries.push(entry);
    }
    if entries.is_empty() || cancelled() || Instant::now() >= deadline {
        return Err(environment_error("Scoped binding validation unavailable"));
    }
    Ok(PreparedScopedBinding {
        authority: authority(env, r),
        entries,
    })
}
