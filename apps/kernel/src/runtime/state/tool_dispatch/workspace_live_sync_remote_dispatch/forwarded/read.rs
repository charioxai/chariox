//! Forwarded read-artifact handling on the home kernel.

use super::*;

pub(super) fn dispatch_forwarded_read(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    _context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
    arguments: serde_json::Value,
    artifact_states: &[crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState],
    workspace_context: &WorkspaceLiveSyncWorkspaceContext,
) -> ForwardedWorkspaceLiveSyncResult {
    let args = serde_json::from_value::<
        crate::transport::runtime_tools::WorkspaceLiveSyncReadArtifactArgs,
    >(arguments)
    .map_err(|error| DaemonError::LocalTransport {
        operation: "forwarded_workspace_live_sync_read_artifact",
        message: format!("invalid tool arguments: {error}"),
    })?;
    let domain =
        KernelRuntimeOwnedState::workspace_live_sync_domain_from_arg(args.domain.as_deref())?;
    let state =
        remote_workspace_live_sync_state_for_path(artifact_states, &PathBuf::from(&args.path))
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "forwarded_workspace_live_sync_read_artifact",
                message: "missing forwarded artifact state".to_string(),
            })?;
    let content = remote_workspace_live_sync_content_from_state(state, domain)?;
    let read = coordinator.read_artifact(crate::io::ArtifactReadRequest {
        workspace_identity: workspace_context.identity.clone(),
        path: PathBuf::from(args.path),
        domain,
        content,
    });
    let mut payload = workspace_live_sync_read_payload(read);
    add_workspace_live_sync_workspace_payload(&mut payload, workspace_context);
    Ok((
        crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
        Vec::new(),
    ))
}
