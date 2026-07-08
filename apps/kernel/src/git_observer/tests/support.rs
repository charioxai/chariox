use std::collections::BTreeMap;
use std::process::Command;

use super::super::{GitTurnContext, GitTurnSnapshot, WorkspaceLiveSyncChange};

pub(super) fn test_context(root: &std::path::Path, prompt_id: &str) -> GitTurnContext {
    GitTurnContext {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider: "dev-stub".to_string(),
        model: "dev-git".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        provider_session_id: Some("provider-session-1".to_string()),
        prompt_id: prompt_id.to_string(),
        turn_id: prompt_id.to_string(),
        prompt_origin: Some(crate::session::PromptOrigin::Arroba),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        started_at_ms: None,
        worktree_path: root.to_path_buf(),
        workspace_live_sync_tracked: false,
        machine_id: None,
        prompt_summary: "make a searchable feature".to_string(),
    }
}

pub(super) fn workspace_live_sync_test_change(session_id: &str) -> WorkspaceLiveSyncChange {
    WorkspaceLiveSyncChange {
        session_id: session_id.to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/repo".to_string(),
        worktree_path: "/repo".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["src/lib.rs".to_string()],
        file_changes: Vec::new(),
        status_fingerprint: "managed_workspace_live_sync".to_string(),
    }
}

pub(super) fn tracked_snapshot(is_dirty: bool, status_fingerprint: &str) -> GitTurnSnapshot {
    GitTurnSnapshot {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider: "dev-stub".to_string(),
        model: "dev-git".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        provider_session_id: Some("provider-session-1".to_string()),
        prompt_id: "prompt-1".to_string(),
        turn_id: "prompt-1".to_string(),
        prompt_origin: Some(crate::session::PromptOrigin::Arroba),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        started_at_ms: None,
        machine_id: None,
        prompt_summary: "make a searchable feature".to_string(),
        repo_root: "/tmp/repo".to_string(),
        worktree_path: "/tmp/repo".to_string(),
        branch: Some("main".to_string()),
        head_sha: Some("abc123".to_string()),
        upstream_ref: None,
        ahead_count: None,
        status_fingerprint: status_fingerprint.to_string(),
        is_dirty,
        workspace_live_sync_tracked: true,
        workspace_live_sync_file_snapshots: BTreeMap::new(),
    }
}

pub(super) fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
