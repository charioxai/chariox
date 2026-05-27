//! Forwarded text read-before-write/edit handling on the home kernel.

use super::*;

pub(super) fn dispatch_forwarded_edit(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    tool_name: &str,
    arguments: serde_json::Value,
    artifact_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    workspace_context: &WorkspaceLiveSyncWorkspaceContext,
) -> ForwardedWorkspaceLiveSyncResult {
    let args = serde_json::from_value::<crate::transport::runtime_tools::WorkspaceLiveSyncEditArtifactArgs>(
        arguments,
    )
    .map_err(|error| DaemonError::LocalTransport {
        operation: "forwarded_workspace_live_sync_edit_artifact",
        message: format!("invalid tool arguments: {error}"),
    })?;
    let domain = KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
    if domain != crate::io::ArtifactDomainKind::TextDocument {
        return Err(DaemonError::LocalTransport {
            operation: "forwarded_workspace_live_sync_edit_artifact",
            message: "remote managed edit currently supports only text artifacts".to_string(),
        });
    }
    let operation = workspace_live_sync_edit_operation_from_args(args.clone())?;
    let path = PathBuf::from(args.path.clone());
    workspace_live_sync_reject_ignored_path(
        &workspace_context.root,
        &path,
        "forwarded_workspace_live_sync_edit_artifact",
    )?;
    let state =
        forwarded_artifact_state(artifact_states, &path, "forwarded_workspace_live_sync_edit_artifact")?;
    let before = remote_workspace_live_sync_text_snapshot_from_state(state);
    coordinator.read_artifact(crate::io::ArtifactReadRequest {
        workspace_identity: context.worker_workspace_identity.clone(),
        path: path.clone(),
        domain,
        content: remote_workspace_live_sync_content_from_state(state, domain)?,
    });
    let reservation = match workspace_live_sync_try_reserve_ranges(
        coordinator,
        &context.worker_workspace_identity,
        &path,
        workspace_live_sync_reservation_ranges_for_operation(
            &operation,
            before.as_ref(),
            crate::io::TextRange::new(0, usize::MAX),
        ),
        remote_reservation_owner(context, tool_name),
    ) {
        Ok(reservation) => reservation,
        Err(mut result) => {
            add_workspace_live_sync_workspace_payload(&mut result.payload, workspace_context);
            return Ok((result, Vec::new()));
        }
    };
    let result = coordinator.apply_edit(crate::io::ArtifactWriteRequest {
        workspace_identity: context.worker_workspace_identity.clone(),
        intent: crate::io::AgentEditIntent {
            path: path.clone(),
            snapshot_id: workspace_live_sync_snapshot_id_from_arg(args.snapshot_id.clone()),
            operation,
        },
    });
    coordinator.release_reservation(reservation);
    let after = workspace_live_sync_result_applied(&result)
        .then(|| current_text_snapshot(coordinator, &workspace_context.identity, &path))
        .flatten();
    let final_states = after
        .as_ref()
        .map(|after| vec![remote_workspace_live_sync_state(&path, Some(after.text.clone()))])
        .unwrap_or_default();
    let mut output = workspace_live_sync_edit_result(
        result,
        WorkspaceLiveSyncChangeContext {
            path,
            before,
            after,
        },
        None,
    );
    add_workspace_live_sync_workspace_payload(&mut output.payload, workspace_context);
    Ok((output, final_states))
}

pub(super) fn dispatch_forwarded_write(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    tool_name: &str,
    arguments: serde_json::Value,
    artifact_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    workspace_context: &WorkspaceLiveSyncWorkspaceContext,
) -> ForwardedWorkspaceLiveSyncResult {
    let args = serde_json::from_value::<crate::transport::runtime_tools::WorkspaceLiveSyncWriteArtifactArgs>(
        arguments,
    )
    .map_err(|error| DaemonError::LocalTransport {
        operation: "forwarded_workspace_live_sync_write_artifact",
        message: format!("invalid tool arguments: {error}"),
    })?;
    let domain = KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
    let path = PathBuf::from(args.path.clone());
    workspace_live_sync_reject_ignored_path(
        &workspace_context.root,
        &path,
        "forwarded_workspace_live_sync_write_artifact",
    )?;
    let state = forwarded_artifact_state(
        artifact_states,
        &path,
        "forwarded_workspace_live_sync_write_artifact",
    )?;
    let before = remote_workspace_live_sync_text_snapshot_from_state(state);
    coordinator.read_artifact(crate::io::ArtifactReadRequest {
        workspace_identity: context.worker_workspace_identity.clone(),
        path: path.clone(),
        domain,
        content: remote_workspace_live_sync_content_from_state(state, domain)?,
    });
    let reservation = match workspace_live_sync_try_reserve_ranges(
        coordinator,
        &context.worker_workspace_identity,
        &path,
        vec![crate::io::TextRange::new(0, usize::MAX)],
        remote_reservation_owner(context, tool_name),
    ) {
        Ok(reservation) => reservation,
        Err(mut result) => {
            add_workspace_live_sync_workspace_payload(&mut result.payload, workspace_context);
            return Ok((result, Vec::new()));
        }
    };
    let result = coordinator.apply_edit(crate::io::ArtifactWriteRequest {
        workspace_identity: context.worker_workspace_identity.clone(),
        intent: crate::io::AgentEditIntent {
            path: path.clone(),
            snapshot_id: workspace_live_sync_write_snapshot_id_from_arg(args.snapshot_id.clone(), &path),
            operation: crate::io::AgentEditOperation::WriteArtifact {
                content: workspace_live_sync_write_content_from_args(
                    "forwarded_workspace_live_sync_write_artifact",
                    domain,
                    &args,
                )?,
            },
        },
    });
    coordinator.release_reservation(reservation);
    let (after, final_states) =
        forwarded_write_after_state(coordinator, workspace_context, &path, domain, &result);
    let mut output = workspace_live_sync_edit_result(
        result,
        WorkspaceLiveSyncChangeContext {
            path,
            before,
            after,
        },
        None,
    );
    add_workspace_live_sync_workspace_payload(&mut output.payload, workspace_context);
    Ok((output, final_states))
}

fn forwarded_artifact_state<'a>(
    artifact_states: &'a [crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    path: &PathBuf,
    operation: &'static str,
) -> Result<&'a crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState, DaemonError> {
    remote_workspace_live_sync_state_for_path(artifact_states, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation,
            message: "missing forwarded artifact state".to_string(),
        }
    })
}

fn remote_reservation_owner(
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    tool_name: &str,
) -> crate::io::ArtifactReservationOwner {
    crate::io::ArtifactReservationOwner::new(
        format!("remote:{}", context.worker_provider_run_id),
        Some(context.home_agent_id.clone()),
        tool_name.to_string(),
    )
}

fn current_text_snapshot(
    coordinator: &crate::io::ArtifactEditCoordinator,
    workspace_identity: &crate::io::WorkspaceIdentity,
    path: &PathBuf,
) -> Option<WorkspaceLiveSyncTextSnapshot> {
    let artifact_id = coordinator.resolve_artifact_id(workspace_identity, path);
    coordinator
        .current_content(&artifact_id)
        .and_then(|content| content.as_text().map(str::to_string))
        .map(|text| WorkspaceLiveSyncTextSnapshot {
            existed: true,
            text,
        })
}

fn forwarded_write_after_state(
    coordinator: &crate::io::ArtifactEditCoordinator,
    workspace_context: &WorkspaceLiveSyncWorkspaceContext,
    path: &PathBuf,
    domain: crate::io::ArtifactDomainKind,
    result: &crate::io::EditResult,
) -> (
    Option<WorkspaceLiveSyncTextSnapshot>,
    Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
) {
    if !workspace_live_sync_result_applied(result) {
        return (None, Vec::new());
    }
    let artifact_id = coordinator.resolve_artifact_id(&workspace_context.identity, path);
    let content = coordinator.current_content(&artifact_id).cloned();
    let after = content.as_ref().and_then(|content| match content {
        crate::io::ArtifactContent::Text(text) => Some(WorkspaceLiveSyncTextSnapshot {
            existed: true,
            text: text.clone(),
        }),
        crate::io::ArtifactContent::Bytes(_) => None,
    });
    let final_states = content
        .map(|content| {
            vec![remote_workspace_live_sync_state_from_content_with_domain(
                path,
                Some(content),
                domain,
            )]
        })
        .unwrap_or_default();
    (after, final_states)
}
