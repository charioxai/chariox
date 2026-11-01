// MP-08 / MP-10 / MP-11 G03: a short connection loss is visible without
// changing the home task, its source, or the worker's lease. A10-S04: a worker
// the relay reports offline for the grace period ends its turn as failed.
// Regression, not acceptance.
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
    let (backing_session, _) =
        crate::app::RemoteLeaseRuntime::new(&mut *worker.router.app.lock().await)
            .leased_agent_backing(&remote.leased_agent_id)
            .unwrap();
    dispatch_json(
        &worker.router,
        json!({"CompletePrompt":{"session_id":backing_session}}),
    )
    .await
    .expect("the independent worker finishes its leased turn");
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

#[test]
fn mp10_a10s04_offline_worker_ends_the_leased_turn_failed_and_wakes_the_delegator() {
    run_test(offline_worker_ends_the_leased_turn_failed_and_wakes_the_delegator);
}

/// A helper whose worker kernel dies mid-turn is resolved by the sweep within
/// seconds, not at its delegator's deadline: the turn ends as a typed provider
/// crash that names the worker, and the delegator wakes with that failure.
async fn offline_worker_ends_the_leased_turn_failed_and_wakes_the_delegator() {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start().await;
    let worker = start_agent_worker(&fixture, agent_worker_state(&fixture)).await;
    let room = fixture.rooms[0].clone();
    let agent =
        spawn_on_agent_worker_with_model(&mut fixture, &room, "controlled-cancel-idle").await;
    let attachment = owner_attachment(&fixture, &room).await;
    run_turn(&fixture, &room, &attachment, &agent, "A10-S04 hold").await;
    let store = store(&fixture).await;
    let delegator = local_agent(&fixture, &room, &agent).await;
    delegator_waiting_on(&store, &room, &delegator, &agent);
    let running = eventually("the leased turn runs", || {
        store
            .agent_tasks(Some(&room), Some(&agent))
            .unwrap()
            .into_iter()
            .find(|t| t.state == ledger::ExecutionState::Working)
    })
    .await;
    let runtime = fixture.home.runtime_state();
    let start = crate::session::unix_epoch_ms();
    runtime
        .sweep_agent_lifecycle_at_for_test(start)
        .await
        .unwrap();

    let AgentWorker { stop, task, .. } = worker;
    stop.send(true).unwrap();
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    runtime
        .sweep_agent_lifecycle_at_for_test(start + 1_000)
        .await
        .unwrap();
    let just_lost = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == running.task_id)
        .unwrap();
    runtime
        .sweep_agent_lifecycle_at_for_test(start + 45_000)
        .await
        .unwrap();
    let ended = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == running.task_id)
        .unwrap();
    let woken: Vec<_> = store
        .agent_inbox(&room, &delegator, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.source_id == agent)
        .collect();
    fixture.stop().await;

    assert_eq!(
        just_lost.state,
        ledger::ExecutionState::Working,
        "a short loss must not end the turn"
    );
    assert_eq!(
        ended.state,
        ledger::ExecutionState::Cancelled,
        "the lost worker's turn stayed {:?}: {}",
        ended.state,
        ended.reason
    );
    assert_eq!(woken.len(), 1, "one wake for the delegator: {woken:?}");
    assert_eq!(woken[0].kind, "source_lost", "{woken:?}");
    let answer = &woken[0].payload["public_answer"];
    assert_eq!(answer["outcome"]["kind"], "provider_crashed", "{answer}");
    assert!(
        answer["outcome"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("worker kernel")),
        "the failure names the lost worker: {answer}"
    );
}

#[test]
fn mp10_a10s04_worker_that_lost_only_its_relay_is_cancelled_when_it_reconnects() {
    run_test(worker_that_lost_only_its_relay_is_cancelled_when_it_reconnects);
}

/// The worker's relay link drops past the grace period while its provider keeps
/// running. The delegator is woken in bounded time, but the loss is not proof
/// that the worker stopped: when the worker is reachable again the home cancels
/// that exact run, and a result the run still produces is not taken as live.
async fn worker_that_lost_only_its_relay_is_cancelled_when_it_reconnects() {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start().await;
    let worker = start_agent_worker(&fixture, agent_worker_state(&fixture)).await;
    let room = fixture.rooms[0].clone();
    let agent =
        spawn_on_agent_worker_with_model(&mut fixture, &room, "controlled-cancel-idle").await;
    let attachment = owner_attachment(&fixture, &room).await;
    run_turn(&fixture, &room, &attachment, &agent, "A10-S04 relay cut").await;
    let store = store(&fixture).await;
    let delegator = local_agent(&fixture, &room, &agent).await;
    delegator_waiting_on(&store, &room, &delegator, &agent);
    let running = eventually("the leased turn runs", || {
        store
            .agent_tasks(Some(&room), Some(&agent))
            .unwrap()
            .into_iter()
            .find(|t| t.state == ledger::ExecutionState::Working)
    })
    .await;
    let worker_run = worker.run_for(&fixture, &agent, &running.prompt_id).await;
    let runtime = fixture.home.runtime_state();
    let start = crate::session::unix_epoch_ms();
    runtime
        .sweep_agent_lifecycle_at_for_test(start)
        .await
        .unwrap();

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
    runtime
        .sweep_agent_lifecycle_at_for_test(start + 1_000)
        .await
        .unwrap();
    runtime
        .sweep_agent_lifecycle_at_for_test(start + 45_000)
        .await
        .unwrap();
    let ended = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == running.task_id)
        .unwrap();
    let worker_run_state = |router: &Arc<CommandRouter>| {
        let router = Arc::clone(router);
        let id = worker_run.id().to_string();
        async move {
            router
                .app
                .lock()
                .await
                .providers()
                .get_run(&id)
                .map(|run| run.state())
        }
    };
    let while_offline = worker_run_state(&router).await;

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
                value: "A10-S04 reconnected".into(),
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
    let mut after_reconnect = None;
    for step in 1..=8 {
        runtime
            .sweep_agent_lifecycle_at_for_test(start + 45_000 + step * 30_000)
            .await
            .unwrap();
        let state = worker_run_state(&worker.router).await;
        if state.as_ref().ok() != Some(&crate::provider::ProviderRunState::Running) {
            after_reconnect = Some(state);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let final_task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == running.task_id)
        .unwrap();
    let woken = store
        .agent_inbox(&room, &delegator, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.source_id == agent)
        .count();
    worker.stop().await;
    fixture.stop().await;

    assert_eq!(ended.state, ledger::ExecutionState::Cancelled, "{ended:?}");
    assert!(
        while_offline.as_ref().ok() == Some(&crate::provider::ProviderRunState::Running),
        "the worker's provider keeps running while only its relay is cut: {while_offline:?}"
    );
    assert!(
        after_reconnect.is_some(),
        "the worker run was never cancelled after it reconnected"
    );
    assert_eq!(
        final_task.state,
        ledger::ExecutionState::Cancelled,
        "{final_task:?}"
    );
    assert_eq!(woken, 1, "the delegator is woken once");
}
