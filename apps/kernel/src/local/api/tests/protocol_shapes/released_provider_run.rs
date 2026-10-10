//! MP-11: immutable released provider-run wire snapshots, retained across 447.
//! These synthetic test-only DTOs are not used by any public transport.
use super::*;
use sha2::{Digest, Sha256};
const RELEASED_PROTOCOL_VERSION: u32 = 416;
#[derive(serde::Serialize)]
enum ReleasedProviderRunResponse {
    ProviderRun {
        provider_run: crate::provider::RuntimeProviderRun,
    },
    AgentForked {
        source_agent_id: String,
        agent: Box<crate::agent::AgentInstance>,
        provider_run: crate::provider::RuntimeProviderRun,
        session: Box<crate::session::RuntimeSession>,
    },
}

#[test]
fn mp11_released416_turn_undo_and_agent_fork_shape_is_versioned() {
    assert_eq!(RELEASED_PROTOCOL_VERSION, 416);

    let undo_request = LocalDaemonRequest::UndoTurn(crate::local::UndoTurnRequest {
        session_id: "session-1".to_string(),
        agent_ref: Some("worker".to_string()),
        turn_ref: Some("turn-1".to_string()),
    });
    let fork_request = LocalDaemonRequest::ForkAgent(crate::local::ForkAgentRequest {
        session_id: "session-1".to_string(),
        source_agent_ref: None,
        alias: Some("fork".to_string()),
    });
    let undo_response =
        LocalDaemonResponse::TurnUndone {
            result: crate::local::TurnUndoResult {
                session_id: "session-1".to_string(),
                agent_id: "agent-1".to_string(),
                turn_id: "turn-1".to_string(),
                prompt_id: "prompt-1".to_string(),
                provider_run_id: "provider-run-1".to_string(),
                reverted_paths: vec!["src/lib.rs".to_string()],
                path_results:
                    vec![crate::workspace_live_sync_journal::WorkspaceLiveSyncPathApplyResult {
                path: "src/lib.rs".to_string(),
                status: crate::workspace_live_sync_journal::WorkspaceLiveSyncApplyStatus::Applied,
                message: "restored".to_string(),
            }],
            },
        };
    let mut agent_value = serde_json::to_value(crate::agent::AgentInstance::new(
        "agent-2",
        "agent-ref-2",
        "session-1",
        Some("fork".to_string()),
        "codex",
        Some("gpt-5".to_string()),
        Some("medium".to_string()),
        Some("worktree-1".to_string()),
        crate::agent::GridPosition::new(0, 0, 1, 1),
    ))
    .expect("agent snapshot should encode");
    agent_value["created_at_ms"] = serde_json::json!(1_000);
    agent_value["last_activity_at_ms"] = serde_json::json!(1_000);
    let agent: crate::agent::AgentInstance =
        serde_json::from_value(agent_value).expect("agent snapshot should decode");
    let run_request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "codex",
        "codex",
        "default",
        "gpt-5",
    )
    .with_agent_id("agent-2")
    .with_variant(Some("medium".to_string()));
    let run = crate::provider::RuntimeProviderRun::new(
        "provider-run-2",
        &run_request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let mut run_value = serde_json::to_value(run).expect("provider run should encode");
    run_value["started_at_ms"] = serde_json::json!(1_000);
    run_value["last_activity_at_ms"] = serde_json::json!(1_000);
    let mut session_value = serde_json::to_value(crate::session::RuntimeSession::new(
        "session-1",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    ))
    .expect("session snapshot should encode");
    session_value["created_at_ms"] = serde_json::json!(1_000);
    session_value["last_used_at_ms"] = serde_json::json!(1_000);
    let fork_response = ReleasedProviderRunResponse::AgentForked {
        source_agent_id: "agent-1".to_string(),
        agent: Box::new(agent),
        provider_run: serde_json::from_value(run_value).expect("provider run should decode"),
        session: Box::new(
            serde_json::from_value(session_value).expect("session snapshot should decode"),
        ),
    };

    let snapshot = serde_json::json!([undo_request, fork_request, undo_response, fork_response]);
    assert_eq!(
        snapshot.pointer("/0/UndoTurn/agent_ref"),
        Some(&serde_json::json!("worker"))
    );
    assert_eq!(snapshot.pointer("/1/ForkAgent/source_agent_ref"), None);
    assert_eq!(
        snapshot.pointer("/2/TurnUndone/result/path_results/0/status"),
        Some(&serde_json::json!("applied"))
    );
    assert_eq!(
        snapshot.pointer("/3/AgentForked/provider_run/agent_instance_id"),
        Some(&serde_json::json!("agent-2"))
    );
    let serialized = serde_json::to_string(&snapshot).expect("turn action snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "d2b82ed473d960af8e53fbd35b4ecb2ed9ebafc67d0d3fecf4e02932d1a85569"
    );
}

#[test]
fn mp11_released416_provider_run_room_browser_capability_shape_is_versioned() {
    assert_eq!(RELEASED_PROTOCOL_VERSION, 416);

    let mut provider_run = RuntimeProviderRun::from_control_capability_inference(
        "provider-run-browser-capability",
        "session-1".to_string(),
        Some("agent-1".to_string()),
        "codex".to_string(),
    );
    provider_run.set_remote_extension_manifest(crate::extension::RemoteExtensionManifest {
        room_browser_available: true,
        ..crate::extension::RemoteExtensionManifest::default()
    });

    let response = ReleasedProviderRunResponse::ProviderRun { provider_run };
    let snapshot = serde_json::to_value(response).expect("response should serialize");
    let manifest = snapshot
        .pointer("/ProviderRun/provider_run/remote_extension_manifest")
        .expect("home Room browser capability should serialize");
    assert_eq!(
        manifest,
        &serde_json::json!({"room_browser_available": true})
    );
    let serialized = serde_json::to_string(manifest).expect("manifest snapshot should encode");
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "e7190f31ffb024735e1241f18c98c8af31e87a865dd43d54da2c2bfd7cf78b83"
    );
}
