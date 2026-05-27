//! Forwarded patch, delete, and move handling on the home kernel.

use super::*;

pub(super) fn dispatch_forwarded_apply_patch(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    tool_name: &str,
    arguments: serde_json::Value,
    artifact_states: Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
    workspace_context: &WorkspaceLiveSyncWorkspaceContext,
) -> ForwardedWorkspaceLiveSyncResult {
    let args = serde_json::from_value::<
        crate::transport::runtime_tools::WorkspaceLiveSyncApplyPatchArgs,
    >(arguments)
    .map_err(|error| DaemonError::LocalTransport {
        operation: "forwarded_workspace_live_sync_apply_patch",
        message: format!("invalid tool arguments: {error}"),
    })?;
    let domain =
        KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
    if domain != crate::io::ArtifactDomainKind::TextDocument {
        return Err(DaemonError::LocalTransport {
            operation: "forwarded_workspace_live_sync_apply_patch",
            message: "remote managed apply_patch currently supports only text artifacts"
                .to_string(),
        });
    }
    let operations = parse_managed_apply_patch(&args.patch_text)?;
    apply_remote_managed_patch_operations(
        coordinator,
        context.worker_workspace_identity.clone(),
        domain,
        operations,
        artifact_states,
        remote_reservation_owner(context, tool_name),
        workspace_context,
    )
}

pub(super) fn dispatch_forwarded_delete(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    tool_name: &str,
    arguments: serde_json::Value,
    artifact_states: Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
    workspace_context: &WorkspaceLiveSyncWorkspaceContext,
) -> ForwardedWorkspaceLiveSyncResult {
    let args = serde_json::from_value::<
        crate::transport::runtime_tools::WorkspaceLiveSyncDeleteArtifactArgs,
    >(arguments)
    .map_err(|error| DaemonError::LocalTransport {
        operation: "forwarded_workspace_live_sync_delete_artifact",
        message: format!("invalid tool arguments: {error}"),
    })?;
    let domain =
        KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
    if domain == crate::io::ArtifactDomainKind::TextDocument {
        apply_remote_managed_patch_operations(
            coordinator,
            context.worker_workspace_identity.clone(),
            domain,
            vec![ManagedPatchOperation::Delete {
                path: PathBuf::from(args.path),
            }],
            artifact_states,
            remote_reservation_owner(context, tool_name),
            workspace_context,
        )
    } else {
        apply_remote_managed_whole_file_operations(
            coordinator,
            context.worker_workspace_identity.clone(),
            domain,
            vec![ManagedWholeFileOperation::Delete {
                path: PathBuf::from(args.path),
            }],
            artifact_states,
            remote_reservation_owner(context, tool_name),
            workspace_context,
        )
    }
}

pub(super) fn dispatch_forwarded_move(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    tool_name: &str,
    arguments: serde_json::Value,
    artifact_states: Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
    workspace_context: &WorkspaceLiveSyncWorkspaceContext,
) -> ForwardedWorkspaceLiveSyncResult {
    let args = serde_json::from_value::<
        crate::transport::runtime_tools::WorkspaceLiveSyncMoveArtifactArgs,
    >(arguments)
    .map_err(|error| DaemonError::LocalTransport {
        operation: "forwarded_workspace_live_sync_move_artifact",
        message: format!("invalid tool arguments: {error}"),
    })?;
    let domain =
        KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
    let (old_text, new_text) = args.normalized_text_transform_fields();
    if domain == crate::io::ArtifactDomainKind::TextDocument {
        apply_remote_managed_patch_operations(
            coordinator,
            context.worker_workspace_identity.clone(),
            domain,
            vec![ManagedPatchOperation::Move {
                from_path: PathBuf::from(args.from_path),
                to_path: PathBuf::from(args.to_path),
                old_text,
                new_text,
            }],
            artifact_states,
            remote_reservation_owner(context, tool_name),
            workspace_context,
        )
    } else {
        if args.has_non_text_transform_fields() {
            return Err(DaemonError::LocalTransport {
                operation: "forwarded_workspace_live_sync_move_artifact",
                message:
                    "non-text managed moves cannot transform content; omit old_text and new_text"
                        .to_string(),
            });
        }
        apply_remote_managed_whole_file_operations(
            coordinator,
            context.worker_workspace_identity.clone(),
            domain,
            vec![ManagedWholeFileOperation::Move {
                from_path: PathBuf::from(args.from_path),
                to_path: PathBuf::from(args.to_path),
            }],
            artifact_states,
            remote_reservation_owner(context, tool_name),
            workspace_context,
        )
    }
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
