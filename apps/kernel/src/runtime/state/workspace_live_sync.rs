//! Workspace live sync runtime-state entry points.
//!
//! This root handles read/write/apply-patch command arguments, workspace identity checks,
//! external-change notices, and delegates diff/patch/payload/remote details to submodules.

use super::*;

pub(super) fn workspace_live_sync_read_payload(read: crate::io::ArtifactReadResult) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "artifact_id": read.artifact_id.as_str(),
        "path": read.path.to_string_lossy(),
        "domain": workspace_live_sync_domain_name(read.domain),
        "version": read.version.value(),
        "snapshot_id": read.snapshot_id.as_str(),
    });
    match read.content {
        crate::io::ArtifactContent::Text(text) => {
            payload["content_text"] = serde_json::Value::String(text);
        }
        crate::io::ArtifactContent::Bytes(bytes) => {
            payload["byte_count"] = serde_json::json!(bytes.len());
            payload["content_base64"] =
                serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(bytes));
        }
    }
    payload
}

mod remote_state;
pub(super) use remote_state::*;
mod remote_patch;
pub(super) use remote_patch::*;
mod remote_whole_file;
pub(super) use remote_whole_file::*;
mod remote;
pub(super) use remote::*;

pub(super) struct WorkspaceLiveSyncChangeContext {
    pub(super) path: PathBuf,
    pub(super) before: Option<WorkspaceLiveSyncTextSnapshot>,
    pub(super) after: Option<WorkspaceLiveSyncTextSnapshot>,
}

pub(super) struct WorkspaceLiveSyncTextSnapshot {
    pub(super) existed: bool,
    pub(super) text: String,
}

mod patch_parser;
pub(super) use patch_parser::*;
mod file_state;
pub(super) use file_state::*;
mod edit_result;
pub(super) use edit_result::*;
mod reservation;
pub(super) use reservation::*;
mod args;
pub(super) use args::*;
mod workspace_identity;
pub(super) use workspace_identity::*;
mod patch_plan;
pub(super) use patch_plan::*;
mod patch;
pub(super) use patch::*;
mod whole_file;
pub(super) use whole_file::*;

pub(super) fn add_workspace_live_sync_change_payload(
    payload: &mut serde_json::Value,
    change: WorkspaceLiveSyncChangeContext,
) {
    if change.before.is_none() && change.after.is_none() {
        return;
    }
    let before = change.before.unwrap_or(WorkspaceLiveSyncTextSnapshot {
        existed: false,
        text: String::new(),
    });
    let after = change.after.unwrap_or(WorkspaceLiveSyncTextSnapshot {
        existed: false,
        text: String::new(),
    });
    let diff = workspace_live_sync_unified_diff(&change.path, &before, &after);
    payload["path"] = serde_json::Value::String(change.path.to_string_lossy().to_string());
    payload["change"] = serde_json::json!({
        "path": change.path.to_string_lossy(),
        "kind": if !before.existed {
            "add"
        } else if !after.existed {
            "delete"
        } else {
            "update"
        },
        "diff": diff.text,
        "diff_truncated": diff.truncated,
    });
}

pub(super) fn add_workspace_live_sync_whole_file_change_payload(
    payload: &mut serde_json::Value,
    path: PathBuf,
    before: Option<crate::io::ArtifactContent>,
    after: Option<crate::io::ArtifactContent>,
) {
    if before.is_none() && after.is_none() {
        return;
    }
    let before_existed = before.is_some();
    let after_existed = after.is_some();
    if let (
        Some(crate::io::ArtifactContent::Text(before)),
        Some(crate::io::ArtifactContent::Text(after)),
    ) = (&before, &after)
    {
        add_workspace_live_sync_change_payload(
            payload,
            WorkspaceLiveSyncChangeContext {
                path,
                before: Some(WorkspaceLiveSyncTextSnapshot {
                    existed: true,
                    text: before.clone(),
                }),
                after: Some(WorkspaceLiveSyncTextSnapshot {
                    existed: true,
                    text: after.clone(),
                }),
            },
        );
        return;
    }
    let normalized_path = path.to_string_lossy().to_string();
    let before_bytes = before
        .as_ref()
        .map(artifact_content_byte_count)
        .unwrap_or(0);
    let after_bytes = after.as_ref().map(artifact_content_byte_count).unwrap_or(0);
    payload["path"] = serde_json::Value::String(normalized_path.clone());
    payload["change"] = serde_json::json!({
        "path": normalized_path,
        "kind": if !before_existed {
            "add"
        } else if !after_existed {
            "delete"
        } else {
            "update"
        },
        "binary": true,
        "before_byte_count": before_bytes,
        "after_byte_count": after_bytes,
        "diff": "Binary files differ",
        "diff_truncated": false,
    });
}

mod diff;
pub(super) use diff::workspace_live_sync_text_for_diff;
use diff::{artifact_content_byte_count, workspace_live_sync_diff_workspace_path, workspace_live_sync_unified_diff};

mod payload;
pub(super) use payload::workspace_live_sync_daemon_error;
use payload::{workspace_live_sync_domain_name, workspace_live_sync_error_payload, workspace_live_sync_warning_payload};
