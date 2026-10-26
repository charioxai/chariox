// MP-08 / MP-10 / MP-11 G03: connection loss is visible without changing
// the home task, its source, or the worker's lease. Regression, not acceptance.
use super::*;

#[test]
fn mp10_g03_waiting_leased_task_reports_disconnect_and_recovery() {
    run_test(waiting_leased_task_reports_disconnect_and_recovery);
}

async fn waiting_leased_task_reports_disconnect_and_recovery() {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start().await;
    let worker = start_agent_worker(&fixture, agent_worker_state(&fixture)).await;
    let room = fixture.rooms[0].clone();
    let agent =
        spawn_on_agent_worker_with_model(&mut fixture, &room, "controlled-cancel-idle").await;
    let attachment = owner_attachment(&fixture, &room).await;
    run_turn(
        &fixture,
        &room,
        &attachment,
        &agent,
        "MP-10 G03 wait on a timer",
    )
    .await;
    let store = store(&fixture).await;
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .pop()
        .unwrap();
    let first = worker.run_for(&fixture, &agent, &task.prompt_id).await;
    let token = first.runtime_mcp_auth_token().unwrap();
    let timer = worker.router.dispatch_authenticated_runtime_tool_call(
        token,
        "chariox.events.timer",
        json!({"task_id":task.task_id,"origin_prompt_id":task.prompt_id,"delay_ms":600000,"label":"G03"}),
    ).await.unwrap();
    worker
        .router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.events.yield",
            json!({"task_id":task.task_id,"origin_prompt_id":task.prompt_id,
            "registration_ids":[timer.payload["registration_id"]],"inbox_cursor":0,
            "reason":"Wait on the G03 timer","deadline_ms":crate::session::unix_epoch_ms()+600000}),
        )
        .await
        .unwrap();
    fixture
        .home
        .runtime_state
        .record_leased_answer_for_test(&room, &agent, "Waiting on G03")
        .unwrap();
    let remote = binding(&fixture, &agent).await;
    super::complete_worker_turn(&worker.router, &remote.leased_agent_id).await;
    let before = eventually("the leased task waits", || {
        store
            .agent_tasks(Some(&room), Some(&agent))
            .unwrap()
            .into_iter()
            .find(|t| t.state == ledger::ExecutionState::Waiting)
    })
    .await;
    let before_binding = binding(&fixture, &agent).await;
    let runtime = fixture.home.runtime_state();
    runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    let connected_notices = runtime.agent_notices_for_test(&room, &agent);

    let AgentWorker {
        state,
        router,
        stop,
        task,
    } = worker;
    stop.send(true).unwrap();
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    let disconnected_notices = runtime.agent_notices_for_test(&room, &agent);
    let disconnected_task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .pop()
        .unwrap();

    let (stop, receiver) = watch::channel(false);
    let relay_state = router.app.lock().await.relay_client_state();
    let task = tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router(
            Arc::clone(&router),
            relay_state,
            receiver,
        ),
    );
    eventually_async("the same worker reconnects", || async {
        send_peer_request_via_temporary_connection(
            &fixture.home_state.config,
            ClientTarget {
                daemon_id: Some(AGENT_WORKER.into()),
                daemon_alias: None,
            },
            RelayPeerRequest::Ping {
                value: "G03 reconnected".into(),
            },
        )
        .await
        .ok()
        .map(|_| ())
    })
    .await;
    let worker = AgentWorker {
        state,
        router,
        stop,
        task,
    };
    runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    runtime.sweep_agent_lifecycle_for_test().await.unwrap();
    let recovered_notices = runtime.agent_notices_for_test(&room, &agent);
    let after = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .pop()
        .unwrap();
    let after_binding = binding(&fixture, &agent).await;
    worker.stop().await;
    fixture.stop().await;

    let uncertain_count = |notices: &[String]| {
        notices
            .iter()
            .filter(|s| s.contains("Worker connection uncertain"))
            .count()
    };
    assert_eq!(
        uncertain_count(&connected_notices),
        0,
        "a reachable worker needs no warning"
    );
    assert_eq!(
        uncertain_count(&disconnected_notices),
        1,
        "worker loss stayed invisible or repeated its warning"
    );
    assert_eq!(
        uncertain_count(&recovered_notices),
        1,
        "recovery duplicated the loss warning"
    );
    assert_eq!(
        recovered_notices
            .iter()
            .filter(|s| s.contains("Worker connection restored"))
            .count(),
        1
    );
    // The ordinary sweep advances its check timestamp even when the task stays waiting.
    let without_check_timestamp = |mut task: ledger::AgentTaskExecution| {
        if let Some(wait) = task.wait.as_mut() {
            wait.last_checked_at_ms = 0;
        }
        task
    };
    assert_eq!(
        without_check_timestamp(before.clone()),
        without_check_timestamp(disconnected_task),
        "a connection warning must not change a waiting task"
    );
    assert_eq!(
        without_check_timestamp(before),
        without_check_timestamp(after),
        "reconnection must retain the same source and deadline"
    );
    assert_eq!(
        (
            &before_binding.worker_kernel_id,
            &before_binding.execution_lease_id,
            &before_binding.leased_agent_id,
            &before_binding.active_worker_provider_run_id
        ),
        (
            &after_binding.worker_kernel_id,
            &after_binding.execution_lease_id,
            &after_binding.leased_agent_id,
            &after_binding.active_worker_provider_run_id
        ),
        "reconnection must not replace the execution lease"
    );
}
