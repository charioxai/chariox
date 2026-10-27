//! Workspace live sync whole-file move/delete application.

use super::*;

#[derive(Debug, Clone)]
pub(in crate::runtime::state) enum WorkspaceLiveSyncWholeFileOperation {
    Delete {
        path: PathBuf,
    },
    Move {
        from_path: PathBuf,
        to_path: PathBuf,
    },
}

pub(in crate::runtime::state) fn apply_workspace_live_sync_whole_file_operations(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: crate::io::WorkspaceIdentity,
    workspace_root: PathBuf,
    domain: crate::io::ArtifactDomainKind,
    operations: Vec<WorkspaceLiveSyncWholeFileOperation>,
    reservation_owner: crate::io::ArtifactReservationOwner,
    external_change_monitor: &crate::io::ArtifactExternalChangeMonitor,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let mut before_states: BTreeMap<PathBuf, Option<crate::io::ArtifactContent>> = BTreeMap::new();
    let mut final_states: BTreeMap<PathBuf, Option<crate::io::ArtifactContent>> = BTreeMap::new();
    let mut reservation_ranges: BTreeMap<PathBuf, Vec<crate::io::TextRange>> = BTreeMap::new();

    for operation in operations {
        match operation {
            WorkspaceLiveSyncWholeFileOperation::Delete { path } => {
                workspace_live_sync_validate_patch_path(&workspace_root, &path)?;
                let current = workspace_live_sync_whole_file_state(
                    &workspace_root,
                    &path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_none() {
                    return Ok(workspace_live_sync_patch_rejected(
                        path,
                        "delete file target does not exist",
                    ));
                }
                reservation_ranges
                    .entry(path.clone())
                    .or_default()
                    .push(crate::io::TextRange::new(0, usize::MAX));
                final_states.insert(path, None);
            }
            WorkspaceLiveSyncWholeFileOperation::Move { from_path, to_path } => {
                workspace_live_sync_validate_patch_path(&workspace_root, &from_path)?;
                workspace_live_sync_validate_patch_path(&workspace_root, &to_path)?;
                if from_path == to_path {
                    return Ok(workspace_live_sync_patch_rejected(
                        from_path,
                        "move source and target are identical",
                    ));
                }
                let source = workspace_live_sync_whole_file_state(
                    &workspace_root,
                    &from_path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(source) = source else {
                    return Ok(workspace_live_sync_patch_rejected(
                        from_path,
                        "move source does not exist",
                    ));
                };
                let target = workspace_live_sync_whole_file_state(
                    &workspace_root,
                    &to_path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                if target.is_some() {
                    return Ok(workspace_live_sync_patch_rejected(
                        to_path,
                        "move target already exists",
                    ));
                }
                reservation_ranges
                    .entry(from_path.clone())
                    .or_default()
                    .push(crate::io::TextRange::new(0, usize::MAX));
                reservation_ranges
                    .entry(to_path.clone())
                    .or_default()
                    .push(crate::io::TextRange::new(0, usize::MAX));
                final_states.insert(from_path, None);
                final_states.insert(to_path, Some(source));
            }
        }
    }

    let mut reservations = Vec::new();
    for (path, ranges) in reservation_ranges {
        match workspace_live_sync_try_reserve_ranges(
            coordinator,
            &workspace_identity,
            &path,
            ranges,
            reservation_owner.clone(),
        ) {
            Ok(token) => reservations.push(token),
            Err(output) => {
                for token in reservations {
                    coordinator.release_reservation(token);
                }
                return Ok(output);
            }
        }
    }

    let external_change_notices = external_change_monitor
        .external_change_notices(&workspace_identity, final_states.keys().cloned());

    for (path, before) in &before_states {
        let latest = match workspace_live_sync_read_optional_content(&workspace_root, path, domain)
        {
            Ok(latest) => latest,
            Err(error) => {
                for token in reservations {
                    coordinator.release_reservation(token);
                }
                return Err(error);
            }
        };
        if &latest != before {
            for token in reservations {
                coordinator.release_reservation(token);
            }
            external_change_monitor.record_external_change(&workspace_identity, path);
            let mut notices = external_change_notices.clone();
            if !notices.iter().any(|notice| notice.path == *path) {
                notices.push(workspace_live_sync_external_change_notice_for_path(
                    path.clone(),
                ));
            }
            let mut output = workspace_live_sync_patch_rejected(
                path.clone(),
                "artifact changed while the workspace live sync whole-file operation was being prepared; reread and retry",
            );
            add_workspace_live_sync_external_change_notices_payload(&mut output.payload, notices);
            return Ok(output);
        }
    }

    if let Err(error) =
        workspace_live_sync_write_final_content_states(&workspace_root, &final_states)
    {
        let _ = workspace_live_sync_write_final_content_states(&workspace_root, &before_states);
        for token in reservations {
            coordinator.release_reservation(token);
        }
        return Err(error);
    }

    for (path, after) in &final_states {
        match after {
            Some(content) => {
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: content.clone(),
                });
            }
            None => coordinator.forget_artifact(&workspace_identity, path),
        }
        external_change_monitor.observe_managed_write(
            reservation_owner.provider_run_id.as_str(),
            &workspace_identity,
            &workspace_root,
            path,
        );
    }
    for token in reservations {
        coordinator.release_reservation(token);
    }

    let mut changes = Vec::new();
    for (path, after) in final_states {
        let before = before_states.get(&path).cloned().flatten();
        let mut change_payload = serde_json::json!({});
        add_workspace_live_sync_whole_file_change_payload(&mut change_payload, path, before, after);
        if let Some(change) = change_payload.get("change") {
            changes.push(change.clone());
        }
    }

    let mut payload = serde_json::json!({
        "applied": true,
        "atomic": true,
        "changes": changes,
    });
    add_workspace_live_sync_external_change_notices_payload(&mut payload, external_change_notices);
    if changes.len() == 1 {
        payload["change"] = changes[0].clone();
        if let Some(path) = changes[0].get("path").cloned() {
            payload["path"] = path;
        }
    }
    Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
}
