#![allow(unused_imports)]
use super::support::*;

#[tokio::test(flavor = "multi_thread")]
async fn incoming_peer_events_project_runtime_to_the_home_session() {
    let _relay_test_guard = relay_client_test_guard().await;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap"),
    ));
    let (session_id, agent_id, attachment_id, daemon_public_key) = {
        let mut app = app.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("attachment should attach");
        (
            session.id().to_string(),
            agent.id().to_string(),
            attachment.id().to_string(),
            relay_crypto::public_key_from_private_key_base64(&app.config().relay_private_key)
                .expect("daemon public key should derive"),
        )
    };
    let sender_private_key = relay_crypto::generate_private_key_base64();
    let plaintext = serde_json::to_vec(&RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id: session_id.clone(),
        home_agent_id: agent_id.clone(),
        provider_run_id: "remote:worker:provider-run-1".to_string(),
        prompts: Vec::new(),
        output_chunks: vec![crate::transport::relay_peer::RelayProjectedOutputChunk {
            kind: crate::terminal::TerminalOutputKind::ProviderOutput,
            merge_key: Some("assistant-1".to_string()),
            bytes: b"remote output".to_vec(),
        }],
        notices: vec!["remote notice".to_string()],
        completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
            message_id: "assistant-msg-1".to_string(),
            completed_at_ms: 1234,
        }],
    })
    .expect("peer event should serialize");
    let encrypted_event =
        relay_crypto::encrypt_payload_for_peer(&sender_private_key, &daemon_public_key, &plaintext)
            .expect("peer event should encrypt");

    let provider_runtime_lanes = {
        let app = app.lock().await;
        app.provider_run_operation_lanes()
    };
    let router = Arc::new(CommandRouter::with_interactive_capacity_and_provider_lanes(
        Arc::clone(&app),
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
        provider_runtime_lanes,
    ));
    handle_daemon_peer_event(&router, encrypted_event)
        .await
        .expect("peer event should project");

    let mut app = app.lock().await;
    let outputs = app
        .terminal_mut()
        .drain_output_records(&session_id, &attachment_id);
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].agent_id.as_deref(), Some(agent_id.as_str()));

    let notices = app
        .terminal_mut()
        .drain_notice_records(&session_id, &attachment_id);
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].agent_id.as_deref(), Some(agent_id.as_str()));

    let completions = app
        .terminal_mut()
        .drain_completion_records(&session_id, &attachment_id);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].agent_id.as_deref(), Some(agent_id.as_str()));
}
#[tokio::test(flavor = "multi_thread")]
async fn forwarded_native_interactions_resolve_back_to_worker_over_temporary_connection() {
    let _relay_test_guard = relay_client_test_guard().await;
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let server = Arc::new(RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    }));
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_home = DaemonConfig::for_tests();
    config_home.daemon_id = "daemon-home".to_string();
    config_home.daemon_alias = Some("home".to_string());
    config_home.host_machine_id = "machine-home".to_string();
    config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));

    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let (home_session_id, home_agent_id) = {
        let mut app = app_home.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        (session.id().to_string(), agent.id().to_string())
    };
    let interaction = crate::session::RuntimeInteraction::new(
        "native-test-interaction",
        "worker-agent",
        crate::session::RuntimeInteractionKind::Permission,
        crate::session::RuntimeInteractionLevel::Warning,
        Some("Synthetic permission".to_string()),
        "Approve synthetic forwarded native interaction?",
        vec![
            crate::session::RuntimeInteractionChoice::new(
                "allow_once",
                "Allow",
                "allowed once",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "deny",
                "Deny",
                "denied",
                Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        None,
        None,
    );
    let context = crate::transport::relay_peer::RemoteNativeInteractionContext {
        home_session_id: home_session_id.clone(),
        home_agent_id: home_agent_id.clone(),
        leased_agent_id: "leased-agent-test".to_string(),
        worker_provider_run_id: "provider-run-test".to_string(),
    };

    let worker_request = {
        let config_worker = config_worker.clone();
        tokio::spawn(async move {
            send_peer_request_via_temporary_connection(
                &config_worker,
                ClientTarget {
                    daemon_id: Some("daemon-home".to_string()),
                    daemon_alias: None,
                },
                RelayPeerRequest::ForwardNativeInteraction {
                    context,
                    interaction,
                },
            )
            .await
        })
    };

    let interaction_id =
        wait_for_active_interaction(Arc::clone(&app_home), &home_session_id, &home_agent_id).await;
    let respond_request =
        crate::local::LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
            session_id: home_session_id.clone(),
            interaction_id,
            choice_id: "allow_once".to_string(),
            custom_reply: None,
        });
    let provider_runtime_lanes = {
        let app = app_home.lock().await;
        app.provider_run_operation_lanes()
    };
    let router = CommandRouter::with_interactive_capacity_and_provider_lanes(
        Arc::clone(&app_home),
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
        provider_runtime_lanes,
    );
    router
        .dispatch(
            KernelCommand::from_local_request("respond-native-test", None, None, &respond_request),
            respond_request,
        )
        .await
        .expect("home interaction response should be accepted");

    let response = worker_request
        .await
        .expect("worker peer request task should join")
        .expect("worker should receive native interaction response");
    match response {
        RelayPeerResponse::NativeInteractionResolved { resolution } => {
            assert_eq!(resolution.status, "answered");
            assert_eq!(resolution.choice_id.as_deref(), Some("allow_once"));
            assert_eq!(resolution.reply.as_deref(), Some("allowed once"));
        }
        other => panic!("unexpected peer response: {other:?}"),
    }

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
