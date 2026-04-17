//! Managed-I/O apply-patch orchestration.
//!
//! This module validates patch requests against tracked workspace state, applies accepted edits,
//! emits file-change output records, and records external-change conflicts for agent notification.

use super::*;

#[derive(Debug, Clone)]
pub(in crate::runtime::state) enum ManagedPatchOperation {
    Add {
        path: PathBuf,
        content: String,
    },
    Update {
        path: PathBuf,
        old_text: String,
        new_text: String,
    },
    Delete {
        path: PathBuf,
    },
    Move {
        from_path: PathBuf,
        to_path: PathBuf,
        old_text: Option<String>,
        new_text: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub(in crate::runtime::state) enum ManagedWholeFileOperation {
    Delete {
        path: PathBuf,
    },
    Move {
        from_path: PathBuf,
        to_path: PathBuf,
    },
}

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

pub(in crate::runtime::state) fn parse_managed_apply_patch(
    patch_text: &str,
) -> Result<Vec<ManagedPatchOperation>, DaemonError> {
    let lines = patch_text.lines().collect::<Vec<_>>();
    if lines.first().map(|line| line.trim()) != Some("*** Begin Patch")
        || lines.last().map(|line| line.trim()) != Some("*** End Patch")
    {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "patch_text must use the apply_patch envelope".to_string(),
        });
    }
    let mut operations = Vec::new();
    let mut index = 1usize;
    while index + 1 < lines.len() {
        let line = lines[index];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            index += 1;
            let mut body = Vec::new();
            while index < lines.len() && !lines[index].starts_with("*** ") {
                let line = lines[index];
                body.push(line.strip_prefix('+').unwrap_or(line).to_string());
                index += 1;
            }
            operations.push(ManagedPatchOperation::Add {
                path: PathBuf::from(path.trim()),
                content: join_patch_lines(&body),
            });
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            operations.push(ManagedPatchOperation::Delete {
                path: PathBuf::from(path.trim()),
            });
            index += 1;
            continue;
        }
        if line.starts_with("*** Move to: ") {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_apply_patch",
                message: "move hunks must follow an update file header".to_string(),
            });
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            index += 1;
            let mut move_to = None;
            if index < lines.len() {
                if let Some(target) = lines[index].strip_prefix("*** Move to: ") {
                    move_to = Some(PathBuf::from(target.trim()));
                    index += 1;
                }
            }
            let mut old_lines = Vec::new();
            let mut new_lines = Vec::new();
            while index < lines.len() && !lines[index].starts_with("*** ") {
                let line = lines[index];
                if line.starts_with("@@") || line.starts_with('\\') {
                    index += 1;
                    continue;
                }
                if let Some(rest) = line.strip_prefix('-') {
                    old_lines.push(rest.to_string());
                } else if let Some(rest) = line.strip_prefix('+') {
                    new_lines.push(rest.to_string());
                } else {
                    let rest = line.strip_prefix(' ').unwrap_or(line);
                    old_lines.push(rest.to_string());
                    new_lines.push(rest.to_string());
                }
                index += 1;
            }
            match move_to {
                Some(to_path) => operations.push(ManagedPatchOperation::Move {
                    from_path: PathBuf::from(path.trim()),
                    to_path,
                    old_text: (!old_lines.is_empty()).then(|| join_patch_lines(&old_lines)),
                    new_text: (!new_lines.is_empty()).then(|| join_patch_lines(&new_lines)),
                }),
                None => {
                    if old_lines.is_empty() {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_apply_patch",
                            message: format!("update hunk for `{}` has no old text", path.trim()),
                        });
                    }
                    operations.push(ManagedPatchOperation::Update {
                        path: PathBuf::from(path.trim()),
                        old_text: join_patch_lines(&old_lines),
                        new_text: join_patch_lines(&new_lines),
                    });
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!("unsupported patch line `{line}`"),
        });
    }
    if operations.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "patch_text did not contain any supported file operations".to_string(),
        });
    }
    Ok(operations)
}

pub(in crate::runtime::state) fn join_patch_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
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

pub(in crate::runtime::state) fn apply_managed_whole_file_operations(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: crate::io::WorkspaceIdentity,
    workspace_root: PathBuf,
    domain: crate::io::ArtifactDomainKind,
    operations: Vec<ManagedWholeFileOperation>,
    reservation_owner: crate::io::ArtifactReservationOwner,
    external_change_monitor: &crate::io::ArtifactExternalChangeMonitor,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let mut before_states: BTreeMap<PathBuf, Option<crate::io::ArtifactContent>> = BTreeMap::new();
    let mut final_states: BTreeMap<PathBuf, Option<crate::io::ArtifactContent>> = BTreeMap::new();
    let mut reservation_ranges: BTreeMap<PathBuf, Vec<crate::io::TextRange>> = BTreeMap::new();

    for operation in operations {
        match operation {
            ManagedWholeFileOperation::Delete { path } => {
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_whole_file_state(
                    &workspace_root,
                    &path,
                    domain,
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
            ManagedWholeFileOperation::Move { from_path, to_path } => {
                managed_io_validate_patch_path(&workspace_root, &from_path)?;
                managed_io_validate_patch_path(&workspace_root, &to_path)?;
                if from_path == to_path {
                    return Ok(managed_patch_rejected(
                        from_path,
                        "move source and target are identical",
                    ));
                }
                let source = managed_whole_file_state(
                    &workspace_root,
                    &from_path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(source) = source else {
                    return Ok(managed_patch_rejected(
                        from_path,
                        "move source does not exist",
                    ));
                };
                let target = managed_whole_file_state(
                    &workspace_root,
                    &to_path,
                    domain,
                    &mut before_states,
                    &mut final_states,
                )?;
                if target.is_some() {
                    return Ok(managed_patch_rejected(
                        to_path,
                        "move target already exists",
                    ));
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
        let latest = match managed_io_read_optional_content(&workspace_root, path, domain) {
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
                "artifact changed while the managed whole-file operation was being prepared; reread and retry",
            );
            add_managed_io_external_change_notices_payload(&mut output.payload, notices);
            return Ok(output);
        }
    }

    if let Err(error) = managed_io_write_final_content_states(&workspace_root, &final_states) {
        let _ = managed_io_write_final_content_states(&workspace_root, &before_states);
        for token in reservations {
            coordinator.release_reservation(token);
        }
        return Err(error);
    }

    for (path, after) in &final_states {
        match after {
            Some(content) => {
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: content.clone(),
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
        let before = before_states.get(&path).cloned().flatten();
        let mut change_payload = serde_json::json!({});
        add_managed_io_whole_file_change_payload(&mut change_payload, path, before, after);
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

pub(in crate::runtime::state) fn managed_patch_state(
    workspace_root: &PathBuf,
    path: &PathBuf,
    before_states: &mut BTreeMap<PathBuf, Option<String>>,
    final_states: &mut BTreeMap<PathBuf, Option<String>>,
) -> Result<Option<String>, DaemonError> {
    if let Some(current) = final_states.get(path) {
        return Ok(current.clone());
    }
    let current = managed_io_read_optional_text(workspace_root, path)?;
    before_states
        .entry(path.clone())
        .or_insert_with(|| current.clone());
    final_states.insert(path.clone(), current.clone());
    Ok(current)
}

pub(in crate::runtime::state) fn managed_whole_file_state(
    workspace_root: &PathBuf,
    path: &PathBuf,
    domain: crate::io::ArtifactDomainKind,
    before_states: &mut BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
    final_states: &mut BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
) -> Result<Option<crate::io::ArtifactContent>, DaemonError> {
    if let Some(current) = final_states.get(path) {
        return Ok(current.clone());
    }
    let current = managed_io_read_optional_content(workspace_root, path, domain)?;
    before_states
        .entry(path.clone())
        .or_insert_with(|| current.clone());
    final_states.insert(path.clone(), current.clone());
    Ok(current)
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

pub(in crate::runtime::state) fn managed_io_validate_patch_path(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Result<(), DaemonError> {
    let _ = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| DaemonError::LocalTransport {
        operation: "runtime_tool_apply_patch",
        message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
    })?;
    if path == std::path::Path::new(crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH)
        && managed_io_is_arroba_source_workspace(workspace_root)
    {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!(
                "the Arroba managed-I/O instruction policy `{}` is owned by Arroba and cannot be edited through managed artifact I/O",
                crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH
            ),
        });
    }
    Ok(())
}

pub(in crate::runtime::state) fn managed_io_read_optional_text(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Result<Option<String>, DaemonError> {
    let full_path = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
        }
    })?;
    match std::fs::read_to_string(&full_path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!("failed to read `{}`: {error}", path.to_string_lossy()),
        }),
    }
}

pub(in crate::runtime::state) fn managed_io_read_optional_content(
    workspace_root: &PathBuf,
    path: &PathBuf,
    domain: crate::io::ArtifactDomainKind,
) -> Result<Option<crate::io::ArtifactContent>, DaemonError> {
    let full_path = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "runtime_tool_managed_io_state",
            message:
                "managed paths must be workspace-relative and cannot escape the workspace root"
                    .to_string(),
        }
    })?;
    match std::fs::read(&full_path) {
        Ok(bytes) => match domain {
            crate::io::ArtifactDomainKind::TextDocument
            | crate::io::ArtifactDomainKind::StructuredDocument => {
                let text =
                    String::from_utf8(bytes).map_err(|error| DaemonError::LocalTransport {
                        operation: "runtime_tool_managed_io_state",
                        message: format!(
                            "failed to decode `{}` as UTF-8: {error}",
                            path.to_string_lossy()
                        ),
                    })?;
                Ok(Some(crate::io::ArtifactContent::Text(text)))
            }
            crate::io::ArtifactDomainKind::OpaqueBlob => {
                Ok(Some(crate::io::ArtifactContent::Bytes(bytes)))
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_managed_io_state",
            message: format!("failed to read `{}`: {error}", path.to_string_lossy()),
        }),
    }
}

pub(in crate::runtime::state) fn managed_io_write_final_states(
    workspace_root: &PathBuf,
    states: &BTreeMap<PathBuf, Option<String>>,
) -> Result<(), DaemonError> {
    for (path, text) in states {
        let full_path = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_apply_patch",
                message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
            }
        })?;
        match text {
            Some(text) => {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_apply_patch",
                            message: format!(
                                "failed to create `{}`: {error}",
                                parent.to_string_lossy()
                            ),
                        }
                    })?;
                }
                std::fs::write(&full_path, text).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_apply_patch",
                    message: format!("failed to write `{}`: {error}", path.to_string_lossy()),
                })?;
            }
            None => match std::fs::remove_file(&full_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_apply_patch",
                        message: format!("failed to delete `{}`: {error}", path.to_string_lossy()),
                    });
                }
            },
        }
    }
    Ok(())
}

pub(in crate::runtime::state) fn managed_io_write_final_content_states(
    workspace_root: &PathBuf,
    states: &BTreeMap<PathBuf, Option<crate::io::ArtifactContent>>,
) -> Result<(), DaemonError> {
    for (path, content) in states {
        let full_path = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_managed_io_state",
                message:
                    "managed paths must be workspace-relative and cannot escape the workspace root"
                        .to_string(),
            }
        })?;
        match content {
            Some(crate::io::ArtifactContent::Text(text)) => {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_managed_io_state",
                            message: format!(
                                "failed to create `{}`: {error}",
                                parent.to_string_lossy()
                            ),
                        }
                    })?;
                }
                std::fs::write(&full_path, text).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_managed_io_state",
                    message: format!("failed to write `{}`: {error}", path.to_string_lossy()),
                })?;
            }
            Some(crate::io::ArtifactContent::Bytes(bytes)) => {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_managed_io_state",
                            message: format!(
                                "failed to create `{}`: {error}",
                                parent.to_string_lossy()
                            ),
                        }
                    })?;
                }
                std::fs::write(&full_path, bytes).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_managed_io_state",
                    message: format!("failed to write `{}`: {error}", path.to_string_lossy()),
                })?;
            }
            None => match std::fs::remove_file(&full_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_managed_io_state",
                        message: format!("failed to delete `{}`: {error}", path.to_string_lossy()),
                    });
                }
            },
        }
    }
    Ok(())
}
