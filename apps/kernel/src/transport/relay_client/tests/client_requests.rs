#![allow(unused_imports)]
use super::support::*;

#[tokio::test(flavor = "multi_thread")]
async fn proxied_session_requests_are_handled_through_relay() {
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

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let created_session_id = {
        let mut app = app.lock().await;
        create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
    };
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut client_socket, _) = connect_async(&url)
        .await
        .expect("client should connect to relay");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let daemon_public_key = expect_client_connected(&mut client_socket).await;

    let list_request_private_key = send_client_request(
        &mut client_socket,
        "list-1",
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::ListSessions(ListSessionsRequest),
    )
    .await;
    let list_response =
        expect_client_response(&mut client_socket, "list-1", &list_request_private_key).await;
    assert!(matches!(
        list_response,
        LocalDaemonResponse::SessionsListed { sessions } if sessions.iter().any(|session| session.id() == created_session_id)
    ));
    assert!(
        app.lock()
            .await
            .session_state_projection_store()
            .has_warmed_list(),
        "relay daemon requests should enter through the command router and warm projections"
    );

    let state_request_private_key = send_client_request(
        &mut client_socket,
        "state-1",
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: created_session_id.clone(),
        }),
    )
    .await;
    let state_response =
        expect_client_response(&mut client_socket, "state-1", &state_request_private_key).await;
    assert!(matches!(
        state_response,
        LocalDaemonResponse::SessionState { session, .. } if session.id() == created_session_id
    ));

    let attach_request_private_key = send_client_request(
        &mut client_socket,
        "attach-1",
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: created_session_id.clone(),
            client_id: "relay-client".to_string(),
            capability_level: ClientCapabilityLevel::MessageTransport,
        }),
    )
    .await;
    let attach_response =
        expect_client_response(&mut client_socket, "attach-1", &attach_request_private_key).await;
    assert!(matches!(
        attach_response,
        LocalDaemonResponse::SessionAttached { attachment } if attachment.session_id() == created_session_id
    ));

    let schema_path = std::env::temp_dir().join(format!(
        "arroba-relay-validate-schema-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &schema_path,
        r#"{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}}"#,
    )
    .expect("schema should write");
    let validate_request_private_key = send_client_request(
        &mut client_socket,
        "validate-1",
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::ValidateWorkflowHandoff(ValidateWorkflowHandoffRequest {
            session_id: created_session_id.clone(),
            handoff_schema_ref: schema_path.display().to_string(),
            handoff_json: r#"{"ok":true}"#.to_string(),
            validation_policy: None,
        }),
    )
    .await;
    let validate_response = expect_client_response(
        &mut client_socket,
        "validate-1",
        &validate_request_private_key,
    )
    .await;
    assert!(matches!(
        validate_response,
        LocalDaemonResponse::WorkflowHandoffValidated {
            valid: true,
            warning: None
        }
    ));

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_client_command_ids_reject_conflicting_retries() {
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

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let created_session_id = {
        let mut app = app.lock().await;
        create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
    };
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut client_socket, _) = connect_async(&url)
        .await
        .expect("client should connect to relay");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let daemon_public_key = expect_client_connected(&mut client_socket).await;
    let command_id = format!("stable-relay-command-{}", std::process::id());

    let list_request_private_key = send_client_command_request(
        &mut client_socket,
        "list-1",
        &command_id,
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::ListSessions(ListSessionsRequest),
    )
    .await;
    let list_response =
        expect_client_response(&mut client_socket, "list-1", &list_request_private_key).await;
    assert!(matches!(
        list_response,
        LocalDaemonResponse::SessionsListed { sessions } if sessions.iter().any(|session| session.id() == created_session_id)
    ));

    let _state_private_key = send_client_command_request(
        &mut client_socket,
        "state-1",
        &command_id,
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: created_session_id.clone(),
        }),
    )
    .await;
    let error = expect_client_response_error(&mut client_socket, "state-1").await;
    assert_eq!(error.code, "duplicate_command_conflict");
    assert!(!error.retryable);

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn interactive_session_requests_are_handled_through_relay() {
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

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let (created_session_id, default_agent_id) = {
        let mut app = app.lock().await;
        create_test_session_with_alias(
            &mut app,
            "workspace-relay-test",
            "worktree-relay-test",
            "main",
        )
    };
    let attachment_id = {
        let mut app = app.lock().await;
        attach_test_client(
            &mut app,
            &created_session_id,
            "relay-client",
            ClientCapabilityLevel::MessageTransport,
        )
    };
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut client_socket, _) = connect_async(&url)
        .await
        .expect("client should connect to relay");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let daemon_public_key = expect_client_connected(&mut client_socket).await;

    let resolve_private_key = send_client_request(
        &mut client_socket,
        "resolve-1",
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "main".to_string(),
            workspace_id: Some("workspace-relay-test".to_string()),
        }),
    )
    .await;
    let resolve_response =
        expect_client_response(&mut client_socket, "resolve-1", &resolve_private_key).await;
    assert!(matches!(
        resolve_response,
        LocalDaemonResponse::SessionResolved { session } if session.id() == created_session_id
    ));

    let focus_private_key = send_client_request(
        &mut client_socket,
        "focus-1",
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: created_session_id.clone(),
            agent_id: default_agent_id.clone(),
        }),
    )
    .await;
    let focus_response =
        expect_client_response(&mut client_socket, "focus-1", &focus_private_key).await;
    assert!(matches!(
        focus_response,
        LocalDaemonResponse::AgentFocused { agent } if agent.id() == default_agent_id
    ));

    let config_private_key = send_client_request(
        &mut client_socket,
        "config-1",
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
            session_id: created_session_id.clone(),
            attachment_id: attachment_id.clone(),
            values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
            requires_idle: false,
        }),
    )
    .await;
    let config_response =
        expect_client_response(&mut client_socket, "config-1", &config_private_key).await;
    assert!(matches!(
        config_response,
        LocalDaemonResponse::SessionConfigUpdated { config, .. }
            if config.values().get("theme").map(String::as_str) == Some("compact")
    ));

    let detach_private_key = send_client_request(
        &mut client_socket,
        "detach-1",
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::DetachFromSession(DetachFromSessionRequest {
            attachment_id: attachment_id.clone(),
        }),
    )
    .await;
    let detach_response =
        expect_client_response(&mut client_socket, "detach-1", &detach_private_key).await;
    assert!(matches!(
        detach_response,
        LocalDaemonResponse::SessionDetached { attachment } if attachment.id() == attachment_id
    ));

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
#[tokio::test(flavor = "multi_thread")]
async fn terminal_resize_errors_are_returned_through_relay() {
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

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let created_session_id = {
        let mut app = app.lock().await;
        create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
    };
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut client_socket, _) = connect_async(&url)
        .await
        .expect("client should connect to relay");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let daemon_public_key = expect_client_connected(&mut client_socket).await;

    let resize_private_key = send_client_request(
        &mut client_socket,
        "resize-1",
        &config.daemon_id,
        &daemon_public_key,
        LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: created_session_id,
            cols: 120,
            rows: 40,
        }),
    )
    .await;
    let resize_error =
        expect_client_error(&mut client_socket, "resize-1", &resize_private_key).await;
    assert_eq!(resize_error.code, "no_active_provider_run");

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
