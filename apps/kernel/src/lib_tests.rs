use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use super::agent::{CreateAgentRequest, GitWorktreePlacement};
use super::app::RemoteLeaseRuntime;
use super::attachment::{AttachRequest, ClientCapabilityLevel};
use super::provider::{LaunchProviderRequest, ProviderResumeState};
use super::session::{CreateSessionRequest, PromptStatus, PromptSubmissionOutcome, SessionStatus};
use super::terminal::TerminalOutputKind;
use super::transport::relay_peer::{
    RelayPeerEvent, RelayPeerRequest, RelayPeerResponse, RelayProjectedCompletion,
    RelayProjectedOutputChunk, RemoteWorkspaceLiveSyncApplyContext,
    RemoteWorkspaceLiveSyncArtifactState, RemoteWorkspaceLiveSyncContext,
};
use super::{DaemonApp, DaemonConfig, DaemonError};
use sha2::{Digest, Sha256};

static CURRENT_DIR_LOCK: Mutex<()> = Mutex::new(());

fn run_test_git(cwd: &std::path::Path, args: &[&str]) {
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

fn wait_for_terminal_output(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> Vec<super::terminal::TerminalOutputRecord> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);

    loop {
        let records = crate::app::provider_output::pump_terminal_output_for_attachment(
            app,
            session_id,
            attachment_id,
        )
        .expect("terminal output should fan out");
        if !records.is_empty() {
            return records;
        }

        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for terminal output"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

mod app_lifecycle;
mod architecture_boundaries;
mod capability_boundaries;
mod client_protocol_conformance;
mod performance_drills;
mod provider_sessions;
mod remote_leases;

#[test]
fn relay_peer_workspace_live_sync_apply_shape_is_versioned() {
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 62);

    let context = RemoteWorkspaceLiveSyncApplyContext {
        home_session_id: "session-1".to_string(),
        link_id: "workspace-link-1".to_string(),
        link_name: "shared".to_string(),
        source_agent_id: "agent-1".to_string(),
        source_worktree_path: "/source".to_string(),
        target_user_id: "user-2".to_string(),
        target_machine_id: "machine-2".to_string(),
        target_kernel_id: "kernel-2".to_string(),
        target_repo_root: "/target".to_string(),
    };
    let change = crate::git_observer::WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/source".to_string(),
        worktree_path: "/source".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["src/lib.rs".to_string()],
        file_changes: vec![crate::git_observer::WorkspaceLiveSyncFileChange {
            path: "src/lib.rs".to_string(),
            previous_path: None,
            kind: crate::git_observer::WorkspaceLiveSyncFileChangeKind::Modified,
            before_content_base64: Some("YmVmb3JlCg==".to_string()),
            after_content_base64: Some("YWZ0ZXIK".to_string()),
            binary: false,
        }],
        status_fingerprint: "fingerprint-1".to_string(),
    };
    let request = RelayPeerRequest::ApplyWorkspaceLiveSyncChange {
        context: context.clone(),
        change,
    };
    let response = RelayPeerResponse::WorkspaceLiveSyncChangeApplied {
        target_result: crate::git_observer::WorkspaceLiveSyncTargetResult {
            session_id: context.home_session_id,
            link_id: context.link_id,
            link_name: context.link_name,
            source_agent_id: context.source_agent_id,
            source_worktree_path: context.source_worktree_path,
            target_user_id: context.target_user_id,
            target_machine_id: context.target_machine_id,
            target_kernel_id: context.target_kernel_id,
            target_repo_root: context.target_repo_root,
            path_results: vec![crate::git_observer::WorkspaceLiveSyncPathApplyResult {
                path: "src/lib.rs".to_string(),
                status: crate::git_observer::WorkspaceLiveSyncApplyStatus::Rebased,
                message: "rebased over non-overlapping target change".to_string(),
            }],
        },
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!("apply_workspace_live_sync_change"))
    );
    assert_eq!(
        snapshot.pointer("/1/kind"),
        Some(&serde_json::json!("workspace_live_sync_change_applied"))
    );
    assert_eq!(
        snapshot.pointer("/1/target_result/path_results/0/status"),
        Some(&serde_json::json!("rebased"))
    );
    let serialized =
        serde_json::to_string(&snapshot).expect("workspace live sync relay apply should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "dd483fae2ed150ca874cd7594ec682e869a5dfb2aa1d73755369bb10c3ce7e8f"
    );
}

#[test]
fn relay_peer_workspace_live_sync_runtime_tool_shape_is_versioned() {
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 62);

    let request = RelayPeerRequest::ForwardWorkspaceLiveSyncRuntimeTool {
        context: RemoteWorkspaceLiveSyncContext {
            home_kernel_id: "kernel-home".to_string(),
            home_session_id: "session-1".to_string(),
            home_agent_id: "agent-1".to_string(),
            leased_agent_id: "leased-agent-1".to_string(),
            worker_provider_run_id: "provider-run-worker".to_string(),
            worker_workspace_identity: crate::io::WorkspaceIdentity {
                vcs_provider: Some("git".to_string()),
                repo_id: None,
                repo_url: Some("https://example.test/repo.git".to_string()),
                branch: Some("main".to_string()),
                head_commit: Some("commit-1".to_string()),
                worktree_root_fingerprint: "fingerprint-1".to_string(),
            },
        },
        tool_name: "arroba.write_artifact".to_string(),
        arguments: serde_json::json!({
            "path": "src/lib.rs",
            "content_text": "after\n",
            "domain": "text"
        }),
        artifact_states: vec![RemoteWorkspaceLiveSyncArtifactState {
            path: "src/lib.rs".to_string(),
            exists: true,
            domain: Some("text".to_string()),
            content_text: Some("before\n".to_string()),
            content_base64: None,
        }],
    };
    let response = RelayPeerResponse::WorkspaceLiveSyncRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "applied": true,
                "path": "src/lib.rs"
            }),
        },
        final_artifact_states: vec![RemoteWorkspaceLiveSyncArtifactState {
            path: "src/lib.rs".to_string(),
            exists: true,
            domain: Some("text".to_string()),
            content_text: Some("after\n".to_string()),
            content_base64: None,
        }],
    };

    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/kind"),
        Some(&serde_json::json!(
            "forward_workspace_live_sync_runtime_tool"
        ))
    );
    assert_eq!(
        snapshot.pointer("/0/context/home_kernel_id"),
        Some(&serde_json::json!("kernel-home"))
    );
    assert_eq!(
        snapshot.pointer("/0/artifact_states/0/domain"),
        Some(&serde_json::json!("text"))
    );
    assert_eq!(
        snapshot.pointer("/1/kind"),
        Some(&serde_json::json!(
            "workspace_live_sync_runtime_tool_handled"
        ))
    );
    assert_eq!(
        snapshot.pointer("/1/final_artifact_states/0/content_text"),
        Some(&serde_json::json!("after\n"))
    );
    let serialized = serde_json::to_string(&snapshot)
        .expect("workspace live sync relay runtime tool should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "8ad2180809c7c2fce18ed7e36de472227c56baef7dd950a12f8d643824475534"
    );
}
