//! Managed-I/O apply-patch orchestration.
//!
//! This module validates patch requests against tracked workspace state, applies accepted edits,
//! and emits atomic patch result payloads.

use super::*;

pub(in crate::runtime::state) fn apply_managed_patch_operations(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: crate::io::WorkspaceIdentity,
    workspace_root: PathBuf,
    domain: crate::io::ArtifactDomainKind,
    operations: Vec<ManagedPatchOperation>,
    reservation_owner: crate::io::ArtifactReservationOwner,
    external_change_monitor: &crate::io::ArtifactExternalChangeMonitor,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let plan = match plan_managed_patch_operations(&workspace_root, operations)? {
        ManagedPatchPlanOutcome::Planned(plan) => plan,
        ManagedPatchPlanOutcome::Rejected(output) => return Ok(output),
    };
    let ManagedPatchPlan {
        before_states,
        final_states,
        reservation_ranges,
    } = plan;

    let mut reservations = Vec::new();
    for (path, ranges) in reservation_ranges {
        match managed_io_try_reserve_ranges(
            coordinator,
            &workspace_identity,
            &path,
            ranges,
            reservation_owner.clone(),
        ) {
            Ok(token) => reservations.push(token),
            Err(output) => {
                for token in reservations {
                    coordinator.release_reservation(token);
                }
                return Ok(output);
            }
        }
    }

    let external_change_notices = external_change_monitor
        .external_change_notices(&workspace_identity, final_states.keys().cloned());

    for (path, before) in &before_states {
        let latest = match managed_io_read_optional_text(&workspace_root, path) {
            Ok(latest) => latest,
            Err(error) => {
                for token in reservations {
                    coordinator.release_reservation(token);
                }
                return Err(error);
            }
        };
        if &latest != before {
            for token in reservations {
                coordinator.release_reservation(token);
            }
            external_change_monitor.record_external_change(&workspace_identity, path);
            let mut notices = external_change_notices.clone();
            if !notices.iter().any(|notice| notice.path == *path) {
                notices.push(managed_io_external_change_notice_for_path(path.clone()));
            }
            let mut output = managed_patch_rejected(
                path.clone(),
                "artifact changed while the managed patch was being prepared; reread and retry",
            );
            add_managed_io_external_change_notices_payload(&mut output.payload, notices);
            return Ok(output);
        }
    }

    if let Err(error) = managed_io_write_final_states(&workspace_root, &final_states) {
        let _ = managed_io_write_final_states(&workspace_root, &before_states);
        for token in reservations {
            coordinator.release_reservation(token);
        }
        return Err(error);
    }

    for (path, after) in &final_states {
        match after {
            Some(text) => {
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: crate::io::ArtifactContent::Text(text.clone()),
                });
            }
            None => coordinator.forget_artifact(&workspace_identity, path),
        }
        external_change_monitor.observe_managed_write(
            reservation_owner.provider_run_id.as_str(),
            &workspace_identity,
            &workspace_root,
            path,
        );
    }
    for token in reservations {
        coordinator.release_reservation(token);
    }

    let mut changes = Vec::new();
    for (path, after) in final_states {
        let before =
            before_states
                .get(&path)
                .cloned()
                .flatten()
                .map(|text| ManagedIoTextSnapshot {
                    existed: true,
                    text,
                });
        let after = after.map(|text| ManagedIoTextSnapshot {
            existed: true,
            text,
        });
        let mut change_payload = serde_json::json!({});
        add_managed_io_change_payload(
            &mut change_payload,
            ManagedIoChangeContext {
                path,
                before,
                after,
            },
        );
        if let Some(change) = change_payload.get("change") {
            changes.push(change.clone());
        }
    }

    let mut payload = serde_json::json!({
        "applied": true,
        "atomic": true,
        "changes": changes,
    });
    add_managed_io_external_change_notices_payload(&mut payload, external_change_notices);
    if changes.len() == 1 {
        payload["change"] = changes[0].clone();
        if let Some(path) = changes[0].get("path").cloned() {
            payload["path"] = path;
        }
    }
    Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
}

pub(in crate::runtime::state) fn replace_unique_text(
    current: &str,
    old_text: &str,
    new_text: &str,
) -> Option<(crate::io::TextRange, String)> {
    let start = current.find(old_text)?;
    if current[start + old_text.len()..].contains(old_text) {
        return None;
    }
    let range = crate::io::TextRange::new(start, start + old_text.len());
    let mut updated = String::with_capacity(current.len() - old_text.len() + new_text.len());
    updated.push_str(&current[..start]);
    updated.push_str(new_text);
    updated.push_str(&current[start + old_text.len()..]);
    Some((range, updated))
}

pub(in crate::runtime::state) fn managed_patch_rejected(
    path: PathBuf,
    message: impl Into<String>,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({
            "applied": false,
            "reason": {
                "kind": "invalid_operation",
                "path": path.to_string_lossy(),
                "message": message.into(),
            },
            "next_action": "Reread the affected artifact with arroba.read_artifact, reconcile with the current content, and retry through Arroba managed I/O.",
        }),
    }
}
