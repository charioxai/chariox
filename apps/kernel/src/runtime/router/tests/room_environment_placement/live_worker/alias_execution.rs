use super::*;
use futures_util::FutureExt;

#[test]
fn room_environment_friendly_alias_real_relay_denies_before_contact_and_moves_canonically() {
    run_test(real_relay_denies_before_contact_and_moves_canonically);
}

async fn worker_counts(fixture: &LiveWorker) -> (usize, usize) {
    let mut app = fixture.worker.app.lock().await;
    let worker = crate::app::RemoteLeaseRuntime::new(&mut app);
    (worker.execution_lease_count(), worker.leased_agent_count())
}

async fn real_relay_denies_before_contact_and_moves_canonically() {
    let mut fixture = LiveWorker::start_configured_with_home_vault_and_worker_id(
        false,
        false,
        None,
        false,
        "unused-until-public-slice-creation".into(),
        true,
        true,
        None,
    )
    .await;
    let result = std::panic::AssertUnwindSafe(check_alias_execution(&fixture))
        .catch_unwind()
        .await;
    fixture
        .worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true)
        .expect("clean fixture-owned worker provider processes");
    fixture
        .home
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true)
        .expect("clean fixture-owned home provider processes");
    fixture.stop().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

async fn check_alias_execution(fixture: &LiveWorker) {
    dispatch_json(&fixture.home, bind(&fixture.rooms[0], "desktop"))
        .await
        .unwrap();
    let slices = fixture.home.app.lock().await.slices().clone();
    let record = slices.resolve("desktop").unwrap();
    assert_eq!(record.worker_kernel_ref.len(), 135);
    assert_eq!(
        fixture._worker_state.config.daemon_alias.as_deref(),
        Some("slice:desktop")
    );
    assert_eq!(
        fixture._worker_state.config.daemon_id,
        record.worker_kernel_ref
    );
    assert_eq!(worker_counts(fixture).await, (0, 0));
    for busy in [false, true] {
        let operation = busy.then(|| slices.try_begin_operation("desktop", "state.save").unwrap());
        let room = &fixture.rooms[if busy { 0 } else { 1 }];
        let agents = dispatch_json(&fixture.home, json!({"ListAgents":{"session_id":room}}))
            .await
            .unwrap();
        let agent_id = agents["AgentsListed"]["agents"][0]["id"].as_str().unwrap();
        for request in [
            json!({"SpawnAgent":{"session_id":room,"provider":"managed-dev-stub","kernel_ref":"slice:desktop"}}),
            json!({"SpawnAgents":{"session_id":room,"agents":[{"provider":"managed-dev-stub","kernel_ref":"slice:desktop"}]}}),
            json!({"CreateSession":{"workspace_id":"unprepared","worktree_id":"unprepared","kernel_ref":"slice:desktop"}}),
            json!({"MoveAgentToRemote":{"session_id":room,"agent_ref":agent_id,"machine_ref":"slice:desktop"}}),
        ] {
            let error = timeout(
                Duration::from_secs(5),
                dispatch_json(&fixture.home, request.clone()),
            )
            .await
            .expect("admission must settle without contacting the worker")
            .unwrap_err()
            .to_string();
            let expected = if busy {
                "active `state.save` operation"
            } else {
                "environment_slice_access_denied"
            };
            assert!(error.contains(expected), "{error}");
            let counts = worker_counts(fixture).await;
            assert_eq!(
                counts,
                (0, 0),
                "denied public request must not create a worker lease or agent"
            );
            eprintln!(
                "ALIAS_ADMISSION_RECEIPT {}",
                json!({
                    "phase":if busy {"active-operation"} else {"foreign-room"},
                    "request":request, "error":error, "workerKernelId":record.worker_kernel_ref,
                    "workerLeaseCount":counts.0, "workerAgentCount":counts.1,
                })
            );
        }
        assert_eq!(
            dispatch_json(&fixture.home, json!({"ListAgents":{"session_id":room}}))
                .await
                .unwrap(),
            agents
        );
        drop(operation);
    }
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent":{
            "session_id":fixture.rooms[0], "provider":"managed-dev-stub", "model":"default"
        }}),
    )
    .await
    .expect("create an ordinary local agent before moving it");
    let agent_id = spawned["AgentSpawned"]["agent"]["id"].as_str().unwrap();
    let moved = timeout(
        Duration::from_secs(15),
        dispatch_json(
            &fixture.home,
            json!({"MoveAgentToRemote":{
                "session_id":fixture.rooms[0], "agent_ref":agent_id, "machine_ref":"slice:desktop"
            }}),
        ),
    )
    .await
    .expect("move settles through the real relay")
    .expect("owner alias moves to admitted canonical worker");
    let remote = &moved["AgentMovedToRemote"]["agent"]["remote_execution"];
    assert_eq!(remote["worker_kernel_id"], record.worker_kernel_ref);
    assert_eq!(remote["worker_machine_id"], record.owner_machine_id);
    assert_eq!(worker_counts(fixture).await, (1, 1));
    assert!(slices
        .resolve("desktop")
        .unwrap()
        .agent_ids
        .iter()
        .any(|id| id == agent_id));
    eprintln!(
        "ALIAS_CANONICAL_MOVE_RECEIPT {}",
        json!({
            "requestedAlias":"slice:desktop", "workerKernelId":remote["worker_kernel_id"],
            "workerMachineId":remote["worker_machine_id"], "sliceId":record.id,
            "agentId":agent_id, "attachedToAdmittedSlice":true,
        })
    );
    dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{"session_id":fixture.rooms[0],"agent_id":agent_id}}),
    )
    .await
    .expect("destroy fixture's remote agent");
    assert_eq!(worker_counts(fixture).await, (0, 0));
    slices
        .delete("desktop")
        .expect("simulate lost local record while worker stays registered");
    for reference in ["slice:desktop", record.worker_kernel_ref.as_str()] {
        let request = json!({"SpawnAgent":{
            "session_id":fixture.rooms[0], "provider":"managed-dev-stub", "kernel_ref":reference
        }});
        let error = timeout(
            Duration::from_secs(5),
            dispatch_json(&fixture.home, request.clone()),
        )
        .await
        .expect("missing physical record must fail before transport")
        .unwrap_err()
        .to_string();
        assert!(error.contains("no local physical slice record"), "{error}");
        assert_eq!(worker_counts(fixture).await, (0, 0));
        eprintln!(
            "ALIAS_ADMISSION_RECEIPT {}",
            json!({
                "phase":"missing-record-live-worker", "request":request, "error":error,
                "workerLeaseCount":0, "workerAgentCount":0,
            })
        );
    }
}
