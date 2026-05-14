//! Remote managed-I/O coordination snapshots.
//!
//! Remote agents use these helpers to discover current artifact state and workspace identity so
//! same-repo/same-branch workers can participate in the same collision-control protocol.

use super::*;

pub(in crate::runtime::state) fn remote_managed_io_artifact_states_for_tool(
    workspace_root: &PathBuf,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Result<Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>, DaemonError> {
    match tool_name {
        crate::transport::runtime_tools::READ_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedReadArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_io_read_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let path = PathBuf::from(args.path);
            let domain =
                KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
            let content = managed_io_read_optional_content(workspace_root, &path, domain)?;
            Ok(vec![remote_managed_io_state_from_content_with_domain(
                &path, content, domain,
            )])
        }
        crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedEditArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_io_edit_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let path = PathBuf::from(args.path);
            let content = managed_io_read_optional_text(workspace_root, &path)?;
            Ok(vec![remote_managed_io_state(&path, content)])
        }
        crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedWriteArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_io_write_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let path = PathBuf::from(args.path);
            let domain =
                KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
            let content = managed_io_read_optional_content(workspace_root, &path, domain)?;
            Ok(vec![remote_managed_io_state_from_content_with_domain(
                &path, content, domain,
            )])
        }
        crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedApplyPatchArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_io_apply_patch_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let operations = parse_managed_apply_patch(&args.patch_text)?;
            remote_managed_io_states_for_patch_operations(workspace_root, &operations)
        }
        crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedDeleteArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_io_delete_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let path = PathBuf::from(args.path);
            let domain =
                KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
            let content = managed_io_read_optional_content(workspace_root, &path, domain)?;
            Ok(vec![remote_managed_io_state_from_content_with_domain(
                &path, content, domain,
            )])
        }
        crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::ManagedMoveArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_managed_io_move_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let domain =
                KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
            if domain == crate::io::ArtifactDomainKind::TextDocument {
                let operations = vec![ManagedPatchOperation::Move {
                    from_path: PathBuf::from(args.from_path),
                    to_path: PathBuf::from(args.to_path),
                    old_text: args.old_text,
                    new_text: args.new_text,
                }];
                remote_managed_io_states_for_patch_operations(workspace_root, &operations)
            } else {
                let from_path = PathBuf::from(args.from_path);
                let to_path = PathBuf::from(args.to_path);
                let from_content =
                    managed_io_read_optional_content(workspace_root, &from_path, domain)?;
                let to_content =
                    managed_io_read_optional_content(workspace_root, &to_path, domain)?;
                Ok(vec![
                    remote_managed_io_state_from_content_with_domain(
                        &from_path,
                        from_content,
                        domain,
                    ),
                    remote_managed_io_state_from_content_with_domain(&to_path, to_content, domain),
                ])
            }
        }
        _ => Ok(Vec::new()),
    }
}

pub(in crate::runtime::state) fn remote_managed_io_states_for_patch_operations(
    workspace_root: &PathBuf,
    operations: &[ManagedPatchOperation],
) -> Result<Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>, DaemonError> {
    let mut paths = BTreeSet::new();
    for operation in operations {
        match operation {
            ManagedPatchOperation::Add { path, .. }
            | ManagedPatchOperation::Update { path, .. }
            | ManagedPatchOperation::Delete { path } => {
                paths.insert(path.clone());
            }
            ManagedPatchOperation::Move {
                from_path, to_path, ..
            } => {
                paths.insert(from_path.clone());
                paths.insert(to_path.clone());
            }
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let content = managed_io_read_optional_text(workspace_root, &path)?;
            Ok(remote_managed_io_state(&path, content))
        })
        .collect()
}

pub(in crate::runtime::state) fn apply_remote_managed_io_final_states(
    workspace_root: &PathBuf,
    initial_states: &[crate::transport::relay_peer::RemoteManagedIoArtifactState],
    final_states: &[crate::transport::relay_peer::RemoteManagedIoArtifactState],
) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
    for final_state in final_states {
        let path = PathBuf::from(&final_state.path);
        let initial = remote_managed_io_state_for_path(initial_states, &path);
        let domain = remote_managed_io_state_domain(final_state);
        let current = remote_managed_io_state_from_content_with_domain(
            &path,
            managed_io_read_optional_content(workspace_root, &path, domain)?,
            domain,
        );
        let expected = initial.cloned().unwrap_or_else(|| {
            remote_managed_io_state_from_content_with_domain(&path, None, domain)
        });
        if !remote_managed_io_states_content_equal(&current, &expected) {
            return Ok(Some(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "applied": false,
                    "reason": {
                        "kind": "external_change_during_remote_apply",
                        "path": path.to_string_lossy(),
                        "message": "The remote workspace artifact changed after the home kernel accepted the managed I/O operation but before the worker could apply it."
                    },
                    "next_action": "Reread the artifact with arroba.read_artifact, reconcile with the current content, and retry through Arroba managed I/O.",
                }),
            }));
        }
    }
    let mut states = BTreeMap::new();
    for state in final_states {
        let path = PathBuf::from(&state.path);
        let content = if state.exists {
            Some(remote_managed_io_content_from_state(
                state,
                remote_managed_io_state_domain(state),
            )?)
        } else {
            None
        };
        states.insert(path, content);
    }
    managed_io_write_final_content_states(workspace_root, &states)?;
    Ok(None)
}

pub(in crate::runtime::state) fn apply_remote_managed_patch_operations(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: crate::io::WorkspaceIdentity,
    domain: crate::io::ArtifactDomainKind,
    operations: Vec<ManagedPatchOperation>,
    artifact_states: Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    reservation_owner: crate::io::ArtifactReservationOwner,
    workspace_context: &ManagedIoWorkspaceContext,
) -> Result<
    (
        crate::transport::runtime_tools::RuntimeToolResult,
        Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    ),
    DaemonError,
> {
    let mut before_states: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut final_states: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut reservation_ranges: BTreeMap<PathBuf, Vec<crate::io::TextRange>> = BTreeMap::new();

    for state in &artifact_states {
        let path = PathBuf::from(&state.path);
        let content = remote_managed_io_content_from_state(state, domain)?;
        coordinator.read_artifact(crate::io::ArtifactReadRequest {
            workspace_identity: workspace_identity.clone(),
            path,
            domain,
            content,
        });
    }

    for operation in operations {
        match operation {
            ManagedPatchOperation::Add { path, content } => {
                managed_io_validate_patch_path(&workspace_context.root, &path)?;
                let current = remote_managed_patch_state(
                    &artifact_states,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_some() {
                    return Ok((
                        managed_patch_rejected(
                            path,
                            "add file target already exists; reread and retry with an update",
                        ),
                        Vec::new(),
                    ));
                }
                reservation_ranges
                    .entry(path.clone())
                    .or_default()
                    .push(crate::io::TextRange::new(0, usize::MAX));
                final_states.insert(path, Some(content));
            }
            ManagedPatchOperation::Update {
                path,
                old_text,
                new_text,
            } => {
                managed_io_validate_patch_path(&workspace_context.root, &path)?;
                let current = remote_managed_patch_state(
                    &artifact_states,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(current) = current else {
                    return Ok((
                        managed_patch_rejected(path, "update file target does not exist"),
                        Vec::new(),
                    ));
                };
                let Some((range, updated)) = replace_unique_text(&current, &old_text, &new_text)
                else {
                    return Ok((
                        managed_patch_rejected(
                            path,
                            "patch old text was not found exactly once in the current artifact",
                        ),
                        Vec::new(),
                    ));
                };
                reservation_ranges
                    .entry(path.clone())
                    .or_default()
                    .push(range);
                final_states.insert(path, Some(updated));
            }
            ManagedPatchOperation::Delete { path } => {
                managed_io_validate_patch_path(&workspace_context.root, &path)?;
                let current = remote_managed_patch_state(
                    &artifact_states,
                    &path,
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
            ManagedPatchOperation::Move {
                from_path,
                to_path,
                old_text,
                new_text,
            } => {
                managed_io_validate_patch_path(&workspace_context.root, &from_path)?;
                managed_io_validate_patch_path(&workspace_context.root, &to_path)?;
                if from_path == to_path {
                    return Ok((
                        managed_patch_rejected(from_path, "move source and target are identical"),
                        Vec::new(),
                    ));
                }
                let source = remote_managed_patch_state(
                    &artifact_states,
                    &from_path,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(mut source) = source else {
                    return Ok((
                        managed_patch_rejected(from_path, "move source does not exist"),
                        Vec::new(),
                    ));
                };
                let target = remote_managed_patch_state(
                    &artifact_states,
                    &to_path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if target.is_some() {
                    return Ok((
                        managed_patch_rejected(to_path, "move target already exists"),
                        Vec::new(),
                    ));
                }
                if let (Some(old_text), Some(new_text)) = (old_text, new_text) {
                    let Some((_range, updated)) =
                        replace_unique_text(&source, &old_text, &new_text)
                    else {
                        return Ok((
                            managed_patch_rejected(
                                from_path,
                                "move patch old text was not found exactly once in the current artifact",
                            ),
                            Vec::new(),
                        ));
                    };
                    source = updated;
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
        match managed_io_try_reserve_ranges(
            coordinator,
            &workspace_identity,
            &path,
            ranges,
            reservation_owner.clone(),
        ) {
            Ok(token) => reservations.push(token),
            Err(mut output) => {
                for token in reservations {
                    coordinator.release_reservation(token);
                }
                add_managed_io_workspace_payload(&mut output.payload, workspace_context);
                return Ok((output, Vec::new()));
            }
        }
    }

    for (path, after) in &final_states {
        match after {
            Some(text) => {
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: crate::io::ArtifactContent::Text(text.clone()),
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
        let before = before_states
            .get(path)
            .cloned()
            .flatten()
            .map(|text| ManagedIoTextSnapshot {
                existed: true,
                text,
            });
        let after_snapshot = after.clone().map(|text| ManagedIoTextSnapshot {
            existed: true,
            text,
        });
        let mut change_payload = serde_json::json!({});
        add_managed_io_change_payload(
            &mut change_payload,
            ManagedIoChangeContext {
                path: path.clone(),
                before,
                after: after_snapshot,
            },
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
    add_managed_io_workspace_payload(&mut payload, workspace_context);
    let final_artifact_states = final_states
        .into_iter()
        .map(|(path, content)| remote_managed_io_state(&path, content))
        .collect::<Vec<_>>();
    Ok((
        crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
        final_artifact_states,
    ))
}

pub(in crate::runtime::state) fn remote_managed_patch_state(
    artifact_states: &[crate::transport::relay_peer::RemoteManagedIoArtifactState],
    path: &PathBuf,
    before_states: &mut BTreeMap<PathBuf, Option<String>>,
    final_states: &mut BTreeMap<PathBuf, Option<String>>,
) -> Result<Option<String>, DaemonError> {
    if let Some(current) = final_states.get(path) {
        return Ok(current.clone());
    }
    let state = remote_managed_io_state_for_path(artifact_states, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "remote_managed_io_patch_state",
            message: format!(
                "missing forwarded artifact state for `{}`",
                path.to_string_lossy()
            ),
        }
    })?;
    let current = state.content_text.clone();
    before_states
        .entry(path.clone())
        .or_insert_with(|| current.clone());
    final_states.insert(path.clone(), current.clone());
    Ok(current)
}

pub(in crate::runtime::state) fn apply_remote_managed_whole_file_operations(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: crate::io::WorkspaceIdentity,
    domain: crate::io::ArtifactDomainKind,
    operations: Vec<ManagedWholeFileOperation>,
    artifact_states: Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    reservation_owner: crate::io::ArtifactReservationOwner,
    workspace_context: &ManagedIoWorkspaceContext,
) -> Result<
    (
        crate::transport::runtime_tools::RuntimeToolResult,
        Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    ),
    DaemonError,
> {
    let mut before_states: BTreeMap<PathBuf, Option<crate::io::ArtifactContent>> = BTreeMap::new();
    let mut final_states: BTreeMap<PathBuf, Option<crate::io::ArtifactContent>> = BTreeMap::new();
    let mut reservation_ranges: BTreeMap<PathBuf, Vec<crate::io::TextRange>> = BTreeMap::new();

    for state in &artifact_states {
        let path = PathBuf::from(&state.path);
        let content = remote_managed_io_content_from_state(state, domain)?;
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
                managed_io_validate_patch_path(&workspace_context.root, &path)?;
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
                managed_io_validate_patch_path(&workspace_context.root, &from_path)?;
                managed_io_validate_patch_path(&workspace_context.root, &to_path)?;
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
        match managed_io_try_reserve_ranges(
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
                add_managed_io_workspace_payload(&mut result.payload, workspace_context);
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
        add_managed_io_whole_file_change_payload(
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
    add_managed_io_workspace_payload(&mut payload, workspace_context);
    let final_artifact_states = final_states
        .into_iter()
        .map(|(path, content)| {
            remote_managed_io_state_from_content_with_domain(&path, content, domain)
        })
        .collect::<Vec<_>>();
    Ok((
        crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
        final_artifact_states,
    ))
}

pub(in crate::runtime::state) fn remote_managed_whole_file_state(
    artifact_states: &[crate::transport::relay_peer::RemoteManagedIoArtifactState],
    path: &PathBuf,
    domain: crate::io::ArtifactDomainKind,
    before_states: &mut BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
    final_states: &mut BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
) -> Result<Option<crate::io::ArtifactContent>, DaemonError> {
    if let Some(current) = final_states.get(path) {
        return Ok(current.clone());
    }
    let state = remote_managed_io_state_for_path(artifact_states, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "remote_managed_io_whole_file_state",
            message: format!(
                "missing forwarded artifact state for `{}`",
                path.to_string_lossy()
            ),
        }
    })?;
    let current = state
        .exists
        .then(|| remote_managed_io_content_from_state(state, domain))
        .transpose()?;
    before_states
        .entry(path.clone())
        .or_insert_with(|| current.clone());
    final_states.insert(path.clone(), current.clone());
    Ok(current)
}

pub(in crate::runtime::state) fn leased_workflow_tool_result_should_complete_turn(
    tool_name: &str,
    result: &crate::transport::runtime_tools::RuntimeToolResult,
) -> bool {
    if !result.ok {
        return false;
    }
    match tool_name {
        crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL => {
            result
                .payload
                .get("valid")
                .and_then(|value| value.as_bool())
                == Some(true)
        }
        crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL => {
            result
                .payload
                .get("valid")
                .and_then(|value| value.as_bool())
                == Some(true)
                && result
                    .payload
                    .get("submitted")
                    .and_then(|value| value.as_bool())
                    == Some(true)
        }
        _ => false,
    }
}

pub(in crate::runtime::state) fn forwarded_workflow_tool_result_should_complete_home_prompt(
    tool_name: &str,
    result: &crate::transport::runtime_tools::RuntimeToolResult,
) -> bool {
    if !result.ok {
        return false;
    }
    tool_name == crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
        && result
            .payload
            .get("valid")
            .and_then(|value| value.as_bool())
            == Some(true)
        && result
            .payload
            .get("submitted")
            .and_then(|value| value.as_bool())
            == Some(true)
}
