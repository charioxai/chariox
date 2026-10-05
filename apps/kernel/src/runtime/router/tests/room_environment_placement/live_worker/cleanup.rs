use super::*;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

#[test]
fn room_environment_worker_cleanup_retries_after_agent_acknowledgement_loss() {
    crate::test_support::isolated_env_test!();
    run_test(cleanup_retries_after_agent_acknowledgement_loss);
}

async fn cleanup_retries_after_agent_acknowledgement_loss() {
    cleanup_retries_after_acknowledgement_loss(false).await;
}

#[test]
fn room_environment_worker_cleanup_retries_after_lease_acknowledgement_loss() {
    crate::test_support::isolated_env_test!();
    run_test(cleanup_retries_after_lease_acknowledgement_loss);
}

async fn cleanup_retries_after_lease_acknowledgement_loss() {
    cleanup_retries_after_acknowledgement_loss(true).await;
}

async fn cleanup_retries_after_acknowledgement_loss(lease_also_deleted: bool) {
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
    if lease_also_deleted {
        let second = send_peer_request_via_temporary_connection(
            &fixture.home_state.config,
            target.clone(),
            RelayPeerRequest::DestroyExecutionLease {
                lease_id: remote.execution_lease_id.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            second,
            RelayPeerResponse::ExecutionLeaseDestroyed {
                lease_id: remote.execution_lease_id.clone(),
            }
        );
    }
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
    if lease_also_deleted {
        let unknown_lease = send_peer_request_via_temporary_connection(
            &fixture.home_state.config,
            target.clone(),
            RelayPeerRequest::DestroyExecutionLease {
                lease_id: "never-created-lease".to_string(),
            },
        )
        .await;
        assert!(
            unknown_lease.is_err(),
            "an unknown lease must not count as completed cleanup"
        );
    }
    if destroyed.is_err() && !lease_also_deleted {
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
    crate::test_support::isolated_env_test!();
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

#[test]
fn room_environment_worker_cleanup_finalizes_when_live_lease_state_is_lost() {
    crate::test_support::isolated_env_test!();
    run_test(cleanup_when_live_lease_state_is_lost);
}

async fn cleanup_when_live_lease_state_is_lost() {
    cleanup_after_worker_state_loss(false).await;
}

#[test]
fn room_environment_worker_cleanup_finalizes_after_worker_restart() {
    crate::test_support::isolated_env_test!();
    run_test(cleanup_after_worker_restart);
}

async fn cleanup_after_worker_restart() {
    cleanup_after_worker_state_loss(true).await;
}

async fn cleanup_after_worker_state_loss(restart: bool) {
    let mut fixture = LiveWorker::start().await;
    fixture.create_slice().await;
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent": {
            "session_id":fixture.rooms[0], "provider":"managed-dev-stub", "model":"default",
            "slice_ref":"desktop", "worktree_placement":placement
        }}),
    )
    .await
    .unwrap();
    let agent_id = spawned["AgentSpawned"]["agent"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    if restart {
        // Drop the worker connector and bootstrap a fresh kernel against the same
        // durable store: both live leases and volatile caller/tombstone maps vanish.
        let task = fixture.tasks.pop().unwrap();
        task.abort();
        let _ = task.await;
        // Release the old store's ownership fence before reacquiring it.
        fixture.worker = Arc::clone(&fixture.home);
        let app = crate::test_support::bootstrap_after_test_owner_exit(
            fixture._worker_state.config.clone(),
        )
        .await;
        fixture.worker = Arc::new(CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            2,
        ));
        let state = fixture.worker.app.lock().await.relay_client_state();
        fixture.tasks.push(tokio::spawn(
            crate::transport::relay_client::run_daemon_relay_connector_with_router(
                Arc::clone(&fixture.worker),
                Arc::clone(&state),
                fixture.shutdown.subscribe(),
            ),
        ));
        timeout(Duration::from_secs(10), async {
            while !state.read().await.connected() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("restarted worker registered");
    } else {
        crate::app::RemoteLeaseRuntime::new(&mut *fixture.worker.app.lock().await)
            .forget_live_leases_for_test();
    }
    let destroyed = dispatch_json(
        &fixture.home,
        json!({"DestroyAgent": {
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
    let receipts = fixture
        .home
        .app
        .lock()
        .await
        .durable_state_store()
        .load_events_by_kind("agent.worker_cleanup.already_absent")
        .unwrap();
    fixture.stop().await;
    destroyed.expect("bound worker absence must finalize home cleanup");
    assert_eq!(
        receipts.len(),
        2,
        "both cleanup phases record worker absence"
    );
    assert!(receipts
        .iter()
        .all(|event| event.payload["worker_kernel_id"] == "environment-worker"));
    assert!(listed["AgentsListed"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["id"] != agent_id));
    let slice: crate::slice::SliceRecord =
        serde_json::from_value(slice["Slice"]["slice"].clone()).unwrap();
    assert!(slice.agent_ids.is_empty());
}
