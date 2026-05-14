//! Managed-I/O runtime-state entry points.
//!
//! This root handles read/write/apply-patch command arguments, workspace identity checks,
//! external-change notices, and delegates diff/patch/payload/remote details to submodules.

use super::*;

pub(super) fn managed_io_read_payload(read: crate::io::ArtifactReadResult) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "artifact_id": read.artifact_id.as_str(),
        "path": read.path.to_string_lossy(),
        "domain": managed_io_domain_name(read.domain),
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

pub(super) fn add_managed_io_workspace_payload(
    payload: &mut serde_json::Value,
    workspace: &ManagedIoWorkspaceContext,
) {
    payload["workspace"] = serde_json::json!({
        "identity_changed": workspace.identity_changed,
        "identity_valid": workspace.valid,
        "identity_generation": workspace.generation,
        "vcs_provider": workspace.identity.vcs_provider.clone(),
        "repo_id": workspace.identity.repo_id.clone(),
        "repo_url": workspace.identity.repo_url.clone(),
        "branch": workspace.identity.branch.clone(),
        "head_commit": workspace.identity.head_commit.clone(),
        "worktree_root_fingerprint": workspace.identity.worktree_root_fingerprint.clone(),
    });
}

pub(super) fn managed_io_workspace_identity_rejected(
    workspace: &ManagedIoWorkspaceContext,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let mut payload = serde_json::json!({
        "applied": false,
        "reason": {
            "kind": "workspace_identity_changed",
            "message": "The provider run workspace identity changed since managed I/O coordination started."
        },
        "next_action": "Stop editing, reread the workspace state, and only retry after Arroba revalidates or rejoins the coordinated workspace.",
    });
    add_managed_io_workspace_payload(&mut payload, workspace);
    crate::transport::runtime_tools::RuntimeToolResult { ok: false, payload }
}

pub(super) fn managed_io_workspace_identities_match(
    home: &crate::io::WorkspaceIdentity,
    worker: &crate::io::WorkspaceIdentity,
) -> bool {
    if let (Some(left), Some(right)) = (home.repo_id.as_deref(), worker.repo_id.as_deref()) {
        return !left.is_empty() && left == right && home.branch == worker.branch;
    }
    if let (Some(left), Some(right)) = (home.repo_url.as_deref(), worker.repo_url.as_deref()) {
        return normalize_managed_io_repo_url(left) == normalize_managed_io_repo_url(right)
            && home.branch == worker.branch;
    }
    home.worktree_root_fingerprint == worker.worktree_root_fingerprint
}

pub(super) fn managed_io_identity_for_session_workspace_link(
    mut identity: crate::io::WorkspaceIdentity,
    session: &crate::session::RuntimeSession,
    workspace_root: &Path,
) -> crate::io::WorkspaceIdentity {
    let Some(link) = session.workspace_link_for_repo_root(workspace_root) else {
        return identity;
    };
    identity.repo_id = Some(format!("workspace_link:{}", link.link_id()));
    identity.repo_url = None;
    identity.branch = None;
    identity.head_commit = None;
    identity
}

pub(super) fn normalize_managed_io_repo_url(value: &str) -> String {
    value.trim().trim_end_matches(".git").to_ascii_lowercase()
}

mod remote_state;
pub(super) use remote_state::*;
mod remote_patch;
pub(super) use remote_patch::*;
mod remote_whole_file;
pub(super) use remote_whole_file::*;
mod remote;
pub(super) use remote::*;

pub(super) struct ManagedIoChangeContext {
    pub(super) path: PathBuf,
    pub(super) before: Option<ManagedIoTextSnapshot>,
    pub(super) after: Option<ManagedIoTextSnapshot>,
}

pub(super) struct ManagedIoTextSnapshot {
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
mod patch_plan;
pub(super) use patch_plan::*;
mod patch;
pub(super) use patch::*;
mod whole_file;
pub(super) use whole_file::*;

pub(super) fn managed_io_is_arroba_source_workspace(root: &PathBuf) -> bool {
    root.join("apps/kernel/Cargo.toml").is_file()
        && root
            .join(crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH)
            .is_file()
}

pub(super) fn add_managed_io_change_payload(
    payload: &mut serde_json::Value,
    change: ManagedIoChangeContext,
) {
    if change.before.is_none() && change.after.is_none() {
        return;
    }
    let before = change.before.unwrap_or(ManagedIoTextSnapshot {
        existed: false,
        text: String::new(),
    });
    let after = change.after.unwrap_or(ManagedIoTextSnapshot {
        existed: false,
        text: String::new(),
    });
    let diff = managed_io_unified_diff(&change.path, &before, &after);
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

pub(super) fn add_managed_io_whole_file_change_payload(
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
        add_managed_io_change_payload(
            payload,
            ManagedIoChangeContext {
                path,
                before: Some(ManagedIoTextSnapshot {
                    existed: true,
                    text: before.clone(),
                }),
                after: Some(ManagedIoTextSnapshot {
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
pub(super) use diff::managed_io_text_for_diff;
use diff::{artifact_content_byte_count, managed_io_diff_workspace_path, managed_io_unified_diff};

pub(super) fn workspace_identity_for_root(
    workspace_root: &PathBuf,
) -> crate::io::WorkspaceIdentity {
    let fingerprint = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.clone())
        .to_string_lossy()
        .to_string();
    let git_root = git_output(workspace_root, &["rev-parse", "--show-toplevel"]);
    let Some(git_root) = git_root else {
        return crate::io::WorkspaceIdentity::local(fingerprint);
    };
    let normalized_git_root = PathBuf::from(git_root.trim())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(git_root.trim()))
        .to_string_lossy()
        .to_string();
    crate::io::WorkspaceIdentity {
        vcs_provider: Some("git".to_string()),
        repo_id: None,
        repo_url: git_output(workspace_root, &["config", "--get", "remote.origin.url"])
            .and_then(|value| non_empty_owned(value.trim())),
        branch: git_output(workspace_root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .and_then(|value| non_empty_owned(value.trim()))
            .map(|branch| {
                if branch == "HEAD" {
                    "detached".to_string()
                } else {
                    branch
                }
            }),
        head_commit: git_output(workspace_root, &["rev-parse", "HEAD"])
            .and_then(|value| non_empty_owned(value.trim())),
        worktree_root_fingerprint: normalized_git_root,
    }
}

pub(super) async fn workspace_identity_for_root_off_thread(
    workspace_root: PathBuf,
) -> Result<crate::io::WorkspaceIdentity, DaemonError> {
    tokio::task::spawn_blocking(move || workspace_identity_for_root(&workspace_root))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "managed_io_workspace_identity",
            message: format!("workspace identity monitor task failed: {error}"),
        })
}

pub(super) fn git_output(workspace_root: &PathBuf, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn non_empty_owned(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_link_attachment_overrides_managed_io_coordination_identity() {
        let mut session = crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "/tmp/worktree-a",
            "machine-1",
            "kernel-1",
        );
        let link = crate::session::WorkspaceLinkDefinition::new(
            "workspace-link-1",
            "session-1",
            "shared",
            "local",
        );
        session.create_workspace_link(link);
        let link = session
            .workspace_link_mut("workspace-link-1")
            .expect("link should exist");
        link.attach(crate::session::WorkspaceLinkAttachment::new(
            "workspace-link-1",
            "local",
            "machine-1",
            "kernel-1",
            "/tmp/worktree-a",
            Some("main".to_string()),
            None,
        ));

        let identity = crate::io::WorkspaceIdentity {
            vcs_provider: Some("git".to_string()),
            repo_id: Some("physical-repo".to_string()),
            repo_url: Some("https://example.test/repo.git".to_string()),
            branch: Some("feature".to_string()),
            head_commit: Some("abc123".to_string()),
            worktree_root_fingerprint: "fingerprint-a".to_string(),
        };

        let identity = managed_io_identity_for_session_workspace_link(
            identity,
            &session,
            Path::new("/tmp/worktree-a"),
        );

        assert_eq!(
            identity.repo_id.as_deref(),
            Some("workspace_link:workspace-link-1")
        );
        assert_eq!(identity.repo_url, None);
        assert_eq!(identity.branch, None);
        assert_eq!(identity.head_commit, None);
        assert_eq!(identity.worktree_root_fingerprint, "fingerprint-a");
    }
}

mod payload;
pub(super) use payload::managed_io_daemon_error;
use payload::{managed_io_domain_name, managed_io_error_payload, managed_io_warning_payload};
