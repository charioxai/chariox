//! Workspace live sync edit result and external-change payloads.

use super::*;

pub(in crate::runtime::state) fn workspace_live_sync_edit_result(
    result: crate::io::EditResult,
    change: WorkspaceLiveSyncChangeContext,
    external_change_notice: Option<crate::io::ArtifactExternalChangeNotice>,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    match result {
        crate::io::EditResult::Applied { new_version } => {
            let mut payload = serde_json::json!({
                "applied": true,
                "new_version": new_version.value(),
            });
            add_workspace_live_sync_change_payload(&mut payload, change);
            add_workspace_live_sync_external_change_notice_payload(
                &mut payload,
                external_change_notice,
            );
            crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload }
        }
        crate::io::EditResult::AppliedWithWarning {
            new_version,
            warning,
        } => {
            let mut payload = serde_json::json!({
                "applied": true,
                "new_version": new_version.value(),
                "warning": workspace_live_sync_warning_payload(warning),
            });
            add_workspace_live_sync_change_payload(&mut payload, change);
            add_workspace_live_sync_external_change_notice_payload(
                &mut payload,
                external_change_notice,
            );
            crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload }
        }
        crate::io::EditResult::Rejected { reason } => {
            let mut payload = serde_json::json!({
                "applied": false,
                "reason": workspace_live_sync_error_payload(reason),
                "next_action": "Reread the artifact with arroba.read_artifact, reconcile with the current content, and retry through arroba.edit_artifact.",
            });
            add_workspace_live_sync_external_change_notice_payload(
                &mut payload,
                external_change_notice,
            );
            crate::transport::runtime_tools::RuntimeToolResult { ok: false, payload }
        }
    }
}

pub(in crate::runtime::state) fn add_workspace_live_sync_external_change_notice_payload(
    payload: &mut serde_json::Value,
    notice: Option<crate::io::ArtifactExternalChangeNotice>,
) {
    add_workspace_live_sync_external_change_notices_payload(payload, notice.into_iter().collect());
}

pub(in crate::runtime::state) fn add_workspace_live_sync_external_change_notices_payload(
    payload: &mut serde_json::Value,
    notices: Vec<crate::io::ArtifactExternalChangeNotice>,
) {
    if notices.is_empty() {
        return;
    }
    let notices = notices
        .into_iter()
        .map(workspace_live_sync_external_change_notice_payload)
        .collect::<Vec<_>>();
    payload["external_changes"] = serde_json::json!(notices);
    if let Some(notice) = payload["external_changes"].get(0).cloned() {
        payload["external_change"] = notice;
    }
}

fn workspace_live_sync_external_change_notice_payload(
    notice: crate::io::ArtifactExternalChangeNotice,
) -> serde_json::Value {
    serde_json::json!({
        "detected": true,
        "path": notice.path.to_string_lossy(),
        "message": notice.message,
        "next_action": "This artifact changed outside Arroba workspace live sync after your last read. If the write was rejected, reread and reconcile before retrying; if it was applied with a rebase warning, verify the diff before continuing.",
    })
}

pub(in crate::runtime::state) fn workspace_live_sync_external_change_notice_for_path(
    path: PathBuf,
) -> crate::io::ArtifactExternalChangeNotice {
    crate::io::ArtifactExternalChangeNotice {
        path,
        message: "artifact changed outside Arroba workspace live sync while the managed operation was being prepared".to_string(),
    }
}

pub(in crate::runtime::state) fn workspace_live_sync_result_applied(
    result: &crate::io::EditResult,
) -> bool {
    matches!(
        result,
        crate::io::EditResult::Applied { .. } | crate::io::EditResult::AppliedWithWarning { .. }
    )
}

pub(in crate::runtime::state) fn record_workspace_live_sync_external_change_if_rejected(
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

pub(in crate::runtime::state) fn record_workspace_live_sync_write_if_applied(
    monitor: &crate::io::ArtifactExternalChangeMonitor,
    provider_run_id: &str,
    workspace_identity: &crate::io::WorkspaceIdentity,
    workspace_root: &PathBuf,
    path: &PathBuf,
    result: &crate::io::EditResult,
) {
    if workspace_live_sync_result_applied(result) {
        monitor.observe_managed_write(provider_run_id, workspace_identity, workspace_root, path);
    }
}
