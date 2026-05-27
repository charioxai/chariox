//! Remote workspace live sync coordination snapshots.
//!
//! Remote agents use these helpers to discover current artifact state and workspace identity so
//! same-repo/same-branch workers can participate in the same collision-control protocol.

use super::*;

pub(in crate::runtime::state) fn remote_workspace_live_sync_artifact_states_for_tool(
    workspace_root: &PathBuf,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Result<Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>, DaemonError> {
    match tool_name {
        crate::transport::runtime_tools::READ_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncReadArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_workspace_live_sync_read_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let path = PathBuf::from(args.path);
            let domain =
                KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
            let content = workspace_live_sync_read_optional_content(workspace_root, &path, domain)?;
            Ok(vec![remote_workspace_live_sync_state_from_content_with_domain(
                &path, content, domain,
            )])
        }
        crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncEditArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_workspace_live_sync_edit_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let path = PathBuf::from(args.path);
            let content = workspace_live_sync_read_optional_text(workspace_root, &path)?;
            Ok(vec![remote_workspace_live_sync_state(&path, content)])
        }
        crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncWriteArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_workspace_live_sync_write_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let path = PathBuf::from(args.path);
            let domain =
                KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
            let content = workspace_live_sync_read_optional_content(workspace_root, &path, domain)?;
            Ok(vec![remote_workspace_live_sync_state_from_content_with_domain(
                &path, content, domain,
            )])
        }
        crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncApplyPatchArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_workspace_live_sync_apply_patch_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let operations = parse_managed_apply_patch(&args.patch_text)?;
            remote_workspace_live_sync_states_for_patch_operations(workspace_root, &operations)
        }
        crate::transport::runtime_tools::DELETE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncDeleteArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_workspace_live_sync_delete_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let path = PathBuf::from(args.path);
            let domain =
                KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
            let content = workspace_live_sync_read_optional_content(workspace_root, &path, domain)?;
            Ok(vec![remote_workspace_live_sync_state_from_content_with_domain(
                &path, content, domain,
            )])
        }
        crate::transport::runtime_tools::MOVE_ARTIFACT_TOOL => {
            let args = serde_json::from_value::<
                crate::transport::runtime_tools::WorkspaceLiveSyncMoveArtifactArgs,
            >(arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "remote_workspace_live_sync_move_state",
                message: format!("invalid tool arguments: {error}"),
            })?;
            let domain =
                KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
            if domain == crate::io::ArtifactDomainKind::TextDocument {
                let operations = vec![ManagedPatchOperation::Move {
                    from_path: PathBuf::from(args.from_path),
                    to_path: PathBuf::from(args.to_path),
                    old_text: args.old_text,
                    new_text: args.new_text,
                }];
                remote_workspace_live_sync_states_for_patch_operations(workspace_root, &operations)
            } else {
                let from_path = PathBuf::from(args.from_path);
                let to_path = PathBuf::from(args.to_path);
                let from_content =
                    workspace_live_sync_read_optional_content(workspace_root, &from_path, domain)?;
                let to_content =
                    workspace_live_sync_read_optional_content(workspace_root, &to_path, domain)?;
                Ok(vec![
                    remote_workspace_live_sync_state_from_content_with_domain(
                        &from_path,
                        from_content,
                        domain,
                    ),
                    remote_workspace_live_sync_state_from_content_with_domain(&to_path, to_content, domain),
                ])
            }
        }
        _ => Ok(Vec::new()),
    }
}

pub(in crate::runtime::state) fn remote_workspace_live_sync_states_for_patch_operations(
    workspace_root: &PathBuf,
    operations: &[ManagedPatchOperation],
) -> Result<Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>, DaemonError> {
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
            let content = workspace_live_sync_read_optional_text(workspace_root, &path)?;
            Ok(remote_workspace_live_sync_state(&path, content))
        })
        .collect()
}

pub(in crate::runtime::state) fn apply_remote_workspace_live_sync_final_states(
    workspace_root: &PathBuf,
    initial_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    final_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
    for final_state in final_states {
        let path = PathBuf::from(&final_state.path);
        let initial = remote_workspace_live_sync_state_for_path(initial_states, &path);
        let domain = remote_workspace_live_sync_state_domain(final_state);
        let current = remote_workspace_live_sync_state_from_content_with_domain(
            &path,
            workspace_live_sync_read_optional_content(workspace_root, &path, domain)?,
            domain,
        );
        let expected = initial.cloned().unwrap_or_else(|| {
            remote_workspace_live_sync_state_from_content_with_domain(&path, None, domain)
        });
        if !remote_workspace_live_sync_states_content_equal(&current, &expected) {
            return Ok(Some(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "applied": false,
                    "reason": {
                        "kind": "external_change_during_remote_apply",
                        "path": path.to_string_lossy(),
                        "message": "The remote workspace artifact changed after the home kernel accepted the workspace live sync operation but before the worker could apply it."
                    },
                    "next_action": "Reread the artifact with arroba.read_artifact, reconcile with the current content, and retry through Arroba workspace live sync.",
                }),
            }));
        }
    }
    let mut states = BTreeMap::new();
    for state in final_states {
        let path = PathBuf::from(&state.path);
        let content = if state.exists {
            Some(remote_workspace_live_sync_content_from_state(
                state,
                remote_workspace_live_sync_state_domain(state),
            )?)
        } else {
            None
        };
        states.insert(path, content);
    }
    workspace_live_sync_write_final_content_states(workspace_root, &states)?;
    Ok(None)
}

pub(in crate::runtime::state) fn leased_workflow_tool_result_should_complete_turn(
    tool_name: &str,
    result: &crate::transport::runtime_tools::RuntimeToolResult,
) -> bool {
    if !result.ok {
        return false;
    }
    match tool_name {
        crate::transport::runtime_tools::VALIDATE_WORKFLOW_HANDOFF_TOOL => {
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
