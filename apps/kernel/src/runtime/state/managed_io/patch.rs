//! Managed-I/O apply-patch orchestration.
//!
//! This module validates patch requests against tracked workspace state, applies accepted edits,
//! emits file-change output records, and records external-change conflicts for agent notification.

use super::*;

pub(in crate::runtime::state) fn managed_io_edit_result(
    result: crate::io::EditResult,
    change: ManagedIoChangeContext,
    external_change_notice: Option<crate::io::ArtifactExternalChangeNotice>,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    match result {
        crate::io::EditResult::Applied { new_version } => {
            let mut payload = serde_json::json!({
                "applied": true,
                "new_version": new_version.value(),
            });
            add_managed_io_change_payload(&mut payload, change);
            add_managed_io_external_change_notice_payload(&mut payload, external_change_notice);
            crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload }
        }
        crate::io::EditResult::AppliedWithWarning {
            new_version,
            warning,
        } => {
            let mut payload = serde_json::json!({
                "applied": true,
                "new_version": new_version.value(),
                "warning": managed_io_warning_payload(warning),
            });
            add_managed_io_change_payload(&mut payload, change);
            add_managed_io_external_change_notice_payload(&mut payload, external_change_notice);
            crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload }
        }
        crate::io::EditResult::Rejected { reason } => {
            let mut payload = serde_json::json!({
                "applied": false,
                "reason": managed_io_error_payload(reason),
                "next_action": "Reread the artifact with arroba.read_artifact, reconcile with the current content, and retry through arroba.edit_artifact.",
            });
            add_managed_io_external_change_notice_payload(&mut payload, external_change_notice);
            crate::transport::runtime_tools::RuntimeToolResult { ok: false, payload }
        }
    }
}

pub(in crate::runtime::state) fn add_managed_io_external_change_notice_payload(
    payload: &mut serde_json::Value,
    notice: Option<crate::io::ArtifactExternalChangeNotice>,
) {
    add_managed_io_external_change_notices_payload(payload, notice.into_iter().collect());
}

pub(in crate::runtime::state) fn add_managed_io_external_change_notices_payload(
    payload: &mut serde_json::Value,
    notices: Vec<crate::io::ArtifactExternalChangeNotice>,
) {
    if notices.is_empty() {
        return;
    }
    let notices = notices
        .into_iter()
        .map(managed_io_external_change_notice_payload)
        .collect::<Vec<_>>();
    payload["external_changes"] = serde_json::json!(notices);
    if let Some(notice) = payload["external_changes"].get(0).cloned() {
        payload["external_change"] = notice;
    }
}

pub(in crate::runtime::state) fn managed_io_external_change_notice_payload(
    notice: crate::io::ArtifactExternalChangeNotice,
) -> serde_json::Value {
    serde_json::json!({
        "detected": true,
        "path": notice.path.to_string_lossy(),
        "message": notice.message,
        "next_action": "This artifact changed outside Arroba managed I/O after your last read. If the write was rejected, reread and reconcile before retrying; if it was applied with a rebase warning, verify the diff before continuing.",
    })
}

pub(in crate::runtime::state) fn managed_io_external_change_notice_for_path(
    path: PathBuf,
) -> crate::io::ArtifactExternalChangeNotice {
    crate::io::ArtifactExternalChangeNotice {
        path,
        message: "artifact changed outside Arroba managed I/O while the managed operation was being prepared".to_string(),
    }
}

pub(in crate::runtime::state) fn managed_io_result_applied(result: &crate::io::EditResult) -> bool {
    matches!(
        result,
        crate::io::EditResult::Applied { .. } | crate::io::EditResult::AppliedWithWarning { .. }
    )
}

pub(in crate::runtime::state) fn record_managed_io_external_change_if_rejected(
    monitor: &crate::io::ArtifactExternalChangeMonitor,
    workspace_identity: &crate::io::WorkspaceIdentity,
    path: &PathBuf,
    result: &crate::io::EditResult,
) {
    if matches!(
        result,
        crate::io::EditResult::Rejected {
            reason: crate::io::ArtifactEditError::ExternalChangeDuringApply { .. }
        }
    ) {
        monitor.record_external_change(workspace_identity, path);
    }
}

pub(in crate::runtime::state) fn record_managed_io_write_if_applied(
    monitor: &crate::io::ArtifactExternalChangeMonitor,
    provider_run_id: &str,
    workspace_identity: &crate::io::WorkspaceIdentity,
    workspace_root: &PathBuf,
    path: &PathBuf,
    result: &crate::io::EditResult,
) {
    if managed_io_result_applied(result) {
        monitor.observe_managed_write(provider_run_id, workspace_identity, workspace_root, path);
    }
}

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

pub(in crate::runtime::state) fn apply_managed_patch_operations(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: crate::io::WorkspaceIdentity,
    workspace_root: PathBuf,
    domain: crate::io::ArtifactDomainKind,
    operations: Vec<ManagedPatchOperation>,
    reservation_owner: crate::io::ArtifactReservationOwner,
    external_change_monitor: &crate::io::ArtifactExternalChangeMonitor,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let mut before_states: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut final_states: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut reservation_ranges: BTreeMap<PathBuf, Vec<crate::io::TextRange>> = BTreeMap::new();

    for operation in operations {
        match operation {
            ManagedPatchOperation::Add { path, content } => {
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_patch_state(
                    &workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_some() {
                    return Ok(managed_patch_rejected(
                        path,
                        "add file target already exists; reread and retry with an update",
                    ));
                }
                reservation_ranges
                    .entry(path.clone())
                    .or_default()
                    .push(crate::io::TextRange::new(0, usize::MAX));
                final_states.insert(path, Some(content));
            }
            ManagedPatchOperation::Update {
                path,
                old_text,
                new_text,
            } => {
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_patch_state(
                    &workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(current) = current else {
                    return Ok(managed_patch_rejected(
                        path,
                        "update file target does not exist",
                    ));
                };
                let Some((range, updated)) = replace_unique_text(&current, &old_text, &new_text)
                else {
                    return Ok(managed_patch_rejected(
                        path,
                        "patch old text was not found exactly once in the current artifact",
                    ));
                };
                reservation_ranges
                    .entry(path.clone())
                    .or_default()
                    .push(range);
                final_states.insert(path, Some(updated));
            }
            ManagedPatchOperation::Delete { path } => {
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_patch_state(
                    &workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_none() {
                    return Ok(managed_patch_rejected(
                        path,
                        "delete file target does not exist",
                    ));
                }
                reservation_ranges
                    .entry(path.clone())
                    .or_default()
                    .push(crate::io::TextRange::new(0, usize::MAX));
                final_states.insert(path, None);
            }
            ManagedPatchOperation::Move {
                from_path,
                to_path,
                old_text,
                new_text,
            } => {
                managed_io_validate_patch_path(&workspace_root, &from_path)?;
                managed_io_validate_patch_path(&workspace_root, &to_path)?;
                if from_path == to_path {
                    return Ok(managed_patch_rejected(
                        from_path,
                        "move source and target are identical",
                    ));
                }
                let source = managed_patch_state(
                    &workspace_root,
                    &from_path,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(mut source) = source else {
                    return Ok(managed_patch_rejected(
                        from_path,
                        "move source does not exist",
                    ));
                };
                let target = managed_patch_state(
                    &workspace_root,
                    &to_path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if target.is_some() {
                    return Ok(managed_patch_rejected(
                        to_path,
                        "move target already exists",
                    ));
                }
                if let (Some(old_text), Some(new_text)) = (old_text, new_text) {
                    let Some((_range, updated)) =
                        replace_unique_text(&source, &old_text, &new_text)
                    else {
                        return Ok(managed_patch_rejected(
                            from_path,
                            "move patch old text was not found exactly once in the current artifact",
                        ));
                    };
                    source = updated;
                }
                reservation_ranges
                    .entry(from_path.clone())
                    .or_default()
                    .push(crate::io::TextRange::new(0, usize::MAX));
                reservation_ranges
                    .entry(to_path.clone())
                    .or_default()
                    .push(crate::io::TextRange::new(0, usize::MAX));
                final_states.insert(from_path, None);
                final_states.insert(to_path, Some(source));
            }
        }
    }

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
