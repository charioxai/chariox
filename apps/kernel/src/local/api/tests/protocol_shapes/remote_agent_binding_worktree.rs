use super::*;

// MP-08 / MP-11: local 1000 adds `worker_worktree_id` to an agent's remote binding: the folder
// the worker runs the child in, sent again whenever the binding is renewed. Absent when unknown.
const FIXTURE: &[u8] = include_bytes!("remote-agent-binding-worktree-1000.json");

fn remote_agent(worker_worktree_id: Option<&str>) -> crate::agent::AgentInstance {
    let mut agent = crate::agent::AgentInstance::new(
        "agent-remote",
        "agent-ref-remote",
        "session-1",
        None,
        "codex",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    agent.set_remote_execution(Some(crate::agent::RemoteAgentBinding {
        worker_kernel_id: "worker-kernel".to_string(),
        worker_machine_id: "worker-machine".to_string(),
        execution_lease_id: "lease-1".to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        active_worker_provider_run_id: Some("worker-run-1".to_string()),
        relay_url: Some("wss://relay.example.test".to_string()),
        relay_token: None,
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
        worker_worktree_id: worker_worktree_id.map(str::to_string),
    }));
    agent
}

fn sent_binding(agent: crate::agent::AgentInstance) -> serde_json::Value {
    let response = LocalDaemonResponse::AgentMovedToRemote { agent };
    serde_json::json!(response)
        .pointer("/AgentMovedToRemote/agent/remote_execution")
        .cloned()
        .expect("remote binding should be present")
}

#[test]
fn mp11_remote_agent_binding_worktree_1000_snapshot_and_hash() {
    assert_eq!(
        format!("{:x}", Sha256::digest(FIXTURE)),
        "46517cdec2aca93adf40f94eb0bf3b9e208a0666fe492e77eb8502dc06bd07d4"
    );
    let fixture: serde_json::Value = serde_json::from_slice(FIXTURE).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    assert_eq!(
        fixture["remote_execution"]["worker_worktree_id"],
        "room-folder"
    );
}

#[test]
fn mp11_a_remote_agent_sends_its_worker_folder_and_omits_an_unknown_one() {
    let fixture: serde_json::Value = serde_json::from_slice(FIXTURE).unwrap();
    assert_eq!(
        sent_binding(remote_agent(Some("room-folder"))),
        fixture["remote_execution"]
    );
    let without = sent_binding(remote_agent(None));
    assert!(without.get("worker_worktree_id").is_none());
    let decoded: crate::agent::RemoteAgentBinding = serde_json::from_value(without).unwrap();
    assert_eq!(decoded.worker_worktree_id, None);
}
