//! Forwarded read-artifact handling on the home kernel.

use super::*;

pub(super) fn dispatch_forwarded_read(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    context: &crate::transport::relay_peer::RemoteManagedIoContext,
    arguments: serde_json::Value,
    artifact_states: &[crate::transport::relay_peer::RemoteManagedIoArtifactState],
    workspace_context: &ManagedIoWorkspaceContext,
) -> ForwardedManagedIoResult {
    let args = serde_json::from_value::<crate::transport::runtime_tools::ManagedReadArtifactArgs>(
        arguments,
    )
    .map_err(|error| DaemonError::LocalTransport {
        operation: "forwarded_managed_io_read_artifact",
        message: format!("invalid tool arguments: {error}"),
    })?;
    let domain = KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
    let state = remote_managed_io_state_for_path(artifact_states, &PathBuf::from(&args.path))
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "forwarded_managed_io_read_artifact",
            message: "missing forwarded artifact state".to_string(),
        })?;
    let content = remote_managed_io_content_from_state(state, domain)?;
    let read = coordinator.read_artifact(crate::io::ArtifactReadRequest {
        workspace_identity: context.worker_workspace_identity.clone(),
        path: PathBuf::from(args.path),
        domain,
        content,
    });
    let mut payload = managed_io_read_payload(read);
    add_managed_io_workspace_payload(&mut payload, workspace_context);
    Ok((
        crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
        Vec::new(),
    ))
}
