use super::*;
use futures_util::FutureExt;

#[test]
fn destroyed_leased_provider_run_is_ended_and_cleanup_succeeds() {
    run_test(destroyed_leased_provider_run_cleanup);
}

#[test]
fn finished_leased_provider_run_is_ended_after_destroy_and_cleanup_succeeds() {
    run_test(finished_leased_provider_run_cleanup);
}

async fn finished_leased_provider_run_cleanup() {
    run_cleanup(true).await;
}

async fn destroyed_leased_provider_run_cleanup() {
    run_cleanup(false).await;
}

async fn run_cleanup(finished: bool) {
    let mut fixture = LiveWorker::start().await;
    let result = std::panic::AssertUnwindSafe(check_cleanup(&mut fixture, finished))
        .catch_unwind()
        .await;
    fixture
        .worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true)
        .expect("clean fixture-owned provider processes");
    fixture.stop().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

async fn check_cleanup(fixture: &mut LiveWorker, finished: bool) {
    fixture.create_slice().await;
    let room = fixture.rooms[0].clone();
    let listed = dispatch_json(&fixture.home, json!({"ListAgents":{"session_id":room}}))
        .await
        .unwrap();
    let remaining = listed["AgentsListed"]["agents"][0]["id"].as_str().unwrap();
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent":{
            "session_id":room, "provider":"managed-dev-stub", "model":"native-tui-idle",
            "slice_ref":"desktop", "worktree_placement":placement
        }}),
    )
    .await
    .unwrap();
    let agent = spawned["AgentSpawned"]["agent"]["id"].as_str().unwrap();
    let attached = dispatch_json(
        &fixture.home,
        json!({"AttachToSession":{
            "session_id":room,"client_id":"leased-cleanup","capability_level":"FullTerminal"
        }}),
    )
    .await
    .unwrap();
    let attachment = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap();
    dispatch_json(
        &fixture.home,
        json!({"FocusAgent":{"session_id":room,"agent_id":agent}}),
    )
    .await
    .unwrap();
    dispatch_json(&fixture.home, json!({"SubmitPrompt":{
        "session_id":room,"attachment_id":attachment,"target_agent_id":agent,"prompt":"keep this leased turn active"
    }})).await.expect("submit a leased dev-stub turn through the real relay");
    let run = timeout(Duration::from_secs(10), async {
        loop {
            let runs = fixture.home.provider_run_projection.list_for_session(&room);
            if let Some(run) = runs.into_iter().find(|run| {
                run.agent_instance_id() == Some(agent)
                    && run.state() == crate::provider::ProviderRunState::Running
            }) {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("leased turn projects a running worker run");
    let remote: crate::agent::RemoteAgentBinding =
        serde_json::from_value(spawned["AgentSpawned"]["agent"]["remote_execution"].clone())
            .unwrap();
    let worker_run_id = crate::provider::worker_provider_run_id_from_projected_leased_id(
        &remote.leased_agent_id,
        run.id(),
    )
    .unwrap();
    let worker_snapshot = fixture
        .worker
        .app
        .lock()
        .await
        .providers()
        .get_run(&worker_run_id)
        .expect("capture the real worker snapshot before destruction");
    if finished {
        dispatch_json(&fixture.home, json!({"CompletePrompt":{"session_id":room}}))
            .await
            .expect("finish the leased turn before destroying the agent");
    }
    dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{"session_id":room,"agent_id":agent}}),
    )
    .await
    .expect("worker cleanup and home agent deletion succeed");
    let focus = dispatch_json(
        &fixture.home,
        json!({"FocusAgent":{"session_id":room,"agent_id":remaining}}),
    )
    .await;
    let read = dispatch_json(
        &fixture.home,
        json!({"GetProviderRun":{"provider_run_id":run.id()}}),
    )
    .await;
    focus
        .expect("cleanup must not resolve the destroyed leased run in the local provider registry");
    assert_eq!(
        read.unwrap()["ProviderRun"]["provider_run"]["state"],
        "Ended"
    );
    // Replay through the same router entry point as a decrypted relay event.
    // The destroyed agent must reject it before any routing/profile mutation.
    let late = fixture
        .home
        .relay_project_remote_runtime_projection(
            crate::runtime::relay_peer_authority::test_projection_authority("worker-1"),
            crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                account_copy_observations: Vec::new(),
                home_session_id: room.to_string(),
                home_agent_id: agent.to_string(),
                provider_run_id: worker_run_id.to_string(),
                provider_run: Some(worker_snapshot),
                prompts: Vec::new(),
                output_chunks: Vec::new(),
                notices: Vec::new(),
                completions: Vec::new(),
            },
        )
        .await
        .expect_err("a deleted agent cannot accept a late worker projection");
    assert!(matches!(late, DaemonError::AgentNotFound { .. }));
    let reread = dispatch_json(
        &fixture.home,
        json!({"GetProviderRun":{"provider_run_id":run.id()}}),
    )
    .await
    .unwrap();
    assert_eq!(reread["ProviderRun"]["provider_run"]["state"], "Ended");
    dispatch_json(
        &fixture.home,
        json!({"FocusAgent":{"session_id":room,"agent_id":remaining}}),
    )
    .await
    .expect("cleanup stays idempotent after the late snapshot");
    let unknown = dispatch_json(
        &fixture.home,
        json!({"GetProviderRun":{"provider_run_id":"never-created-run"}}),
    )
    .await
    .expect_err("unknown runs must remain errors");
    assert!(matches!(unknown, DaemonError::ProviderRunNotFound { .. }));
    assert_eq!(
        crate::app::RemoteLeaseRuntime::new(&mut *fixture.worker.app.lock().await)
            .leased_agent_count(),
        0
    );
}
