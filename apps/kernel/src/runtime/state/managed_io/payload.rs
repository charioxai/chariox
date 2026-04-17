use super::*;

pub(super) fn managed_io_warning_payload(
    warning: crate::io::ArtifactEditWarning,
) -> serde_json::Value {
    match warning {
        crate::io::ArtifactEditWarning::RebasedOverNonOverlappingChange {
            base_version,
            applied_version,
        } => serde_json::json!({
            "kind": "rebased_over_non_overlapping_change",
            "base_version": base_version.value(),
            "applied_version": applied_version.value(),
        }),
    }
}

pub(super) fn managed_io_error_payload(error: crate::io::ArtifactEditError) -> serde_json::Value {
    match error {
        crate::io::ArtifactEditError::ArtifactNotTracked { path } => serde_json::json!({
            "kind": "artifact_not_tracked",
            "path": path.to_string_lossy(),
        }),
        crate::io::ArtifactEditError::SnapshotNotFound { snapshot_id } => serde_json::json!({
            "kind": "snapshot_not_found",
            "snapshot_id": snapshot_id.as_str(),
        }),
        crate::io::ArtifactEditError::UnsupportedDomain { domain } => serde_json::json!({
            "kind": "unsupported_domain",
            "domain": managed_io_domain_name(domain),
        }),
        crate::io::ArtifactEditError::InvalidOperation { message } => serde_json::json!({
            "kind": "invalid_operation",
            "message": message,
        }),
        crate::io::ArtifactEditError::Filesystem { path, message } => serde_json::json!({
            "kind": "filesystem",
            "path": path.to_string_lossy(),
            "message": message,
        }),
        crate::io::ArtifactEditError::ExternalChangeDuringApply { path } => serde_json::json!({
            "kind": "external_change_during_apply",
            "path": path.to_string_lossy(),
        }),
        crate::io::ArtifactEditError::ActiveReservationConflict {
            path,
            active_owner,
            requested_ranges,
            reserved_ranges,
            message,
        } => serde_json::json!({
            "kind": "active_reservation_conflict",
            "path": path.to_string_lossy(),
            "active_owner": managed_io_reservation_owner_payload(active_owner),
            "requested_ranges": requested_ranges.into_iter().map(managed_io_range_payload).collect::<Vec<_>>(),
            "reserved_ranges": reserved_ranges.into_iter().map(managed_io_range_payload).collect::<Vec<_>>(),
            "message": message,
        }),
        crate::io::ArtifactEditError::Conflict {
            path,
            base_version,
            current_version,
            requested_ranges,
            changed_ranges,
            message,
        } => serde_json::json!({
            "kind": "conflict",
            "path": path.to_string_lossy(),
            "base_version": base_version.value(),
            "current_version": current_version.value(),
            "requested_ranges": requested_ranges.into_iter().map(managed_io_range_payload).collect::<Vec<_>>(),
            "changed_ranges": changed_ranges.into_iter().map(managed_io_range_payload).collect::<Vec<_>>(),
            "message": message,
        }),
    }
}

pub(super) fn managed_io_reservation_owner_payload(
    owner: crate::io::ArtifactReservationOwner,
) -> serde_json::Value {
    serde_json::json!({
        "provider_run_id": owner.provider_run_id,
        "agent_instance_id": owner.agent_instance_id,
        "tool_name": owner.tool_name,
    })
}

pub(super) fn managed_io_range_payload(range: crate::io::TextRange) -> serde_json::Value {
    serde_json::json!({
        "start": range.start,
        "end": range.end,
    })
}

pub(super) fn managed_io_domain_name(domain: crate::io::ArtifactDomainKind) -> &'static str {
    match domain {
        crate::io::ArtifactDomainKind::TextDocument => "text",
        crate::io::ArtifactDomainKind::StructuredDocument => "structured",
        crate::io::ArtifactDomainKind::OpaqueBlob => "opaque",
    }
}

pub(in crate::runtime::state) fn managed_io_daemon_error(
    error: crate::io::ArtifactEditError,
) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_managed_io",
        message: managed_io_error_payload(error).to_string(),
    }
}
