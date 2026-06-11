//! Workspace live sync workspace identity matching, rejection payloads, and git fingerprinting.

use super::*;

pub(in crate::runtime::state) fn add_workspace_live_sync_workspace_payload(
    payload: &mut serde_json::Value,
    workspace: &WorkspaceLiveSyncWorkspaceContext,
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

pub(in crate::runtime::state) fn workspace_live_sync_workspace_identity_rejected(
    workspace: &WorkspaceLiveSyncWorkspaceContext,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let mut payload = serde_json::json!({
        "applied": false,
        "reason": {
            "kind": "workspace_identity_changed",
            "message": "The provider run workspace identity changed since workspace live sync coordination started."
        },
        "next_action": "Stop editing, reread the workspace state, and only retry after Arroba revalidates or rejoins the coordinated workspace.",
    });
    add_workspace_live_sync_workspace_payload(&mut payload, workspace);
    crate::transport::runtime_tools::RuntimeToolResult { ok: false, payload }
}

pub(in crate::runtime::state) fn workspace_live_sync_workspace_identities_match(
    home: &crate::io::WorkspaceIdentity,
    worker: &crate::io::WorkspaceIdentity,
) -> bool {
    if let (Some(left), Some(right)) = (home.repo_id.as_deref(), worker.repo_id.as_deref()) {
        return !left.is_empty() && left == right && home.branch == worker.branch;
    }
    if let (Some(left), Some(right)) = (home.repo_url.as_deref(), worker.repo_url.as_deref()) {
        return normalize_workspace_live_sync_repo_url(left)
            == normalize_workspace_live_sync_repo_url(right)
            && home.branch == worker.branch;
    }
    home.worktree_root_fingerprint == worker.worktree_root_fingerprint
}

pub(in crate::runtime::state) fn workspace_live_sync_identity_for_session_workspace_link(
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

pub(in crate::runtime::state) fn normalize_workspace_live_sync_repo_url(value: &str) -> String {
    value.trim().trim_end_matches(".git").to_ascii_lowercase()
}

pub(in crate::runtime::state) fn workspace_live_sync_is_arroba_source_workspace(
    root: &PathBuf,
) -> bool {
    root.join("apps/kernel/Cargo.toml").is_file()
        && root
            .join(crate::provider::WORKSPACE_LIVE_SYNC_INSTRUCTIONS_SOURCE_PATH)
            .is_file()
}

pub(in crate::runtime::state) fn workspace_identity_for_root(
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

pub(in crate::runtime::state) async fn workspace_identity_for_root_off_thread(
    workspace_root: PathBuf,
) -> Result<crate::io::WorkspaceIdentity, DaemonError> {
    tokio::task::spawn_blocking(move || workspace_identity_for_root(&workspace_root))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "workspace_live_sync_workspace_identity",
            message: format!("workspace identity monitor task failed: {error}"),
        })
}

fn git_output(workspace_root: &PathBuf, args: &[&str]) -> Option<String> {
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

fn non_empty_owned(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_link_attachment_overrides_workspace_live_sync_coordination_identity() {
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

        let identity = workspace_live_sync_identity_for_session_workspace_link(
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

    #[test]
    fn workspace_link_identities_match_across_different_physical_worktrees() {
        let mut session = crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "/tmp/home-worktree",
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
            "/tmp/home-worktree",
            Some("main".to_string()),
            Some("home-fingerprint".to_string()),
        ));

        let home_identity = workspace_live_sync_identity_for_session_workspace_link(
            crate::io::WorkspaceIdentity {
                vcs_provider: Some("git".to_string()),
                repo_id: Some("home-physical".to_string()),
                repo_url: Some("https://example.test/home.git".to_string()),
                branch: Some("main".to_string()),
                head_commit: Some("abc123".to_string()),
                worktree_root_fingerprint: "home-fingerprint".to_string(),
            },
            &session,
            Path::new("/tmp/home-worktree"),
        );
        let worker_identity = crate::io::WorkspaceIdentity {
            vcs_provider: Some("git".to_string()),
            repo_id: Some("workspace_link:workspace-link-1".to_string()),
            repo_url: None,
            branch: None,
            head_commit: None,
            worktree_root_fingerprint: "worker-fingerprint".to_string(),
        };

        assert!(workspace_live_sync_workspace_identities_match(
            &home_identity,
            &worker_identity
        ));
    }

    #[test]
    fn workspace_link_identity_rewrites_worker_context_path_before_matching() {
        let mut session = crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "/tmp/home-worktree",
            "machine-home",
            "kernel-home",
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
            "machine-home",
            "kernel-home",
            "/tmp/home-worktree",
            Some("main".to_string()),
            Some("home-fingerprint".to_string()),
        ));
        link.attach(crate::session::WorkspaceLinkAttachment::new(
            "workspace-link-1",
            "local",
            "machine-worker",
            "kernel-worker",
            "/tmp/worker-worktree",
            Some("main".to_string()),
            Some("worker-fingerprint".to_string()),
        ));

        let home_identity = workspace_live_sync_identity_for_session_workspace_link(
            crate::io::WorkspaceIdentity {
                vcs_provider: Some("git".to_string()),
                repo_id: None,
                repo_url: None,
                branch: Some("main".to_string()),
                head_commit: Some("home-commit".to_string()),
                worktree_root_fingerprint: "home-fingerprint".to_string(),
            },
            &session,
            Path::new("/tmp/home-worktree"),
        );
        let worker_identity = workspace_live_sync_identity_for_session_workspace_link(
            crate::io::WorkspaceIdentity {
                vcs_provider: Some("git".to_string()),
                repo_id: None,
                repo_url: None,
                branch: Some("main".to_string()),
                head_commit: Some("worker-commit".to_string()),
                worktree_root_fingerprint: "worker-fingerprint".to_string(),
            },
            &session,
            Path::new("/tmp/worker-worktree"),
        );

        assert_eq!(
            worker_identity.repo_id.as_deref(),
            Some("workspace_link:workspace-link-1")
        );
        assert!(workspace_live_sync_workspace_identities_match(
            &home_identity,
            &worker_identity
        ));
    }
}
