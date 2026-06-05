use super::*;

pub(super) fn workspace_live_sync_runtime_tool_applied(
    output: &crate::transport::runtime_tools::RuntimeToolResult,
) -> bool {
    output.ok
        && output
            .payload
            .get("applied")
            .and_then(|value| value.as_bool())
            == Some(true)
}

pub(super) fn workspace_live_sync_managed_mode_before_snapshots(
    workspace_root: &PathBuf,
    operations: &[ManagedPatchOperation],
) -> Result<BTreeMap<PathBuf, Option<Vec<u8>>>, DaemonError> {
    let mut snapshots = BTreeMap::new();
    for path in operations
        .iter()
        .flat_map(workspace_live_sync_managed_mode_patch_operation_paths)
    {
        if snapshots.contains_key(&path) {
            continue;
        }
        snapshots.insert(
            path.clone(),
            workspace_live_sync_managed_read_optional_bytes(workspace_root, &path)?,
        );
    }
    Ok(snapshots)
}

fn workspace_live_sync_managed_mode_patch_operation_paths(
    operation: &ManagedPatchOperation,
) -> Vec<PathBuf> {
    match operation {
        ManagedPatchOperation::Add { path, .. }
        | ManagedPatchOperation::Update { path, .. }
        | ManagedPatchOperation::Delete { path } => vec![path.clone()],
        ManagedPatchOperation::Move {
            from_path, to_path, ..
        } => vec![from_path.clone(), to_path.clone()],
    }
}

pub(super) fn workspace_live_sync_managed_mode_patch_file_changes(
    workspace_root: &PathBuf,
    operations: &[ManagedPatchOperation],
    before_snapshots: BTreeMap<PathBuf, Option<Vec<u8>>>,
) -> Result<Vec<crate::git_observer::WorkspaceLiveSyncFileChange>, DaemonError> {
    operations
        .iter()
        .map(|operation| match operation {
            ManagedPatchOperation::Add { path, .. }
            | ManagedPatchOperation::Update { path, .. }
            | ManagedPatchOperation::Delete { path } => {
                Ok(workspace_live_sync_managed_mode_file_change(
                    path.clone(),
                    None,
                    before_snapshots.get(path).cloned().flatten(),
                    workspace_live_sync_managed_read_optional_bytes(workspace_root, path)?,
                ))
            }
            ManagedPatchOperation::Move {
                from_path, to_path, ..
            } => Ok(workspace_live_sync_managed_mode_file_change(
                to_path.clone(),
                Some(from_path.clone()),
                before_snapshots.get(from_path).cloned().flatten(),
                workspace_live_sync_managed_read_optional_bytes(workspace_root, to_path)?,
            )),
        })
        .collect()
}

pub(super) fn workspace_live_sync_managed_mode_remote_tool_file_changes(
    tool_name: &str,
    arguments: &serde_json::Value,
    initial_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    final_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
) -> Result<Vec<crate::git_observer::WorkspaceLiveSyncFileChange>, DaemonError> {
    match tool_name {
        crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncEditArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_workspace_live_sync_fanout",
                message: format!("invalid edit arguments for fanout: {error}"),
            })?;
            remote_state_file_changes_for_paths(
                vec![(PathBuf::from(args.path), None)],
                initial_states,
                final_states,
            )
        }
        crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncWriteArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_workspace_live_sync_fanout",
                message: format!("invalid write arguments for fanout: {error}"),
            })?;
            remote_state_file_changes_for_paths(
                vec![(PathBuf::from(args.path), None)],
                initial_states,
                final_states,
            )
        }
        crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncApplyPatchArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_workspace_live_sync_fanout",
                message: format!("invalid apply_patch arguments for fanout: {error}"),
            })?;
            let operations = parse_workspace_live_sync_apply_patch(&args.patch_text)?;
            let paths = operations
                .into_iter()
                .map(|operation| match operation {
                    ManagedPatchOperation::Add { path, .. }
                    | ManagedPatchOperation::Update { path, .. }
                    | ManagedPatchOperation::Delete { path } => (path, None),
                    ManagedPatchOperation::Move {
                        from_path, to_path, ..
                    } => (to_path, Some(from_path)),
                })
                .collect::<Vec<_>>();
            remote_state_file_changes_for_paths(paths, initial_states, final_states)
        }
        crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncDeleteArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_workspace_live_sync_fanout",
                message: format!("invalid delete arguments for fanout: {error}"),
            })?;
            remote_state_file_changes_for_paths(
                vec![(PathBuf::from(args.path), None)],
                initial_states,
                final_states,
            )
        }
        crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncMoveArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_workspace_live_sync_fanout",
                message: format!("invalid move arguments for fanout: {error}"),
            })?;
            remote_state_file_changes_for_paths(
                vec![(
                    PathBuf::from(args.to_path),
                    Some(PathBuf::from(args.from_path)),
                )],
                initial_states,
                final_states,
            )
        }
        _ => Ok(Vec::new()),
    }
}

fn remote_state_file_changes_for_paths(
    paths: Vec<(PathBuf, Option<PathBuf>)>,
    initial_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    final_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
) -> Result<Vec<crate::git_observer::WorkspaceLiveSyncFileChange>, DaemonError> {
    paths
        .into_iter()
        .map(|(path, previous_path)| {
            let before_path = previous_path.as_ref().unwrap_or(&path);
            let before =
                remote_workspace_live_sync_optional_state_bytes(initial_states, before_path)?;
            let after = remote_workspace_live_sync_optional_state_bytes(final_states, &path)?;
            Ok(workspace_live_sync_managed_mode_file_change(
                path,
                previous_path,
                before,
                after,
            ))
        })
        .collect()
}

fn remote_workspace_live_sync_optional_state_bytes(
    states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    path: &PathBuf,
) -> Result<Option<Vec<u8>>, DaemonError> {
    match remote_workspace_live_sync_state_for_path(states, path) {
        Some(state) => remote_workspace_live_sync_state_bytes(state),
        None => Ok(None),
    }
}

fn remote_workspace_live_sync_state_bytes(
    state: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState,
) -> Result<Option<Vec<u8>>, DaemonError> {
    if !state.exists {
        return Ok(None);
    }
    match remote_workspace_live_sync_content_from_state(
        state,
        remote_workspace_live_sync_state_domain(state),
    )? {
        crate::io::ArtifactContent::Text(text) => Ok(Some(text.into_bytes())),
        crate::io::ArtifactContent::Bytes(bytes) => Ok(Some(bytes)),
    }
}

pub(super) fn workspace_live_sync_managed_mode_file_change(
    path: PathBuf,
    previous_path: Option<PathBuf>,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
) -> crate::git_observer::WorkspaceLiveSyncFileChange {
    let kind = match (&previous_path, before.as_ref(), after.as_ref()) {
        (Some(_), _, _) => crate::git_observer::WorkspaceLiveSyncFileChangeKind::Renamed,
        (None, None, Some(_)) => crate::git_observer::WorkspaceLiveSyncFileChangeKind::Added,
        (None, Some(_), None) => crate::git_observer::WorkspaceLiveSyncFileChangeKind::Deleted,
        _ => crate::git_observer::WorkspaceLiveSyncFileChangeKind::Modified,
    };
    let binary = before.as_ref().is_some_and(|bytes| bytes.contains(&0))
        || after.as_ref().is_some_and(|bytes| bytes.contains(&0));
    crate::git_observer::WorkspaceLiveSyncFileChange {
        path: path.to_string_lossy().to_string(),
        previous_path: previous_path.map(|path| path.to_string_lossy().to_string()),
        kind,
        before_content_base64: before
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
        after_content_base64: after
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
        binary,
    }
}

pub(super) fn workspace_live_sync_managed_read_optional_bytes(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Result<Option<Vec<u8>>, DaemonError> {
    workspace_live_sync_validate_patch_path(workspace_root, path)?;
    let full_path = workspace_root.join(path);
    match std::fs::read(&full_path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonError::LocalTransport {
            operation: "managed_workspace_live_sync_fanout",
            message: format!(
                "failed to read `{}` for fanout: {error}",
                path.to_string_lossy()
            ),
        }),
    }
}
