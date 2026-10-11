//! MP-08 / MP-10 / MP-11: scoped Apply delegates to the existing manifest and launch resolver.
use super::*;
use crate::{config::DaemonConfig, error::DaemonError};
use std::collections::BTreeMap;

pub(crate) fn apply_scoped_binding(
    store: &ProjectEnvironmentStore,
    config: &DaemonConfig,
    env: &ProjectEnvironment,
    r: &Requirement,
) -> Result<(), DaemonError> {
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
    if project_environment_protected_name(name) {
        return Err(environment_error("Protected launch binding"));
    }
    let evidence = ProjectEnvironmentEvidence::default();
    let mut stored =
        store
            .load(&env.local_project_id)?
            .unwrap_or_else(|| StoredProjectEnvironment {
                apply_owned_bindings: Default::default(),
                source: None,
                manifest: ProjectEnvironmentManifest {
                    schema_version: 1,
                    project_id: env.local_project_id.clone(),
                    evidence_digest: evidence.digest(),
                    entries: vec![],
                    private_files: vec![],
                    toolchain_hints: vec![],
                    package_hints: vec![],
                    service_hints: vec![],
                },
                evidence,
                reported_missing: Default::default(),
                reviewed_manifest: None,
                last_review: None,
            });
    let folders: Vec<_> = env
        .folders
        .iter()
        .filter(|f| {
            r.scope == RequirementScope::Project
                || r.scope
                    == (RequirementScope::Folder {
                        folder_id: f.folder_id.clone(),
                    })
        })
        .collect();
    if folders.is_empty() || folders.iter().any(|f| f.local_workspace_binding.is_empty()) {
        return Err(environment_error("Apply folder is unavailable"));
    }
    reconcile_owned_bindings(&mut stored, env, &[r.scope.clone()]);
    // Resolve each selected folder independently. Project-wide declarations do
    // not borrow the first folder's value or leak a sibling Vault binding.
    for folder in folders {
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
        let owned = stored.manifest.entries.iter().any(|e| {
            e.workspace_id == entry.workspace_id
                && e.name == entry.name
                && binding_owned_by(&stored, r, e)
        });
        if stored.manifest.entries.iter().any(|e| {
            e.workspace_id == entry.workspace_id
                && e.name == entry.name
                && e.locator != entry.locator
                && !e.excluded
                && !binding_owned_by(&stored, r, e)
        }) {
            return Err(environment_error(
                "Existing scoped launch binding conflicts",
            ));
        }
        let single = ProjectEnvironmentManifest {
            entries: vec![entry.clone()],
            ..stored.manifest.clone()
        };
        let root = std::path::PathBuf::from(&entry.workspace_id);
        let inherited = match &locator {
            ProjectEnvironmentLocator::WorkspaceEnvironment { name } => std::env::var(name)
                .ok()
                .map(|v| BTreeMap::from([(name.clone(), zeroize::Zeroizing::new(v))]))
                .unwrap_or_default(),
            _ => BTreeMap::new(),
        };
        let vault = crate::secret::project_environment_vault(config)?;
        let resolved = resolve_project_environment(
            &single,
            &BTreeMap::from([(entry.workspace_id.clone(), root)]),
            &BTreeMap::from([(entry.workspace_id.clone(), inherited)]),
            vault.as_ref(),
        )?;
        if !resolved
            .values
            .contains_key(&(entry.workspace_id.clone(), name.clone()))
        {
            return Err(environment_error("Scoped binding is unavailable"));
        }
        // MP-11: close the Check/resolution race without projecting a value hash.
        if classification == ProjectEnvironmentClassification::NonSecret
            && resolved
                .values
                .values()
                .any(|value| super::detect_index::credential_content(value))
        {
            return Err(environment_error(
                "Scoped public input requires Vault classification",
            ));
        }
        // Never acquire ownership of an unrelated legacy binding just because
        // its locator happens to match. Ownership and entry publish together.
        let legacy =
            stored.manifest.entries.iter().any(|e| {
                e.workspace_id == entry.workspace_id && e.name == entry.name && !e.excluded
            });
        if owned || !legacy {
            stored.apply_owned_bindings.retain(|o| {
                o.entry.workspace_id != entry.workspace_id || o.entry.name != entry.name
            });
            stored
                .apply_owned_bindings
                .push(super::store::ApplyOwnedBinding {
                    requirement_id: r.requirement_id.clone(),
                    scope: r.scope.clone(),
                    entry: entry.clone(),
                });
        }
        stored
            .manifest
            .entries
            .retain(|e| e.workspace_id != entry.workspace_id || e.name != entry.name);
        stored.manifest.entries.push(entry);
    }
    store.save(&stored)
}

// MP-08 / MP-10 / MP-11: reconcile only scopes included in this reviewed Apply.
fn reconcile_owned_bindings(
    stored: &mut StoredProjectEnvironment,
    env: &ProjectEnvironment,
    scopes: &[RequirementScope],
) {
    let entries = &mut stored.manifest.entries;
    stored.apply_owned_bindings.retain(|owner| {
        let scope_selected = scopes.contains(&owner.scope);
        let declared = environment_requirements(env).any(|r| {
            r.requirement_id == owner.requirement_id
                && r.scope == owner.scope
                && match &r.spec {
                    RequirementSpec::Variables { name, .. }
                    | RequirementSpec::Secrets { name, .. } => {
                        name == &owner.entry.name
                            && env.folders.iter().any(|f| {
                                f.local_workspace_binding == owner.entry.workspace_id
                                    && (r.scope == RequirementScope::Project
                                        || r.scope
                                            == RequirementScope::Folder {
                                                folder_id: f.folder_id.clone(),
                                            })
                            })
                    }
                    _ => false,
                }
        });
        if scope_selected && !declared {
            entries.retain(|entry| entry != &owner.entry);
            false
        } else {
            true
        }
    });
}
pub(crate) fn retire_removed_apply_bindings(
    store: &ProjectEnvironmentStore,
    env: &ProjectEnvironment,
    scopes: &[RequirementScope],
) -> Result<(), DaemonError> {
    if let Some(mut stored) = store.load(&env.local_project_id)? {
        reconcile_owned_bindings(&mut stored, env, scopes);
        store.save(&stored)?;
    }
    Ok(())
}

// MP-11: reviewed replacement cannot seize another requirement or legacy authority.
fn binding_owned_by(
    stored: &StoredProjectEnvironment,
    requirement: &Requirement,
    entry: &ProjectEnvironmentEntry,
) -> bool {
    stored.apply_owned_bindings.iter().any(|owner| {
        owner.requirement_id == requirement.requirement_id
            && owner.scope == requirement.scope
            && owner.entry == *entry
    })
}
