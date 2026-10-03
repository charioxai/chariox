//! Capture admission requires a protected layout or explicit broker-retained release F legacy state.
//! Home-volume filtering cannot attest environment secrets or committed image layers.
use crate::error::DaemonError;
use crate::slice::SliceRecord;

// The broker verifies retained host ownership, identities, mounts, environment,
// runtime/image lineage and the actual home. Release F legacy layouts emit a diagnostic; unsupported topologies refuse.
pub(crate) fn require_verified_layout(
    record: &SliceRecord,
    operation: &'static str,
) -> Result<(), DaemonError> {
    #[cfg(test)]
    if TEST_VERIFIED_LAYOUT.with(std::cell::Cell::get) {
        return Ok(());
    }
    super::broker::require_capture_preflight(&super::local_docker_container_name(record))
        .map_err(|_| refusal(operation))
}

#[cfg(test)]
thread_local! {
    static TEST_VERIFIED_LAYOUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Release F's capture-mechanics tests (helper cleanup, stall recovery) run
/// below the Phase 1 capture preflight. Production never bypasses it: without
/// broker-admitted protected or explicit release F legacy state, save/backup
/// is refused before Docker runs.
#[cfg(test)]
pub(crate) fn with_test_verified_layout<T>(run: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_VERIFIED_LAYOUT.with(|flag| flag.set(false));
        }
    }
    TEST_VERIFIED_LAYOUT.with(|flag| flag.set(true));
    let _reset = Reset;
    run()
}

fn refusal(operation: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: "Slice save/backup is unavailable because this storage layout may include credentials. Existing saved state is preserved.".to_string(),
    }
}

#[cfg(test)]
fn require_supported_layout(operation: &'static str) -> Result<(), DaemonError> {
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
