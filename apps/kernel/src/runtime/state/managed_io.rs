use super::*;

pub(super) fn managed_io_read_payload(read: crate::io::ArtifactReadResult) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "artifact_id": read.artifact_id.as_str(),
        "path": read.path.to_string_lossy(),
        "domain": managed_io_domain_name(read.domain),
        "version": read.version.value(),
        "snapshot_id": read.snapshot_id.as_str(),
    });
    match read.content {
        crate::io::ArtifactContent::Text(text) => {
            payload["content_text"] = serde_json::Value::String(text);
        }
        crate::io::ArtifactContent::Bytes(bytes) => {
            payload["byte_count"] = serde_json::json!(bytes.len());
            payload["content_base64"] =
                serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(bytes));
        }
    }
    payload
}

pub(super) fn add_managed_io_workspace_payload(
    payload: &mut serde_json::Value,
    workspace: &ManagedIoWorkspaceContext,
) {
    payload["workspace"] = serde_json::json!({
        "identity_changed": workspace.identity_changed,
        "identity_valid": workspace.valid,
        "identity_generation": workspace.generation,
        "vcs_provider": workspace.identity.vcs_provider.clone(),
        "repo_id": workspace.identity.repo_id.clone(),
        "repo_url": workspace.identity.repo_url.clone(),
        "branch": workspace.identity.branch.clone(),
        "head_commit": workspace.identity.head_commit.clone(),
        "worktree_root_fingerprint": workspace.identity.worktree_root_fingerprint.clone(),
    });
}

pub(super) fn managed_io_workspace_identity_rejected(
    workspace: &ManagedIoWorkspaceContext,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let mut payload = serde_json::json!({
        "applied": false,
        "reason": {
            "kind": "workspace_identity_changed",
            "message": "The provider run workspace identity changed since managed I/O coordination started."
        },
        "next_action": "Stop editing, reread the workspace state, and only retry after Arroba revalidates or rejoins the coordinated workspace.",
    });
    add_managed_io_workspace_payload(&mut payload, workspace);
    crate::transport::runtime_tools::RuntimeToolResult { ok: false, payload }
}

pub(super) fn managed_io_workspace_identities_match(
    home: &crate::io::WorkspaceIdentity,
    worker: &crate::io::WorkspaceIdentity,
) -> bool {
    if let (Some(left), Some(right)) = (home.repo_id.as_deref(), worker.repo_id.as_deref()) {
        return !left.is_empty() && left == right && home.branch == worker.branch;
    }
    if let (Some(left), Some(right)) = (home.repo_url.as_deref(), worker.repo_url.as_deref()) {
        return normalize_managed_io_repo_url(left) == normalize_managed_io_repo_url(right)
            && home.branch == worker.branch;
    }
    home.worktree_root_fingerprint == worker.worktree_root_fingerprint
}

pub(super) fn normalize_managed_io_repo_url(value: &str) -> String {
    value.trim().trim_end_matches(".git").to_ascii_lowercase()
}

pub(super) fn managed_io_edit_operation_from_args(
    args: crate::transport::runtime_tools::ManagedEditArtifactArgs,
) -> Result<crate::io::AgentEditOperation, DaemonError> {
    match (args.range, args.old_text) {
        (Some(range), Some(old_text)) => Ok(crate::io::AgentEditOperation::ReplaceRange {
            range: crate::io::TextRange::new(range.start, range.end),
            old_text,
            new_text: args.new_text,
        }),
        (None, Some(old_text)) => Ok(crate::io::AgentEditOperation::ReplaceText {
            old_text,
            new_text: args.new_text,
        }),
        (Some(_), None) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_edit_artifact",
            message: "range edits require old_text".to_string(),
        }),
        (None, None) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_edit_artifact",
            message: "managed text edits require old_text or range+old_text".to_string(),
        }),
    }
}

pub(super) fn managed_io_write_content_from_args(
    operation: &'static str,
    domain: crate::io::ArtifactDomainKind,
    args: &crate::transport::runtime_tools::ManagedWriteArtifactArgs,
) -> Result<crate::io::ArtifactContent, DaemonError> {
    match domain {
        crate::io::ArtifactDomainKind::TextDocument
        | crate::io::ArtifactDomainKind::StructuredDocument => {
            let Some(text) = args.content_text.clone() else {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: "content_text is required for text and structured artifacts"
                        .to_string(),
                });
            };
            Ok(crate::io::ArtifactContent::Text(text))
        }
        crate::io::ArtifactDomainKind::OpaqueBlob => {
            let Some(content_base64) = args.content_base64.as_deref() else {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: "content_base64 is required for opaque artifacts".to_string(),
                });
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(content_base64)
                .map_err(|error| DaemonError::LocalTransport {
                    operation,
                    message: format!("content_base64 is not valid base64: {error}"),
                })?;
            Ok(crate::io::ArtifactContent::Bytes(bytes))
        }
    }
}

pub(super) fn managed_io_snapshot_id_from_arg(
    snapshot_id: Option<String>,
) -> Option<crate::io::ArtifactSnapshotId> {
    snapshot_id
        .filter(|snapshot_id| {
            let snapshot_id = snapshot_id.trim();
            !snapshot_id.is_empty() && snapshot_id != "__arroba_create__" && snapshot_id != "*"
        })
        .map(crate::io::ArtifactSnapshotId::new)
}

pub(super) fn managed_io_write_snapshot_id_from_arg(
    snapshot_id: Option<String>,
    path: &Path,
) -> Option<crate::io::ArtifactSnapshotId> {
    let snapshot_id = managed_io_snapshot_id_from_arg(snapshot_id)?;
    let snapshot_value = snapshot_id.as_str();
    let path_value = path.to_string_lossy();
    if snapshot_value.starts_with("snap:") && !snapshot_value.contains(path_value.as_ref()) {
        return None;
    }
    Some(snapshot_id)
}

pub(super) fn remote_managed_io_artifact_states_for_tool(
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

pub(super) fn remote_managed_io_states_for_patch_operations(
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

pub(super) fn remote_managed_io_state(
    path: &PathBuf,
    content_text: Option<String>,
) -> crate::transport::relay_peer::RemoteManagedIoArtifactState {
    crate::transport::relay_peer::RemoteManagedIoArtifactState {
        path: path.to_string_lossy().to_string(),
        exists: content_text.is_some(),
        domain: Some("text".to_string()),
        content_text,
        content_base64: None,
    }
}

pub(super) fn remote_managed_io_state_from_content(
    path: &PathBuf,
    content: Option<crate::io::ArtifactContent>,
) -> crate::transport::relay_peer::RemoteManagedIoArtifactState {
    match content {
        Some(crate::io::ArtifactContent::Text(text)) => remote_managed_io_state(path, Some(text)),
        Some(crate::io::ArtifactContent::Bytes(bytes)) => {
            crate::transport::relay_peer::RemoteManagedIoArtifactState {
                path: path.to_string_lossy().to_string(),
                exists: true,
                domain: Some("opaque".to_string()),
                content_text: None,
                content_base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            }
        }
        None => crate::transport::relay_peer::RemoteManagedIoArtifactState {
            path: path.to_string_lossy().to_string(),
            exists: false,
            domain: None,
            content_text: None,
            content_base64: None,
        },
    }
}

pub(super) fn remote_managed_io_state_from_content_with_domain(
    path: &PathBuf,
    content: Option<crate::io::ArtifactContent>,
    domain: crate::io::ArtifactDomainKind,
) -> crate::transport::relay_peer::RemoteManagedIoArtifactState {
    let mut state = remote_managed_io_state_from_content(path, content);
    state.domain = Some(
        match domain {
            crate::io::ArtifactDomainKind::TextDocument => "text",
            crate::io::ArtifactDomainKind::StructuredDocument => "structured",
            crate::io::ArtifactDomainKind::OpaqueBlob => "opaque",
        }
        .to_string(),
    );
    state
}

pub(super) fn remote_managed_io_state_for_path<'a>(
    states: &'a [crate::transport::relay_peer::RemoteManagedIoArtifactState],
    path: &PathBuf,
) -> Option<&'a crate::transport::relay_peer::RemoteManagedIoArtifactState> {
    let expected = path.to_string_lossy();
    states.iter().find(|state| state.path == expected)
}

pub(super) fn remote_managed_io_content_from_state(
    state: &crate::transport::relay_peer::RemoteManagedIoArtifactState,
    domain: crate::io::ArtifactDomainKind,
) -> Result<crate::io::ArtifactContent, DaemonError> {
    match domain {
        crate::io::ArtifactDomainKind::TextDocument
        | crate::io::ArtifactDomainKind::StructuredDocument => Ok(
            crate::io::ArtifactContent::Text(state.content_text.clone().unwrap_or_default()),
        ),
        crate::io::ArtifactDomainKind::OpaqueBlob => {
            let bytes = match state.content_base64.as_deref() {
                Some(content_base64) => base64::engine::general_purpose::STANDARD
                    .decode(content_base64)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "remote_managed_io_content",
                        message: format!(
                            "forwarded opaque artifact state is not valid base64: {error}"
                        ),
                    })?,
                None => Vec::new(),
            };
            Ok(crate::io::ArtifactContent::Bytes(bytes))
        }
    }
}

pub(super) fn remote_managed_io_state_domain(
    state: &crate::transport::relay_peer::RemoteManagedIoArtifactState,
) -> crate::io::ArtifactDomainKind {
    if let Some(domain) = state.domain.as_deref() {
        if let Ok(domain) = KernelRuntimeOwnedState::managed_io_domain_from_arg(Some(domain)) {
            return domain;
        }
    }
    if state.content_base64.is_some() {
        crate::io::ArtifactDomainKind::OpaqueBlob
    } else {
        crate::io::ArtifactDomainKind::TextDocument
    }
}

pub(super) fn remote_managed_io_states_content_equal(
    left: &crate::transport::relay_peer::RemoteManagedIoArtifactState,
    right: &crate::transport::relay_peer::RemoteManagedIoArtifactState,
) -> bool {
    left.exists == right.exists
        && left.content_text == right.content_text
        && left.content_base64 == right.content_base64
}

pub(super) fn remote_managed_io_text_snapshot_from_state(
    state: &crate::transport::relay_peer::RemoteManagedIoArtifactState,
) -> Option<ManagedIoTextSnapshot> {
    Some(ManagedIoTextSnapshot {
        existed: state.exists,
        text: state.content_text.clone().unwrap_or_default(),
    })
}

pub(super) fn apply_remote_managed_io_final_states(
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

pub(super) fn apply_remote_managed_patch_operations(
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
                        return Ok((managed_patch_rejected(
                            from_path,
                            "move patch old text was not found exactly once in the current artifact",
                        ), Vec::new()));
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

pub(super) fn remote_managed_patch_state(
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

pub(super) fn apply_remote_managed_whole_file_operations(
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

pub(super) fn remote_managed_whole_file_state(
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

pub(super) fn leased_workflow_tool_result_should_complete_turn(
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

pub(super) fn forwarded_workflow_tool_result_should_complete_home_prompt(
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

pub(super) struct ManagedIoChangeContext {
    pub(super) path: PathBuf,
    pub(super) before: Option<ManagedIoTextSnapshot>,
    pub(super) after: Option<ManagedIoTextSnapshot>,
}

pub(super) struct ManagedIoTextSnapshot {
    pub(super) existed: bool,
    pub(super) text: String,
}

#[derive(Debug, Clone)]
pub(super) enum ManagedPatchOperation {
    Add {
        path: PathBuf,
        content: String,
    },
    Update {
        path: PathBuf,
        old_text: String,
        new_text: String,
    },
    Delete {
        path: PathBuf,
    },
    Move {
        from_path: PathBuf,
        to_path: PathBuf,
        old_text: Option<String>,
        new_text: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub(super) enum ManagedWholeFileOperation {
    Delete {
        path: PathBuf,
    },
    Move {
        from_path: PathBuf,
        to_path: PathBuf,
    },
}

pub(super) fn managed_io_edit_result(
    result: crate::io::EditResult,
    change: ManagedIoChangeContext,
    external_change_notice: Option<crate::io::ArtifactExternalChangeNotice>,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    match result {
        crate::io::EditResult::Applied { new_version } => {
            let mut payload = serde_json::json!({
                "applied": true,
                "new_version": new_version.value(),
            });
            add_managed_io_change_payload(&mut payload, change);
            add_managed_io_external_change_notice_payload(&mut payload, external_change_notice);
            crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload }
        }
        crate::io::EditResult::AppliedWithWarning {
            new_version,
            warning,
        } => {
            let mut payload = serde_json::json!({
                "applied": true,
                "new_version": new_version.value(),
                "warning": managed_io_warning_payload(warning),
            });
            add_managed_io_change_payload(&mut payload, change);
            add_managed_io_external_change_notice_payload(&mut payload, external_change_notice);
            crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload }
        }
        crate::io::EditResult::Rejected { reason } => {
            let mut payload = serde_json::json!({
                "applied": false,
                "reason": managed_io_error_payload(reason),
                "next_action": "Reread the artifact with arroba.read_artifact, reconcile with the current content, and retry through arroba.edit_artifact.",
            });
            add_managed_io_external_change_notice_payload(&mut payload, external_change_notice);
            crate::transport::runtime_tools::RuntimeToolResult { ok: false, payload }
        }
    }
}

pub(super) fn add_managed_io_external_change_notice_payload(
    payload: &mut serde_json::Value,
    notice: Option<crate::io::ArtifactExternalChangeNotice>,
) {
    add_managed_io_external_change_notices_payload(payload, notice.into_iter().collect());
}

pub(super) fn add_managed_io_external_change_notices_payload(
    payload: &mut serde_json::Value,
    notices: Vec<crate::io::ArtifactExternalChangeNotice>,
) {
    if notices.is_empty() {
        return;
    }
    let notices = notices
        .into_iter()
        .map(managed_io_external_change_notice_payload)
        .collect::<Vec<_>>();
    payload["external_changes"] = serde_json::json!(notices);
    if let Some(notice) = payload["external_changes"].get(0).cloned() {
        payload["external_change"] = notice;
    }
}

pub(super) fn managed_io_external_change_notice_payload(
    notice: crate::io::ArtifactExternalChangeNotice,
) -> serde_json::Value {
    serde_json::json!({
        "detected": true,
        "path": notice.path.to_string_lossy(),
        "message": notice.message,
        "next_action": "This artifact changed outside Arroba managed I/O after your last read. If the write was rejected, reread and reconcile before retrying; if it was applied with a rebase warning, verify the diff before continuing.",
    })
}

pub(super) fn managed_io_external_change_notice_for_path(
    path: PathBuf,
) -> crate::io::ArtifactExternalChangeNotice {
    crate::io::ArtifactExternalChangeNotice {
        path,
        message: "artifact changed outside Arroba managed I/O while the managed operation was being prepared".to_string(),
    }
}

pub(super) fn managed_io_result_applied(result: &crate::io::EditResult) -> bool {
    matches!(
        result,
        crate::io::EditResult::Applied { .. } | crate::io::EditResult::AppliedWithWarning { .. }
    )
}

pub(super) fn record_managed_io_external_change_if_rejected(
    monitor: &crate::io::ArtifactExternalChangeMonitor,
    workspace_identity: &crate::io::WorkspaceIdentity,
    path: &PathBuf,
    result: &crate::io::EditResult,
) {
    if matches!(
        result,
        crate::io::EditResult::Rejected {
            reason: crate::io::ArtifactEditError::ExternalChangeDuringApply { .. }
        }
    ) {
        monitor.record_external_change(workspace_identity, path);
    }
}

pub(super) fn record_managed_io_write_if_applied(
    monitor: &crate::io::ArtifactExternalChangeMonitor,
    provider_run_id: &str,
    workspace_identity: &crate::io::WorkspaceIdentity,
    workspace_root: &PathBuf,
    path: &PathBuf,
    result: &crate::io::EditResult,
) {
    if managed_io_result_applied(result) {
        monitor.observe_managed_write(provider_run_id, workspace_identity, workspace_root, path);
    }
}

pub(super) fn managed_io_reservation_ranges_for_operation(
    operation: &crate::io::AgentEditOperation,
    before: Option<&ManagedIoTextSnapshot>,
    fallback: crate::io::TextRange,
) -> Vec<crate::io::TextRange> {
    match operation {
        crate::io::AgentEditOperation::ReplaceRange { range, .. } => vec![*range],
        crate::io::AgentEditOperation::ReplaceText { old_text, .. } => before
            .and_then(|before| before.text.find(old_text))
            .map(|start| vec![crate::io::TextRange::new(start, start + old_text.len())])
            .unwrap_or_else(|| vec![fallback]),
        crate::io::AgentEditOperation::WriteArtifact { .. } => vec![fallback],
    }
}

pub(super) fn managed_io_try_reserve_ranges(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: &crate::io::WorkspaceIdentity,
    path: &PathBuf,
    ranges: Vec<crate::io::TextRange>,
    owner: crate::io::ArtifactReservationOwner,
) -> Result<crate::io::ArtifactReservationToken, crate::transport::runtime_tools::RuntimeToolResult>
{
    coordinator
        .try_reserve_ranges(workspace_identity, path, ranges, owner)
        .map_err(|reason| crate::transport::runtime_tools::RuntimeToolResult {
            ok: false,
            payload: serde_json::json!({
                "applied": false,
                "reason": managed_io_error_payload(reason),
                "next_action": "Another managed writer has reserved the same artifact area. Wait for that write to finish, reread the artifact with arroba.read_artifact, and retry through Arroba managed I/O.",
            }),
        })
}

pub(super) fn managed_io_reservation_owner(
    provider_run: &crate::provider::RuntimeProviderRun,
    tool_name: &str,
) -> crate::io::ArtifactReservationOwner {
    crate::io::ArtifactReservationOwner::new(
        provider_run.id().to_string(),
        provider_run.agent_instance_id().map(str::to_string),
        tool_name.to_string(),
    )
}

pub(super) fn parse_managed_apply_patch(
    patch_text: &str,
) -> Result<Vec<ManagedPatchOperation>, DaemonError> {
    let lines = patch_text.lines().collect::<Vec<_>>();
    if lines.first().map(|line| line.trim()) != Some("*** Begin Patch")
        || lines.last().map(|line| line.trim()) != Some("*** End Patch")
    {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "patch_text must use the apply_patch envelope".to_string(),
        });
    }
    let mut operations = Vec::new();
    let mut index = 1usize;
    while index + 1 < lines.len() {
        let line = lines[index];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            index += 1;
            let mut body = Vec::new();
            while index < lines.len() && !lines[index].starts_with("*** ") {
                let line = lines[index];
                body.push(line.strip_prefix('+').unwrap_or(line).to_string());
                index += 1;
            }
            operations.push(ManagedPatchOperation::Add {
                path: PathBuf::from(path.trim()),
                content: join_patch_lines(&body),
            });
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            operations.push(ManagedPatchOperation::Delete {
                path: PathBuf::from(path.trim()),
            });
            index += 1;
            continue;
        }
        if line.starts_with("*** Move to: ") {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_apply_patch",
                message: "move hunks must follow an update file header".to_string(),
            });
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            index += 1;
            let mut move_to = None;
            if index < lines.len() {
                if let Some(target) = lines[index].strip_prefix("*** Move to: ") {
                    move_to = Some(PathBuf::from(target.trim()));
                    index += 1;
                }
            }
            let mut old_lines = Vec::new();
            let mut new_lines = Vec::new();
            while index < lines.len() && !lines[index].starts_with("*** ") {
                let line = lines[index];
                if line.starts_with("@@") || line.starts_with('\\') {
                    index += 1;
                    continue;
                }
                if let Some(rest) = line.strip_prefix('-') {
                    old_lines.push(rest.to_string());
                } else if let Some(rest) = line.strip_prefix('+') {
                    new_lines.push(rest.to_string());
                } else {
                    let rest = line.strip_prefix(' ').unwrap_or(line);
                    old_lines.push(rest.to_string());
                    new_lines.push(rest.to_string());
                }
                index += 1;
            }
            match move_to {
                Some(to_path) => operations.push(ManagedPatchOperation::Move {
                    from_path: PathBuf::from(path.trim()),
                    to_path,
                    old_text: (!old_lines.is_empty()).then(|| join_patch_lines(&old_lines)),
                    new_text: (!new_lines.is_empty()).then(|| join_patch_lines(&new_lines)),
                }),
                None => {
                    if old_lines.is_empty() {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_apply_patch",
                            message: format!("update hunk for `{}` has no old text", path.trim()),
                        });
                    }
                    operations.push(ManagedPatchOperation::Update {
                        path: PathBuf::from(path.trim()),
                        old_text: join_patch_lines(&old_lines),
                        new_text: join_patch_lines(&new_lines),
                    });
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!("unsupported patch line `{line}`"),
        });
    }
    if operations.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "patch_text did not contain any supported file operations".to_string(),
        });
    }
    Ok(operations)
}

pub(super) fn join_patch_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

pub(super) fn apply_managed_patch_operations(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: crate::io::WorkspaceIdentity,
    workspace_root: PathBuf,
    domain: crate::io::ArtifactDomainKind,
    operations: Vec<ManagedPatchOperation>,
    reservation_owner: crate::io::ArtifactReservationOwner,
    external_change_monitor: &crate::io::ArtifactExternalChangeMonitor,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let mut before_states: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut final_states: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut reservation_ranges: BTreeMap<PathBuf, Vec<crate::io::TextRange>> = BTreeMap::new();

    for operation in operations {
        match operation {
            ManagedPatchOperation::Add { path, content } => {
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_patch_state(
                    &workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_some() {
                    return Ok(managed_patch_rejected(
                        path,
                        "add file target already exists; reread and retry with an update",
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
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_patch_state(
                    &workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(current) = current else {
                    return Ok(managed_patch_rejected(
                        path,
                        "update file target does not exist",
                    ));
                };
                let Some((range, updated)) = replace_unique_text(&current, &old_text, &new_text)
                else {
                    return Ok(managed_patch_rejected(
                        path,
                        "patch old text was not found exactly once in the current artifact",
                    ));
                };
                reservation_ranges
                    .entry(path.clone())
                    .or_default()
                    .push(range);
                final_states.insert(path, Some(updated));
            }
            ManagedPatchOperation::Delete { path } => {
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_patch_state(
                    &workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_none() {
                    return Ok(managed_patch_rejected(
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
            ManagedPatchOperation::Move {
                from_path,
                to_path,
                old_text,
                new_text,
            } => {
                managed_io_validate_patch_path(&workspace_root, &from_path)?;
                managed_io_validate_patch_path(&workspace_root, &to_path)?;
                if from_path == to_path {
                    return Ok(managed_patch_rejected(
                        from_path,
                        "move source and target are identical",
                    ));
                }
                let source = managed_patch_state(
                    &workspace_root,
                    &from_path,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(mut source) = source else {
                    return Ok(managed_patch_rejected(
                        from_path,
                        "move source does not exist",
                    ));
                };
                let target = managed_patch_state(
                    &workspace_root,
                    &to_path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if target.is_some() {
                    return Ok(managed_patch_rejected(
                        to_path,
                        "move target already exists",
                    ));
                }
                if let (Some(old_text), Some(new_text)) = (old_text, new_text) {
                    let Some((_range, updated)) =
                        replace_unique_text(&source, &old_text, &new_text)
                    else {
                        return Ok(managed_patch_rejected(
                            from_path,
                            "move patch old text was not found exactly once in the current artifact",
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
        let latest = match managed_io_read_optional_text(&workspace_root, path) {
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
                notices.push(managed_io_external_change_notice_for_path(path.clone()));
            }
            let mut output = managed_patch_rejected(
                path.clone(),
                "artifact changed while the managed patch was being prepared; reread and retry",
            );
            add_managed_io_external_change_notices_payload(&mut output.payload, notices);
            return Ok(output);
        }
    }

    if let Err(error) = managed_io_write_final_states(&workspace_root, &final_states) {
        let _ = managed_io_write_final_states(&workspace_root, &before_states);
        for token in reservations {
            coordinator.release_reservation(token);
        }
        return Err(error);
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
        let before =
            before_states
                .get(&path)
                .cloned()
                .flatten()
                .map(|text| ManagedIoTextSnapshot {
                    existed: true,
                    text,
                });
        let after = after.map(|text| ManagedIoTextSnapshot {
            existed: true,
            text,
        });
        let mut change_payload = serde_json::json!({});
        add_managed_io_change_payload(
            &mut change_payload,
            ManagedIoChangeContext {
                path,
                before,
                after,
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
    add_managed_io_external_change_notices_payload(&mut payload, external_change_notices);
    if changes.len() == 1 {
        payload["change"] = changes[0].clone();
        if let Some(path) = changes[0].get("path").cloned() {
            payload["path"] = path;
        }
    }
    Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
}

pub(super) fn apply_managed_whole_file_operations(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: crate::io::WorkspaceIdentity,
    workspace_root: PathBuf,
    domain: crate::io::ArtifactDomainKind,
    operations: Vec<ManagedWholeFileOperation>,
    reservation_owner: crate::io::ArtifactReservationOwner,
    external_change_monitor: &crate::io::ArtifactExternalChangeMonitor,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let mut before_states: BTreeMap<PathBuf, Option<crate::io::ArtifactContent>> = BTreeMap::new();
    let mut final_states: BTreeMap<PathBuf, Option<crate::io::ArtifactContent>> = BTreeMap::new();
    let mut reservation_ranges: BTreeMap<PathBuf, Vec<crate::io::TextRange>> = BTreeMap::new();

    for operation in operations {
        match operation {
            ManagedWholeFileOperation::Delete { path } => {
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_whole_file_state(
                    &workspace_root,
                    &path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_none() {
                    return Ok(managed_patch_rejected(
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
            ManagedWholeFileOperation::Move { from_path, to_path } => {
                managed_io_validate_patch_path(&workspace_root, &from_path)?;
                managed_io_validate_patch_path(&workspace_root, &to_path)?;
                if from_path == to_path {
                    return Ok(managed_patch_rejected(
                        from_path,
                        "move source and target are identical",
                    ));
                }
                let source = managed_whole_file_state(
                    &workspace_root,
                    &from_path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(source) = source else {
                    return Ok(managed_patch_rejected(
                        from_path,
                        "move source does not exist",
                    ));
                };
                let target = managed_whole_file_state(
                    &workspace_root,
                    &to_path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                if target.is_some() {
                    return Ok(managed_patch_rejected(
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
        match managed_io_try_reserve_ranges(
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
        let latest = match managed_io_read_optional_content(&workspace_root, path, domain) {
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
                notices.push(managed_io_external_change_notice_for_path(path.clone()));
            }
            let mut output = managed_patch_rejected(
                path.clone(),
                "artifact changed while the managed whole-file operation was being prepared; reread and retry",
            );
            add_managed_io_external_change_notices_payload(&mut output.payload, notices);
            return Ok(output);
        }
    }

    if let Err(error) = managed_io_write_final_content_states(&workspace_root, &final_states) {
        let _ = managed_io_write_final_content_states(&workspace_root, &before_states);
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
        add_managed_io_whole_file_change_payload(&mut change_payload, path, before, after);
        if let Some(change) = change_payload.get("change") {
            changes.push(change.clone());
        }
    }

    let mut payload = serde_json::json!({
        "applied": true,
        "atomic": true,
        "changes": changes,
    });
    add_managed_io_external_change_notices_payload(&mut payload, external_change_notices);
    if changes.len() == 1 {
        payload["change"] = changes[0].clone();
        if let Some(path) = changes[0].get("path").cloned() {
            payload["path"] = path;
        }
    }
    Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
}

pub(super) fn managed_patch_state(
    workspace_root: &PathBuf,
    path: &PathBuf,
    before_states: &mut BTreeMap<PathBuf, Option<String>>,
    final_states: &mut BTreeMap<PathBuf, Option<String>>,
) -> Result<Option<String>, DaemonError> {
    if let Some(current) = final_states.get(path) {
        return Ok(current.clone());
    }
    let current = managed_io_read_optional_text(workspace_root, path)?;
    before_states
        .entry(path.clone())
        .or_insert_with(|| current.clone());
    final_states.insert(path.clone(), current.clone());
    Ok(current)
}

pub(super) fn managed_whole_file_state(
    workspace_root: &PathBuf,
    path: &PathBuf,
    domain: crate::io::ArtifactDomainKind,
    before_states: &mut BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
    final_states: &mut BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
) -> Result<Option<crate::io::ArtifactContent>, DaemonError> {
    if let Some(current) = final_states.get(path) {
        return Ok(current.clone());
    }
    let current = managed_io_read_optional_content(workspace_root, path, domain)?;
    before_states
        .entry(path.clone())
        .or_insert_with(|| current.clone());
    final_states.insert(path.clone(), current.clone());
    Ok(current)
}

pub(super) fn replace_unique_text(
    current: &str,
    old_text: &str,
    new_text: &str,
) -> Option<(crate::io::TextRange, String)> {
    let start = current.find(old_text)?;
    if current[start + old_text.len()..].contains(old_text) {
        return None;
    }
    let range = crate::io::TextRange::new(start, start + old_text.len());
    let mut updated = String::with_capacity(current.len() - old_text.len() + new_text.len());
    updated.push_str(&current[..start]);
    updated.push_str(new_text);
    updated.push_str(&current[start + old_text.len()..]);
    Some((range, updated))
}

pub(super) fn managed_patch_rejected(
    path: PathBuf,
    message: impl Into<String>,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({
            "applied": false,
            "reason": {
                "kind": "invalid_operation",
                "path": path.to_string_lossy(),
                "message": message.into(),
            },
            "next_action": "Reread the affected artifact with arroba.read_artifact, reconcile with the current content, and retry through Arroba managed I/O.",
        }),
    }
}

pub(super) fn managed_io_validate_patch_path(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Result<(), DaemonError> {
    let _ = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| DaemonError::LocalTransport {
        operation: "runtime_tool_apply_patch",
        message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
    })?;
    if path == std::path::Path::new(crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH)
        && managed_io_is_arroba_source_workspace(workspace_root)
    {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!(
                "the Arroba managed-I/O instruction policy `{}` is owned by Arroba and cannot be edited through managed artifact I/O",
                crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH
            ),
        });
    }
    Ok(())
}

pub(super) fn managed_io_read_optional_text(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Result<Option<String>, DaemonError> {
    let full_path = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
        }
    })?;
    match std::fs::read_to_string(&full_path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!("failed to read `{}`: {error}", path.to_string_lossy()),
        }),
    }
}

pub(super) fn managed_io_read_optional_content(
    workspace_root: &PathBuf,
    path: &PathBuf,
    domain: crate::io::ArtifactDomainKind,
) -> Result<Option<crate::io::ArtifactContent>, DaemonError> {
    let full_path = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "runtime_tool_managed_io_state",
            message:
                "managed paths must be workspace-relative and cannot escape the workspace root"
                    .to_string(),
        }
    })?;
    match std::fs::read(&full_path) {
        Ok(bytes) => match domain {
            crate::io::ArtifactDomainKind::TextDocument
            | crate::io::ArtifactDomainKind::StructuredDocument => {
                let text =
                    String::from_utf8(bytes).map_err(|error| DaemonError::LocalTransport {
                        operation: "runtime_tool_managed_io_state",
                        message: format!(
                            "failed to decode `{}` as UTF-8: {error}",
                            path.to_string_lossy()
                        ),
                    })?;
                Ok(Some(crate::io::ArtifactContent::Text(text)))
            }
            crate::io::ArtifactDomainKind::OpaqueBlob => {
                Ok(Some(crate::io::ArtifactContent::Bytes(bytes)))
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_managed_io_state",
            message: format!("failed to read `{}`: {error}", path.to_string_lossy()),
        }),
    }
}

pub(super) fn managed_io_write_final_states(
    workspace_root: &PathBuf,
    states: &BTreeMap<PathBuf, Option<String>>,
) -> Result<(), DaemonError> {
    for (path, text) in states {
        let full_path = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_apply_patch",
                message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
            }
        })?;
        match text {
            Some(text) => {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_apply_patch",
                            message: format!(
                                "failed to create `{}`: {error}",
                                parent.to_string_lossy()
                            ),
                        }
                    })?;
                }
                std::fs::write(&full_path, text).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_apply_patch",
                    message: format!("failed to write `{}`: {error}", path.to_string_lossy()),
                })?;
            }
            None => match std::fs::remove_file(&full_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_apply_patch",
                        message: format!("failed to delete `{}`: {error}", path.to_string_lossy()),
                    });
                }
            },
        }
    }
    Ok(())
}

pub(super) fn managed_io_write_final_content_states(
    workspace_root: &PathBuf,
    states: &BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
) -> Result<(), DaemonError> {
    for (path, content) in states {
        let full_path = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_managed_io_state",
                message:
                    "managed paths must be workspace-relative and cannot escape the workspace root"
                        .to_string(),
            }
        })?;
        match content {
            Some(crate::io::ArtifactContent::Text(text)) => {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_managed_io_state",
                            message: format!(
                                "failed to create `{}`: {error}",
                                parent.to_string_lossy()
                            ),
                        }
                    })?;
                }
                std::fs::write(&full_path, text).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_managed_io_state",
                    message: format!("failed to write `{}`: {error}", path.to_string_lossy()),
                })?;
            }
            Some(crate::io::ArtifactContent::Bytes(bytes)) => {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_managed_io_state",
                            message: format!(
                                "failed to create `{}`: {error}",
                                parent.to_string_lossy()
                            ),
                        }
                    })?;
                }
                std::fs::write(&full_path, bytes).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_managed_io_state",
                    message: format!("failed to write `{}`: {error}", path.to_string_lossy()),
                })?;
            }
            None => match std::fs::remove_file(&full_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_managed_io_state",
                        message: format!("failed to delete `{}`: {error}", path.to_string_lossy()),
                    });
                }
            },
        }
    }
    Ok(())
}

pub(super) fn managed_io_is_arroba_source_workspace(root: &PathBuf) -> bool {
    root.join("apps/kernel/Cargo.toml").is_file()
        && root
            .join(crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH)
            .is_file()
}

pub(super) fn add_managed_io_change_payload(
    payload: &mut serde_json::Value,
    change: ManagedIoChangeContext,
) {
    if change.before.is_none() && change.after.is_none() {
        return;
    }
    let before = change.before.unwrap_or(ManagedIoTextSnapshot {
        existed: false,
        text: String::new(),
    });
    let after = change.after.unwrap_or(ManagedIoTextSnapshot {
        existed: false,
        text: String::new(),
    });
    let diff = managed_io_unified_diff(&change.path, &before, &after);
    payload["path"] = serde_json::Value::String(change.path.to_string_lossy().to_string());
    payload["change"] = serde_json::json!({
        "path": change.path.to_string_lossy(),
        "kind": if !before.existed {
            "add"
        } else if !after.existed {
            "delete"
        } else {
            "update"
        },
        "diff": diff.text,
        "diff_truncated": diff.truncated,
    });
}

pub(super) fn add_managed_io_whole_file_change_payload(
    payload: &mut serde_json::Value,
    path: PathBuf,
    before: Option<crate::io::ArtifactContent>,
    after: Option<crate::io::ArtifactContent>,
) {
    if before.is_none() && after.is_none() {
        return;
    }
    let before_existed = before.is_some();
    let after_existed = after.is_some();
    if let (
        Some(crate::io::ArtifactContent::Text(before)),
        Some(crate::io::ArtifactContent::Text(after)),
    ) = (&before, &after)
    {
        add_managed_io_change_payload(
            payload,
            ManagedIoChangeContext {
                path,
                before: Some(ManagedIoTextSnapshot {
                    existed: true,
                    text: before.clone(),
                }),
                after: Some(ManagedIoTextSnapshot {
                    existed: true,
                    text: after.clone(),
                }),
            },
        );
        return;
    }
    let normalized_path = path.to_string_lossy().to_string();
    let before_bytes = before
        .as_ref()
        .map(artifact_content_byte_count)
        .unwrap_or(0);
    let after_bytes = after.as_ref().map(artifact_content_byte_count).unwrap_or(0);
    payload["path"] = serde_json::Value::String(normalized_path.clone());
    payload["change"] = serde_json::json!({
        "path": normalized_path,
        "kind": if !before_existed {
            "add"
        } else if !after_existed {
            "delete"
        } else {
            "update"
        },
        "binary": true,
        "before_byte_count": before_bytes,
        "after_byte_count": after_bytes,
        "diff": "Binary files differ",
        "diff_truncated": false,
    });
}

pub(super) fn artifact_content_byte_count(content: &crate::io::ArtifactContent) -> usize {
    match content {
        crate::io::ArtifactContent::Text(text) => text.len(),
        crate::io::ArtifactContent::Bytes(bytes) => bytes.len(),
    }
}

pub(super) struct ManagedIoDiff {
    pub(super) text: String,
    pub(super) truncated: bool,
}

pub(super) const MANAGED_IO_MAX_DIFF_BYTES: usize = 80_000;

pub(super) fn managed_io_unified_diff(
    path: &PathBuf,
    before: &ManagedIoTextSnapshot,
    after: &ManagedIoTextSnapshot,
) -> ManagedIoDiff {
    let normalized_path = path.to_string_lossy();
    let mut lines = Vec::new();
    lines.push(format!(
        "diff --git a/{normalized_path} b/{normalized_path}"
    ));
    if !before.existed {
        lines.push("new file mode 100644".to_string());
        lines.push("--- /dev/null".to_string());
    } else {
        if !after.existed {
            lines.push("deleted file mode 100644".to_string());
        }
        lines.push(format!("--- a/{normalized_path}"));
    }
    if after.existed {
        lines.push(format!("+++ b/{normalized_path}"));
    } else {
        lines.push("+++ /dev/null".to_string());
    }
    let before_lines = diff_lines(&before.text);
    let after_lines = diff_lines(&after.text);
    lines.extend(managed_io_diff_hunks(&before_lines, &after_lines));
    let mut text = lines.join("\n");
    let mut truncated = false;
    if text.len() > MANAGED_IO_MAX_DIFF_BYTES {
        text.truncate(MANAGED_IO_MAX_DIFF_BYTES);
        text.push_str("\n... diff truncated ...");
        truncated = true;
    }
    ManagedIoDiff { text, truncated }
}

pub(super) fn diff_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n')
        .map(|line| line.strip_suffix('\n').unwrap_or(line))
        .collect()
}

#[derive(Clone, Copy)]
pub(super) enum ManagedIoDiffOp<'a> {
    Context(&'a str),
    Remove(&'a str),
    Add(&'a str),
}

pub(super) fn managed_io_diff_ops<'a>(
    before: &'a [&'a str],
    after: &'a [&'a str],
) -> Vec<ManagedIoDiffOp<'a>> {
    let lcs = managed_io_lcs_table(before, after);
    let mut ops = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < before.len() && j < after.len() {
        if before[i] == after[j] {
            ops.push(ManagedIoDiffOp::Context(before[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            ops.push(ManagedIoDiffOp::Remove(before[i]));
            i += 1;
        } else {
            ops.push(ManagedIoDiffOp::Add(after[j]));
            j += 1;
        }
    }
    while i < before.len() {
        ops.push(ManagedIoDiffOp::Remove(before[i]));
        i += 1;
    }
    while j < after.len() {
        ops.push(ManagedIoDiffOp::Add(after[j]));
        j += 1;
    }
    ops
}

pub(super) fn managed_io_diff_hunks(before: &[&str], after: &[&str]) -> Vec<String> {
    const CONTEXT: usize = 3;
    let ops = managed_io_diff_ops(before, after);
    if !ops
        .iter()
        .any(|op| matches!(op, ManagedIoDiffOp::Remove(_) | ManagedIoDiffOp::Add(_)))
    {
        return vec![format!("@@ -1,{} +1,{} @@", before.len(), after.len())];
    }

    let mut old_positions = Vec::with_capacity(ops.len());
    let mut new_positions = Vec::with_capacity(ops.len());
    let (mut old_line, mut new_line) = (1usize, 1usize);
    for op in &ops {
        old_positions.push(old_line);
        new_positions.push(new_line);
        match op {
            ManagedIoDiffOp::Context(_) => {
                old_line += 1;
                new_line += 1;
            }
            ManagedIoDiffOp::Remove(_) => old_line += 1,
            ManagedIoDiffOp::Add(_) => new_line += 1,
        }
    }

    let changed_indices = ops
        .iter()
        .enumerate()
        .filter_map(|(idx, op)| {
            matches!(op, ManagedIoDiffOp::Remove(_) | ManagedIoDiffOp::Add(_)).then_some(idx)
        })
        .collect::<Vec<_>>();
    let mut groups = Vec::new();
    for idx in changed_indices {
        let start = idx.saturating_sub(CONTEXT);
        let end = (idx + CONTEXT + 1).min(ops.len());
        if let Some((_, current_end)) = groups.last_mut() {
            if start <= *current_end {
                *current_end = (*current_end).max(end);
                continue;
            }
        }
        groups.push((start, end));
    }

    let mut lines = Vec::new();
    for (start, end) in groups {
        let hunk_ops = &ops[start..end];
        let old_start = old_positions[start];
        let new_start = new_positions[start];
        let old_count = hunk_ops
            .iter()
            .filter(|op| !matches!(op, ManagedIoDiffOp::Add(_)))
            .count();
        let new_count = hunk_ops
            .iter()
            .filter(|op| !matches!(op, ManagedIoDiffOp::Remove(_)))
            .count();
        lines.push(format!(
            "@@ -{},{} +{},{} @@",
            old_start, old_count, new_start, new_count
        ));
        lines.extend(hunk_ops.iter().map(|op| match op {
            ManagedIoDiffOp::Context(line) => format!(" {line}"),
            ManagedIoDiffOp::Remove(line) => format!("-{line}"),
            ManagedIoDiffOp::Add(line) => format!("+{line}"),
        }));
    }
    lines
}

pub(super) fn managed_io_lcs_table(before: &[&str], after: &[&str]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0; after.len() + 1]; before.len() + 1];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            table[i][j] = if before[i] == after[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    table
}

pub(super) fn managed_io_text_for_diff(
    workspace_root: &PathBuf,
    path: &PathBuf,
    allow_missing: bool,
) -> Option<ManagedIoTextSnapshot> {
    let full_path = managed_io_diff_workspace_path(workspace_root, path)?;
    match std::fs::read_to_string(full_path) {
        Ok(text) => Some(ManagedIoTextSnapshot {
            existed: true,
            text,
        }),
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
            Some(ManagedIoTextSnapshot {
                existed: false,
                text: String::new(),
            })
        }
        Err(_) => None,
    }
}

pub(super) fn managed_io_diff_workspace_path(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Option<PathBuf> {
    if path.is_absolute() {
        return None;
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => relative.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    Some(workspace_root.join(relative))
}

pub(super) fn workspace_identity_for_root(
    workspace_root: &PathBuf,
) -> crate::io::WorkspaceIdentity {
    let fingerprint = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.clone())
        .to_string_lossy()
        .to_string();
    let git_root = git_output(workspace_root, &["rev-parse", "--show-toplevel"]);
    let Some(git_root) = git_root else {
        return crate::io::WorkspaceIdentity::local(fingerprint);
    };
    let normalized_git_root = PathBuf::from(git_root.trim())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(git_root.trim()))
        .to_string_lossy()
        .to_string();
    crate::io::WorkspaceIdentity {
        vcs_provider: Some("git".to_string()),
        repo_id: None,
        repo_url: git_output(workspace_root, &["config", "--get", "remote.origin.url"])
            .and_then(|value| non_empty_owned(value.trim())),
        branch: git_output(workspace_root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .and_then(|value| non_empty_owned(value.trim()))
            .map(|branch| {
                if branch == "HEAD" {
                    "detached".to_string()
                } else {
                    branch
                }
            }),
        head_commit: git_output(workspace_root, &["rev-parse", "HEAD"])
            .and_then(|value| non_empty_owned(value.trim())),
        worktree_root_fingerprint: normalized_git_root,
    }
}

pub(super) async fn workspace_identity_for_root_off_thread(
    workspace_root: PathBuf,
) -> Result<crate::io::WorkspaceIdentity, DaemonError> {
    tokio::task::spawn_blocking(move || workspace_identity_for_root(&workspace_root))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "managed_io_workspace_identity",
            message: format!("workspace identity monitor task failed: {error}"),
        })
}

pub(super) fn git_output(workspace_root: &PathBuf, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn non_empty_owned(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

pub(super) fn managed_io_warning_payload(
    warning: crate::io::ArtifactEditWarning,
) -> serde_json::Value {
    match warning {
        crate::io::ArtifactEditWarning::RebasedOverNonOverlappingChange {
            base_version,
            applied_version,
        } => serde_json::json!({
            "kind": "rebased_over_non_overlapping_change",
            "base_version": base_version.value(),
            "applied_version": applied_version.value(),
        }),
    }
}

pub(super) fn managed_io_error_payload(error: crate::io::ArtifactEditError) -> serde_json::Value {
    match error {
        crate::io::ArtifactEditError::ArtifactNotTracked { path } => serde_json::json!({
            "kind": "artifact_not_tracked",
            "path": path.to_string_lossy(),
        }),
        crate::io::ArtifactEditError::SnapshotNotFound { snapshot_id } => serde_json::json!({
            "kind": "snapshot_not_found",
            "snapshot_id": snapshot_id.as_str(),
        }),
        crate::io::ArtifactEditError::UnsupportedDomain { domain } => serde_json::json!({
            "kind": "unsupported_domain",
            "domain": managed_io_domain_name(domain),
        }),
        crate::io::ArtifactEditError::InvalidOperation { message } => serde_json::json!({
            "kind": "invalid_operation",
            "message": message,
        }),
        crate::io::ArtifactEditError::Filesystem { path, message } => serde_json::json!({
            "kind": "filesystem",
            "path": path.to_string_lossy(),
            "message": message,
        }),
        crate::io::ArtifactEditError::ExternalChangeDuringApply { path } => serde_json::json!({
            "kind": "external_change_during_apply",
            "path": path.to_string_lossy(),
        }),
        crate::io::ArtifactEditError::ActiveReservationConflict {
            path,
            active_owner,
            requested_ranges,
            reserved_ranges,
            message,
        } => serde_json::json!({
            "kind": "active_reservation_conflict",
            "path": path.to_string_lossy(),
            "active_owner": managed_io_reservation_owner_payload(active_owner),
            "requested_ranges": requested_ranges.into_iter().map(managed_io_range_payload).collect::<Vec<_>>(),
            "reserved_ranges": reserved_ranges.into_iter().map(managed_io_range_payload).collect::<Vec<_>>(),
            "message": message,
        }),
        crate::io::ArtifactEditError::Conflict {
            path,
            base_version,
            current_version,
            requested_ranges,
            changed_ranges,
            message,
        } => serde_json::json!({
            "kind": "conflict",
            "path": path.to_string_lossy(),
            "base_version": base_version.value(),
            "current_version": current_version.value(),
            "requested_ranges": requested_ranges.into_iter().map(managed_io_range_payload).collect::<Vec<_>>(),
            "changed_ranges": changed_ranges.into_iter().map(managed_io_range_payload).collect::<Vec<_>>(),
            "message": message,
        }),
    }
}

pub(super) fn managed_io_reservation_owner_payload(
    owner: crate::io::ArtifactReservationOwner,
) -> serde_json::Value {
    serde_json::json!({
        "provider_run_id": owner.provider_run_id,
        "agent_instance_id": owner.agent_instance_id,
        "tool_name": owner.tool_name,
    })
}

pub(super) fn managed_io_range_payload(range: crate::io::TextRange) -> serde_json::Value {
    serde_json::json!({
        "start": range.start,
        "end": range.end,
    })
}

pub(super) fn managed_io_domain_name(domain: crate::io::ArtifactDomainKind) -> &'static str {
    match domain {
        crate::io::ArtifactDomainKind::TextDocument => "text",
        crate::io::ArtifactDomainKind::StructuredDocument => "structured",
        crate::io::ArtifactDomainKind::OpaqueBlob => "opaque",
    }
}

pub(super) fn managed_io_daemon_error(error: crate::io::ArtifactEditError) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_managed_io",
        message: managed_io_error_payload(error).to_string(),
    }
}
