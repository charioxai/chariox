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
        "6aab10f8400cfb7d35de4834227bcf4547f3ef82a37e691f166d1c6ec39c429c"
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

// MP-08 / MP-11, peer 441: a worker answers a spawn and every leased-agent update with the
// checkout it created for the agent, so the home names the folder the agent runs in.
#[test]
fn mp11_leased_agent_created_worktree_441_is_pinned() {
    const { assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 441) };
    use crate::transport::relay_peer::RelayPeerResponse;
    let mut leased_agent = crate::execution_lease::LeasedAgent::new(
        "leased-agent-1".to_string(),
        "lease-1".to_string(),
        "agent-home".to_string(),
        "codex".to_string(),
        "default".to_string(),
        Some("gpt-5.4".to_string()),
        Some("high".to_string()),
        None,
        None,
        "backing-session".to_string(),
        "backing-agent".to_string(),
        "backing-attachment".to_string(),
    );
    leased_agent.created_worktree_id = Some("/srv/worker/checkouts/kid".to_string());
    leased_agent.created_at_ms = 1_800_000_000_000;
    let expected_agent = serde_json::json!({
        "id": "leased-agent-1",
        "lease_id": "lease-1",
        "home_agent_id": "agent-home",
        "provider": "codex",
        "account_profile": "default",
        "model": "gpt-5.4",
        "effort": "high",
        "execution_mode": null,
        "permission_level": null,
        "backing_session_id": "backing-session",
        "backing_agent_id": "backing-agent",
        "backing_attachment_id": "backing-attachment",
        "created_worktree_id": "/srv/worker/checkouts/kid",
        "created_at_ms": 1_800_000_000_000u64,
    });
    let responses = [
        RelayPeerResponse::LeasedAgentSpawned {
            leased_agent: leased_agent.clone(),
        },
        RelayPeerResponse::LeasedAgentConfigUpdated {
            leased_agent: leased_agent.clone(),
        },
        RelayPeerResponse::LeasedAgentProfileUpdated {
            leased_agent: leased_agent.clone(),
        },
        RelayPeerResponse::LeasedAgentMetaModeUpdated { leased_agent },
    ];
    let snapshot: Vec<serde_json::Value> = responses
        .iter()
        .map(|response| serde_json::to_value(response).unwrap())
        .collect();
    for (response, value) in responses.iter().zip(&snapshot) {
        assert_eq!(value["leased_agent"], expected_agent, "{value}");
        let decoded: RelayPeerResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(&decoded, response);
    }
    assert_eq!(
        snapshot
            .iter()
            .map(|value| value["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "leased_agent_spawned",
            "leased_agent_config_updated",
            "leased_agent_profile_updated",
            "leased_agent_meta_mode_updated",
        ]
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "293c17776d84fbf729e01a48b99a43bd063a008bef99b7f229379a57e0f22f3f"
    );
}
