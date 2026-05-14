//! Managed-I/O reservation ownership and range policy.

use super::*;

pub(in crate::runtime::state) fn managed_io_reservation_ranges_for_operation(
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

pub(in crate::runtime::state) fn managed_io_try_reserve_ranges(
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

pub(in crate::runtime::state) fn managed_io_reservation_owner(
    provider_run: &crate::provider::RuntimeProviderRun,
    tool_name: &str,
) -> crate::io::ArtifactReservationOwner {
    crate::io::ArtifactReservationOwner::new(
        provider_run.id().to_string(),
        provider_run.agent_instance_id().map(str::to_string),
        tool_name.to_string(),
    )
}
