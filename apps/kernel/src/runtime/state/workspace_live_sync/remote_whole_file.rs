//! Remote workspace live sync whole-file move/delete application.

use super::*;

pub(in crate::runtime::state) fn apply_remote_managed_whole_file_operations(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: crate::io::WorkspaceIdentity,
    domain: crate::io::ArtifactDomainKind,
    operations: Vec<ManagedWholeFileOperation>,
    artifact_states: Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
    reservation_owner: crate::io::ArtifactReservationOwner,
    workspace_context: &WorkspaceLiveSyncWorkspaceContext,
) -> Result<
    (
        crate::transport::runtime_tools::RuntimeToolResult,
        Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
    ),
    DaemonError,
> {
    let mut before_states: BTreeMap<PathBuf, Option<crate::io::ArtifactContent>> = BTreeMap::new();
    let mut final_states: BTreeMap<PathBuf, Option<crate::io::ArtifactContent>> = BTreeMap::new();
    let mut reservation_ranges: BTreeMap<PathBuf, Vec<crate::io::TextRange>> = BTreeMap::new();

    for state in &artifact_states {
        let path = PathBuf::from(&state.path);
        let content = remote_workspace_live_sync_content_from_state(state, domain)?;
        coordinator.read_artifact(crate::io::ArtifactReadRequest {
            workspace_identity: workspace_identity.clone(),
            path,
            domain,
            content,
        });
    }

    for operation in operations {
        match operation {
            ManagedWholeFileOperation::Delete { path } => {
                workspace_live_sync_validate_patch_path(&workspace_context.root, &path)?;
                let current = remote_managed_whole_file_state(
                    &artifact_states,
                    &path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_none() {
                    return Ok((
                        managed_patch_rejected(path, "delete file target does not exist"),
                        Vec::new(),
                    ));
                }
                reservation_ranges
                    .entry(path.clone())
                    .or_default()
                    .push(crate::io::TextRange::new(0, usize::MAX));
                final_states.insert(path, None);
            }
            ManagedWholeFileOperation::Move { from_path, to_path } => {
                workspace_live_sync_validate_patch_path(&workspace_context.root, &from_path)?;
                workspace_live_sync_validate_patch_path(&workspace_context.root, &to_path)?;
                if from_path == to_path {
                    return Ok((
                        managed_patch_rejected(from_path, "move source and target are identical"),
                        Vec::new(),
                    ));
                }
                let source = remote_managed_whole_file_state(
                    &artifact_states,
                    &from_path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(source) = source else {
                    return Ok((
                        managed_patch_rejected(from_path, "move source does not exist"),
                        Vec::new(),
                    ));
                };
                let target = remote_managed_whole_file_state(
                    &artifact_states,
                    &to_path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                if target.is_some() {
                    return Ok((
                        managed_patch_rejected(to_path, "move target already exists"),
                        Vec::new(),
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
            Err(mut result) => {
                for token in reservations {
                    coordinator.release_reservation(token);
                }
                add_workspace_live_sync_workspace_payload(&mut result.payload, workspace_context);
                return Ok((result, Vec::new()));
            }
        }
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
    }
    for token in reservations {
        coordinator.release_reservation(token);
    }

    let mut changes = Vec::new();
    for (path, after) in &final_states {
        let before = before_states.get(path).cloned().flatten();
        let mut change_payload = serde_json::json!({});
        add_workspace_live_sync_whole_file_change_payload(
            &mut change_payload,
            path.clone(),
            before,
            after.clone(),
        );
        if let Some(change) = change_payload.get("change") {
            changes.push(change.clone());
        }
    }

    let mut payload = serde_json::json!({
        "applied": true,
        "atomic": true,
        "changes": changes,
    });
    if changes.len() == 1 {
        payload["change"] = changes[0].clone();
        if let Some(path) = changes[0].get("path").cloned() {
            payload["path"] = path;
        }
    }
    add_workspace_live_sync_workspace_payload(&mut payload, workspace_context);
    let final_artifact_states = final_states
        .into_iter()
        .map(|(path, content)| {
            remote_workspace_live_sync_state_from_content_with_domain(&path, content, domain)
        })
        .collect::<Vec<_>>();
    Ok((
        crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
        final_artifact_states,
    ))
}

fn remote_managed_whole_file_state(
    artifact_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    path: &PathBuf,
    domain: crate::io::ArtifactDomainKind,
    before_states: &mut BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
    final_states: &mut BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
) -> Result<Option<crate::io::ArtifactContent>, DaemonError> {
    if let Some(current) = final_states.get(path) {
        return Ok(current.clone());
    }
    let state = remote_workspace_live_sync_state_for_path(artifact_states, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "remote_workspace_live_sync_whole_file_state",
            message: format!(
                "missing forwarded artifact state for `{}`",
                path.to_string_lossy()
            ),
        }
    })?;
    let current = state
        .exists
        .then(|| remote_workspace_live_sync_content_from_state(state, domain))
        .transpose()?;
    before_states
        .entry(path.clone())
        .or_insert_with(|| current.clone());
    final_states.insert(path.clone(), current.clone());
    Ok(current)
}
