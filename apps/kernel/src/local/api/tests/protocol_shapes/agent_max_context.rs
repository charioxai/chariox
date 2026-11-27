//! Local 1009: an agent may carry a max context, set at spawn or through its config.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn agent_max_context_1009_snapshot_and_hash() {
    let bytes = include_bytes!("agent-max-context-1009.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "9684bac964ee41875bee0759a983ed3f511eaf08f77f649d2dceea12aa6954ae"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    for wire in fixture["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
}

/// The setting is optional on every wire shape: a request without it is the provider's default.
#[test]
fn a_request_without_a_max_context_keeps_the_provider_default() {
    let spawn: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "SpawnAgent": {"session_id": "room", "alias": null, "model": null, "effort": null,
                       "worktree_id": null, "kernel_ref": null}
    }))
    .unwrap();
    let LocalDaemonRequest::SpawnAgent(spawn) = spawn else {
        panic!("not a spawn")
    };
    assert_eq!(spawn.max_context_tokens, None);
    let config: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "UpdateAgentConfig": {"session_id": "room", "agent_id": "agent"}
    }))
    .unwrap();
    let LocalDaemonRequest::UpdateAgentConfig(config) = config else {
        panic!("not a config update")
    };
    assert_eq!(
        (config.max_context_tokens, config.clear_max_context_tokens),
        (None, false)
    );
}
