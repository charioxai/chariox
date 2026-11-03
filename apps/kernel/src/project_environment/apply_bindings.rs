//! MP-08 / MP-10 / MP-11: scoped Apply delegates to the existing manifest and launch resolver.
use super::*;
#[cfg(test)]
use crate::config::DaemonConfig;
use crate::error::DaemonError;

#[cfg(test)]
pub(crate) fn apply_scoped_binding(
    store: &ProjectEnvironmentStore,
    config: &DaemonConfig,
    env: &ProjectEnvironment,
    r: &Requirement,
) -> Result<(), DaemonError> {
    let prepared = prepare_scoped_binding(
        config,
        env,
        r,
        std::time::Instant::now() + std::time::Duration::from_secs(60),
        || false,
    )?;
    publish_scoped_binding(store, env, r, prepared)
}
pub(crate) fn publish_scoped_binding(
    store: &ProjectEnvironmentStore,
    env: &ProjectEnvironment,
    r: &Requirement,
    prepared: PreparedScopedBinding,
) -> Result<(), DaemonError> {
    let entries = prepared.into_entries(env, r)?;
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
    reconcile_owned_bindings(&mut stored, env, std::slice::from_ref(&r.scope));
    // Publish only independently validated folder entries; values remain with the resolver.
    for entry in entries {
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
            entries.retain(|entry| !same_binding_authority(entry, &owner.entry));
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
            && same_binding_authority(entry, &owner.entry)
    })
}

// MP-08 / MP-10 / MP-11: availability observations cannot change ownership.
// Every other field still fences independent selection, scope and locator edits.
fn same_binding_authority(
    entry: &ProjectEnvironmentEntry,
    owned: &ProjectEnvironmentEntry,
) -> bool {
    let mut observed = entry.clone();
    observed.status = owned.status;
    observed == *owned
}

// MP-08 / MP-10: supplementary publication-only mounted-I/O gate.
#[cfg(test)]
pub(crate) fn publication_probe_test_gate() {
    if let Some(marker) = std::env::var_os("CHARIOX_APPLY_BINDING_STALL_MARKER") {
        let marker = std::path::PathBuf::from(marker);
        std::fs::write(&marker, std::process::id().to_string()).unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.with_extension("release").exists() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
