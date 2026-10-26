//! MP-08/MP-09/MP-10/MP-11 G06: forwarded deletion must leave the private
//! connector's current-thread runtime polling its cleanup acknowledgements.
use super::*;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

#[test]
fn forwarded_delete_on_current_thread_private_connector_acknowledges_worker_cleanup() {
    crate::test_support::isolated_env_test!();
    run_test(forwarded_delete);
}

async fn forwarded_delete() {
    let _room_tools = HomePersistenceEnvironment::set("CHARIOX_ROOM_AGENT_TOOLS", "1".as_ref());
    let mut fixture = LiveWorker::start_with_private_relay(true).await;
    fixture.create_slice().await;
    let room = fixture.rooms[0].clone();
    let mut agents = Vec::new();
    for alias in ["delegator", "child"] {
        let placement = fixture.placement();
        let spawned = dispatch_json(
            &fixture.home,
            json!({"SpawnAgent": {
                "session_id": room, "alias": alias,
                "provider": "managed-dev-stub", "model": "default",
                "slice_ref": "desktop", "worktree_placement": placement
            }}),
        )
        .await
        .unwrap();
        agents.push(
            serde_json::from_value::<crate::agent::AgentInstance>(
                spawned["AgentSpawned"]["agent"].clone(),
            )
            .unwrap(),
        );
    }
    let parent = &agents[0];
    let child = &agents[1];
    let mut parent_binding = parent.remote_execution().unwrap().clone();
    parent_binding.active_worker_provider_run_id = Some("delegator-worker-run".into());
    {
        let app = fixture.home.app.lock().await;
        app.agents()
            .bind_remote_execution(parent.id(), parent_binding.clone())
            .unwrap();
        let mut child = child.clone();
        child.record_spawn_creator(Some(parent.id().to_string()));
        app.agents_mut().restore_agent(child);
    }
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: fixture.home_state.config.daemon_id.clone(),
        home_session_id: room.clone(),
        home_agent_id: parent.id().to_string(),
        home_prompt_id: None,
        leased_agent_id: parent_binding.leased_agent_id.clone(),
        worker_kernel_id: parent_binding.worker_kernel_id.clone(),
        worker_machine_id: parent_binding.worker_machine_id.clone(),
        worker_provider_run_id: "delegator-worker-run".into(),
        worker_worktree_path: fixture._worker_state.worktree.path().display().to_string(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(
            fixture._worker_state.worktree.path().display().to_string(),
        ),
    };
    // create_slice starts the production private connector, including its
    // own current-thread runtime and router/session lanes. Forward over that
    // socket rather than invoking the home's main router directly.
    let deleted = send_peer_request_via_temporary_connection(
        &fixture._worker_state.config,
        ClientTarget {
            daemon_id: Some(fixture.home_state.config.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::ForwardMetaRuntimeTool {
            context,
            tool_name: crate::transport::runtime_tools::META_RUN_COMMAND_TOOL.into(),
            arguments: json!({"command": format!("agent delete {}", child.agent_ref())}),
        },
    )
    .await;
    let home_deleted = fixture
        .home
        .app
        .lock()
        .await
        .agents()
        .get_agent(child.id())
        .is_err();
    let (worker_agents, worker_leases) = {
        let mut app = fixture.worker.app.lock().await;
        let worker = crate::app::RemoteLeaseRuntime::new(&mut app);
        (worker.leased_agent_count(), worker.execution_lease_count())
    };
    let responsive = timeout(
        Duration::from_secs(1),
        dispatch_json(&fixture.home, json!({"ListSessions": null})),
    )
    .await;
    fixture.stop().await;

    let RelayPeerResponse::MetaRuntimeToolHandled { result } =
        deleted.expect("private connector returns the forwarded command response")
    else {
        panic!("expected forwarded Meta command response")
    };
    assert!(
        result.ok,
        "worker must acknowledge deletion: {:?}",
        result.payload
    );
    assert!(
        home_deleted,
        "the home removes the child after acknowledgement"
    );
    assert_eq!(worker_agents, 1, "only the delegator remains on the worker");
    assert_eq!(worker_leases, 1, "the child's lease is also acknowledged");
    responsive
        .unwrap()
        .expect("another app command remains usable");
}
