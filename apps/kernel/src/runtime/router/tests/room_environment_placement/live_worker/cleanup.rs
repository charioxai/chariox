use super::*;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

#[test]
fn room_environment_worker_cleanup_retries_after_agent_acknowledgement_loss() {
    run_test(cleanup_retries_after_agent_acknowledgement_loss);
}

async fn cleanup_retries_after_agent_acknowledgement_loss() {
    let mut fixture = LiveWorker::start().await;
    fixture.create_slice().await;
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent":{
            "session_id":fixture.rooms[0], "provider":"managed-dev-stub", "model":"default",
            "slice_ref":"desktop", "worktree_placement":placement
        }}),
    )
    .await
    .unwrap();
    let agent = &spawned["AgentSpawned"]["agent"];
    let agent_id = agent["id"].as_str().unwrap();
    let remote: crate::agent::RemoteAgentBinding =
        serde_json::from_value(agent["remote_execution"].clone()).unwrap();
    let target = ClientTarget {
        daemon_id: Some(remote.worker_kernel_id.clone()),
        daemon_alias: None,
    };
    // Execute the first cleanup phase through the real encrypted peer boundary
    // without letting the home command observe its acknowledgement. This is the
    // state left by a disconnect between worker deletion and the home receipt.
    let first = send_peer_request_via_temporary_connection(
        &fixture.home_state.config,
        target.clone(),
        RelayPeerRequest::DestroyLeasedAgent {
            leased_agent_id: remote.leased_agent_id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        first,
        RelayPeerResponse::LeasedAgentDestroyed {
            leased_agent_id: remote.leased_agent_id.clone(),
        }
    );
    let destroyed = dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{
            "session_id":fixture.rooms[0], "agent_id":agent_id
        }}),
    )
    .await;
    let listed = dispatch_json(
        &fixture.home,
        json!({"ListAgents":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let slice = dispatch_json(&fixture.home, json!({"GetSlice":{"slice_ref":"desktop"}}))
        .await
        .unwrap();
    // Unknown worker IDs must still fail; absence alone is not proof of cleanup.
    let unknown = send_peer_request_via_temporary_connection(
        &fixture.home_state.config,
        target.clone(),
        RelayPeerRequest::DestroyLeasedAgent {
            leased_agent_id: "never-created-agent".to_string(),
        },
    )
    .await;
    if destroyed.is_err() {
        send_peer_request_via_temporary_connection(
            &fixture.home_state.config,
            target,
            RelayPeerRequest::DestroyExecutionLease {
                lease_id: remote.execution_lease_id,
            },
        )
        .await
        .expect("clean up the fixture lease even when the regression fails");
    }
    fixture.stop().await;
    destroyed.expect("retry must finish after the worker has already deleted the agent");
    assert!(
        unknown.is_err(),
        "unknown worker state must not be acknowledged as deleted"
    );
    assert!(listed["AgentsListed"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|candidate| candidate["id"] != agent_id));
    let slice: crate::slice::SliceRecord =
        serde_json::from_value(slice["Slice"]["slice"].clone()).unwrap();
    assert!(slice.agent_ids.is_empty());
}

#[test]
fn room_environment_worker_cleanup_uses_the_slice_private_relay() {
    run_test(cleanup_uses_the_slice_private_relay);
}

async fn cleanup_uses_the_slice_private_relay() {
    let mut fixture = LiveWorker::start_with_private_relay(true).await;
    fixture.create_slice().await;
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent":{
            "session_id":fixture.rooms[0], "provider":"managed-dev-stub", "model":"default",
            "slice_ref":"desktop", "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn through the slice relay with its separate token");
    let agent = &spawned["AgentSpawned"]["agent"];
    let agent_id = agent["id"].as_str().unwrap();
    assert_ne!(
        agent["remote_execution"]["relay_url"],
        json!(fixture.home_state.config.relay_url)
    );
    let destroyed = dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{
            "session_id":fixture.rooms[0], "agent_id":agent_id
        }}),
    )
    .await;
    let listed = dispatch_json(
        &fixture.home,
        json!({"ListAgents":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let slice = dispatch_json(&fixture.home, json!({"GetSlice":{"slice_ref":"desktop"}}))
        .await
        .unwrap();
    fixture.stop().await;
    let destroyed = destroyed.expect("cleanup must use the bound slice relay, not the home relay");
    assert_eq!(destroyed["AgentDestroyed"]["agent"]["id"], agent_id);
    assert!(listed["AgentsListed"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|agent| agent["id"] != agent_id));
    let slice: crate::slice::SliceRecord =
        serde_json::from_value(slice["Slice"]["slice"].clone()).unwrap();
    assert!(slice.agent_ids.is_empty());
}
