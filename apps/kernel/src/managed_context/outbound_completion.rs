//! MP-08/MP-11: durable completion before cleanup, and consumed-transfer recovery.
use super::*;
use crate::managed_context::outbound::{
    ManagedContextOutboundResume, ManagedContextOutboundTransferResult,
};

pub(super) fn load_transfer_checkpoint(
    root: &Path,
) -> Result<Option<ManagedContextOutboundResume>, DaemonError> {
    let path = root.join("transfer.json");
    if !path_entry_exists(&path)? {
        return Ok(None);
    }
    let bytes = read_bounded_regular_file(&path, 1024)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| outbound_service_error("invalid outbound transfer checkpoint", false))
}

pub(super) fn persist_transfer_checkpoint(
    root: &Path,
    checkpoint: &ManagedContextOutboundResume,
) -> Result<(), DaemonError> {
    let bytes = serde_json::to_vec(checkpoint)
        .map_err(|_| outbound_service_error("serialize outbound transfer checkpoint", true))?;
    crate::config::write_private_file(&root.join("transfer.json"), &bytes)
        .map_err(|error| outbound_service_io_error("persist outbound transfer checkpoint", error))
}

pub(super) fn complete_outbound_operation(
    store: &ManagedContextOutboundOperationStore,
    context_id: &str,
    artifact_root: &Path,
    result: ManagedContextOutboundTransferResult,
) {
    // MP-08: retain archive, capability and target transfer binding if the final
    // durable write fails. After restart retry queries that exact target receipt.
    let persisted = store.update(context_id, |status| {
        status.phase = ManagedContextOutboundOperationPhase::Completed;
        status.accepted_bytes = result.package_size_bytes;
        status.package_size_bytes = result.package_size_bytes;
        status.receipt = Some(result.receipt.into());
        status.failure_code = None;
        status.failure_message = None;
        status.retryable = false;
    });
    if persisted {
        // Cleanup failure cannot erase an authoritative durable completion.
        let _ = remove_artifact_root(artifact_root);
    }
}

// MP-08/MP-11: settle a crash after Completed was fsynced but before cleanup.
pub(super) fn cleanup_durable_completion(
    parent: &Path,
    context_id: &str,
    artifact: &PersistedOutboundArtifact,
) -> Result<bool, DaemonError> {
    let path = parent
        .join(".operations")
        .join(format!("{context_id}.json"));
    let status = read_bounded_regular_file(&path, 256 * 1024)
        .ok()
        .and_then(|bytes| {
            serde_json::from_slice::<ManagedContextOutboundOperationStatus>(&bytes).ok()
        });
    let completed = status.is_some_and(|status| {
        status.context_id == context_id
            && status.plan_digest == artifact.plan_digest
            && status.phase == ManagedContextOutboundOperationPhase::Completed
            && status.receipt.is_some_and(|receipt| {
                receipt.archive_sha256 == artifact.package_sha256
                    && receipt.plan_digest == artifact.plan_digest
                    && receipt.destination == artifact.destination
            })
    });
    if completed {
        remove_artifact_root(&parent.join(context_id))?;
    }
    Ok(completed)
}
