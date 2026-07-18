use crate::error::DaemonError;

use super::SliceHostRuntimeState;
use crate::slice::SliceStatus;

pub(super) fn reconcile_slice_status_after_kernel_restart(
    status: SliceStatus,
    host_runtime: SliceHostRuntimeState,
) -> SliceStatus {
    match status {
        SliceStatus::Starting | SliceStatus::Stopping => SliceStatus::Unhealthy,
        SliceStatus::Running => match host_runtime {
            SliceHostRuntimeState::Stopped | SliceHostRuntimeState::Missing => SliceStatus::Stopped,
            SliceHostRuntimeState::Running => SliceStatus::Running,
            SliceHostRuntimeState::Unknown => SliceStatus::Unhealthy,
        },
        SliceStatus::Stopped => match host_runtime {
            SliceHostRuntimeState::Running => SliceStatus::Running,
            SliceHostRuntimeState::Stopped
            | SliceHostRuntimeState::Missing
            | SliceHostRuntimeState::Unknown => SliceStatus::Stopped,
        },
        SliceStatus::Unhealthy => match host_runtime {
            SliceHostRuntimeState::Stopped | SliceHostRuntimeState::Missing => SliceStatus::Stopped,
            SliceHostRuntimeState::Running => SliceStatus::Running,
            SliceHostRuntimeState::Unknown => SliceStatus::Unhealthy,
        },
    }
}

pub(super) fn validate_slice_name(name: &str) -> Result<(), DaemonError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "slice.validate",
            message: "slice name must not be empty".to_string(),
        });
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(DaemonError::LocalTransport {
            operation: "slice.validate",
            message: "slice name may only contain ASCII letters, numbers, '-', '_' or '.'"
                .to_string(),
        });
    }
    Ok(())
}

pub(super) fn redact_slice_operation_error(error: &str) -> String {
    error
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect()
}
