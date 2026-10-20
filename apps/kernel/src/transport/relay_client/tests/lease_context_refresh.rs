//! MP-08 / MP-10 / MP-11: synchronous refresh failure ordering and public retry envelope.
//! Real local relay/kernel services with the supplementary managed-dev-stub provider.
use super::support::*;

#[test]
fn review976r2_sync_refresh_keeps_old_binding_when_marker_write_fails() {
    run_async_with_large_test_stack("lease-context-refresh", sync_refresh_async);
}

async fn sync_refresh_async() {
    let _relay_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".into(),
        port: 0,
        shared_token: Some("lease-context-test".into()),
    });
    let listener = server.bind_listener().await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = server.registry();
    let (stop_tx, stop_rx) = oneshot::channel();
    let relay_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = stop_rx.await;
            })
            .await
            .unwrap();
    });
    let mut config = DaemonConfig::for_tests();
    config.daemon_id = "context-refresh-worker".into();
    config.host_machine_id = "context-refresh-machine".into();
    config.relay_url = Some(format!("ws://{addr}"));
    config.relay_token = Some("lease-context-test".into());
    config.accept_remote_leases = true;
    let worker = Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap()));
    let worker_state = worker.lock().await.relay_client_state();
    let (worker_stop_tx, worker_stop_rx) = watch::channel(false);
    let worker_task = tokio::spawn(run_daemon_relay_connector(
        worker.clone(),
        worker_state,
        worker_stop_rx,
    ));
    wait_for_daemon_registration(registry, &config.daemon_id).await;
    let presence = relay_discovery::list_live_kernels_for_machine(&config, &config.host_machine_id)
        .await
        .unwrap()
        .into_iter()
        .find(|k| k.kernel_id == config.daemon_id)
        .unwrap();
    let mut home_config = DaemonConfig::for_tests();
    home_config.relay_url = config.relay_url.clone();
    home_config.relay_token = config.relay_token.clone();
    let mut home = DaemonApp::bootstrap(home_config).unwrap();
    let worktree = crate::test_support::TestWorktree::new("lease-context-refresh");
    let (session, agent) = crate::app::KernelSessionService::new(&mut home)
        .create_session(worktree.session_request().with_agent_defaults(
            crate::session::SessionAgentDefaults {
                provider: "managed-dev-stub".into(),
                model: Some("native-tui-idle".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    let original = crate::agent::RemoteAgentBinding {
        worker_kernel_id: config.daemon_id.clone(),
        worker_machine_id: config.host_machine_id.clone(),
        execution_lease_id: "stale-execution".into(),
        leased_agent_id: "stale-agent".into(),
        active_worker_provider_run_id: None,
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    };
    home.agents()
        .bind_remote_execution(agent.id(), original.clone())
        .unwrap();
    let db = rusqlite::Connection::open(home.operational_history_store().path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_context_marker BEFORE INSERT ON remote_lease_context_pending BEGIN SELECT RAISE(FAIL, 'marker write failed'); END;").unwrap();
    let mut observations = Vec::new();
    for authorized in [false, true] {
        home.agents()
            .bind_remote_execution(agent.id(), original.clone())
            .unwrap();
        let result = if authorized {
            home.refresh_remote_agent_binding_authorized(agent.id(), &|| Ok(()))
        } else {
            home.refresh_remote_agent_binding_to_worker_kernel(agent.id(), &presence)
        };
        println!(
            "synchronous refresh authorized={authorized}: {:?}",
            result.as_ref().err()
        );
        observations.push((
            result.is_err(),
            home.agents()
                .get_agent(agent.id())
                .unwrap()
                .remote_execution()
                .cloned(),
        ));
    }
    db.execute_batch("DROP TRIGGER fail_context_marker")
        .unwrap();
    // Create a genuine owned lease, then lose its volatile worker state like
    // a worker restart. Durable caller receipts must still authorize the retry.
    home.refresh_remote_agent_binding_to_worker_kernel(agent.id(), &presence)
        .unwrap();
    RemoteLeaseRuntime::new(&mut *worker.lock().await).forget_live_leases_for_test();
    home.operational_history_store()
        .append(&crate::history::HistoryEvent::transcript(
            1,
            &crate::history::SessionHistoryEntry::user_prompt(
                session.id(),
                "earlier-client",
                agent.id(),
                "earlier home goal: preserve continuity",
            ),
            crate::history::HistoryEventTurnContext::default(),
        ))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut home)
        .attach(AttachRequest::new(
            session.id(),
            "refresh-client",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let home = Arc::new(Mutex::new(home));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        home.clone(),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    );
    let request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session.id().into(),
        attachment_id: attachment.id().into(),
        prompt: "continue audit".into(),
        target_agent_id: Some(agent.id().into()),
        attachments: Vec::new(),
    });
    let command = KernelCommand::from_local_request("refresh-replay-submit", None, None, &request);
    let response = router.dispatch(command, request).await;
    let mut received = None;
    for _ in 0..1600 {
        let current = home.lock().await.agents().get_agent(agent.id()).unwrap();
        if current
            .remote_execution()
            .is_some_and(|b| b.active_worker_provider_run_id.is_some())
        {
            received = Some(current);
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    println!(
        "public admission ok={}; accepted={}",
        response.is_ok(),
        received.is_some()
    );
    if received.is_none() {
        let events = home
            .lock()
            .await
            .operational_history_store()
            .load_session_events(session.id(), Some(agent.id()))
            .unwrap();
        for event in events.into_iter().filter(|e| {
            matches!(
                e.kind,
                crate::history::HistoryEventKind::ProviderError
                    | crate::history::HistoryEventKind::Notice
            )
        }) {
            println!("retry diagnostic: {:?}", event.content);
        }
    }
    let replay = if let Some(rebound) = received.as_ref() {
        // The binding is rebound before the worker's replayed prompt is registered.
        let leased_agent_id = rebound.remote_execution().unwrap().leased_agent_id.clone();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let replayed = {
                let mut worker = worker.lock().await;
                let leased = RemoteLeaseRuntime::new(&mut worker)
                    .leased_agent_snapshot_for_test(&leased_agent_id)
                    .unwrap();
                worker
                    .prompt_owner_active_prompt_for_agent_snapshot(
                        &leased.backing_session_id,
                        &leased.backing_agent_id,
                    )
                    .unwrap()
                    .is_some_and(|p| {
                        p.hidden_system_context()
                            .contains("earlier home goal: preserve continuity")
                    })
            };
            if replayed || tokio::time::Instant::now() >= deadline {
                break replayed;
            }
            sleep(Duration::from_millis(25)).await;
        }
    } else {
        false
    };
    // The home kernel's background tasks may hold the app lock at this instant.
    let pending = match received.as_ref() {
        Some(a) => Some(
            home.lock()
                .await
                .operational_history_store()
                .remote_lease_context_pending(
                    session.id(),
                    agent.id(),
                    a.remote_execution().unwrap(),
                )
                .unwrap(),
        ),
        None => None,
    };
    let _ = worker_stop_tx.send(true);
    worker_task.await.unwrap();
    worker
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true)
        .unwrap();
    let _ = stop_tx.send(());
    relay_task.await.unwrap();
    for (failed, binding) in observations {
        assert!(failed);
        assert_eq!(
            binding.as_ref(),
            Some(&original),
            "a failed replay marker write must not commit a recreated binding"
        );
    }
    assert!(
        response.is_ok(),
        "public submit admission failed: {response:?}"
    );
    assert!(received.is_some(), "stale lease must refresh and accept");
    assert!(
        replay,
        "refreshed worker must receive the earlier home goal"
    );
    assert_eq!(
        pending,
        Some(false),
        "accepted replay must consume its marker"
    );
}
