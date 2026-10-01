//! Containment until every capture boundary has a verified credential-separated layout.
//! Home-volume filtering cannot attest environment secrets or committed image layers.
use crate::error::DaemonError;
use crate::slice::SliceRecord;

// Prepared completing seam. Existing call sites remain on the containment
// refusal until the protected layout and restore path pass source approval.
pub(crate) fn require_verified_layout(
    record: &SliceRecord,
    operation: &'static str,
) -> Result<(), DaemonError> {
    super::broker::require_capture_preflight(&super::local_docker_container_name(record))
        .map_err(|_| refusal(operation))
}

fn refusal(operation: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: "Slice save/backup is unavailable because this storage layout may include credentials. Existing saved state is preserved.".to_string(),
    }
}

pub(crate) fn require_supported_layout(operation: &'static str) -> Result<(), DaemonError> {
    // No current provisioner produces a verified protected capture layout. Do not
    // accept a label, environment flag, or synthetic mount list as that proof.
    // A completing change must verify actual private mounts, the immutable base,
    // container configuration and the archive sink before introducing acceptance.
    Err(refusal(operation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_capture_refuses_save_backup_and_rollback_without_secret_details() {
        for operation in [
            "slice.state.save",
            "slice.backup.create",
            "slice.backup.restore",
        ] {
            let error = require_supported_layout(operation).unwrap_err();
            assert!(error
                .to_string()
                .contains("unavailable because this storage layout"));
            assert!(error
                .to_string()
                .contains("Existing saved state is preserved"));
        }
    }
}
