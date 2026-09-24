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
        "chariox-relay-validate-schema-{}.json",
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

#[test]
fn authenticated_public_client_preserves_worker_relay_retryability() {
    run_async_with_large_test_stack(
        "public-worker-relay-retryability",
        authenticated_public_client_preserves_worker_relay_retryability_async,
    );
}

async fn authenticated_public_client_preserves_worker_relay_retryability_async() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _test_home = RelayTestHome::new();
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
    let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());
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
    config_home.daemon_id = "daemon-public-retry-home".to_string();
    config_home.daemon_alias = Some("public-retry-home".to_string());
    config_home.host_machine_id = "machine-public-retry-home".to_string();
    config_home.relay_url = Some(relay_url.clone());
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-public-retry-worker".to_string();
    config_worker.daemon_alias = Some("public-retry-worker".to_string());
    config_worker.host_machine_id = "machine-public-retry-worker".to_string();
    config_worker.host_machine_alias = Some("public-retry-worker".to_string());
    config_worker.relay_url = Some(relay_url.clone());
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.accept_remote_leases = true;
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
    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let provider = relay_discovery::list_live_kernels_for_machine(
        &config_home,
        config_worker
            .host_machine_alias
            .as_deref()
            .expect("worker machine alias should be configured"),
    )
    .await
    .expect("worker kernels should be discoverable")
    .first()
    .and_then(|kernel| {
        kernel
            .available_providers
            .iter()
            .find(|provider| provider.as_str() == "managed-dev-stub")
    })
    .cloned()
    .expect("worker should advertise managed-dev-stub");

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
    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");

    let session_id = {
        let mut app = app_home.lock().await;
        create_test_session(&mut app, "workspace-public-retry", "worktree-public-retry")
    };
    let remote_agent_id = {
        let mut app = app_home.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("public-retry-agent")
                    .with_model("default")
                    .with_effort("medium")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn")
            .id()
            .to_string()
    };
    let leased_agent_id = app_home
        .lock()
        .await
        .agents()
        .get_agent(&remote_agent_id)
        .expect("remote agent should remain available")
        .remote_execution()
        .expect("remote binding should remain available")
        .leased_agent_id
        .clone();

    let destroyed = crate::transport::relay_client::send_peer_request_via_temporary_connection(
        &config_home,
        ClientTarget {
            daemon_id: Some(config_worker.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::DestroyLeasedAgent {
            leased_agent_id: leased_agent_id.clone(),
        },
    )
    .await
    .expect("worker-side lease loss should be controllable through peer transport");
    assert_eq!(
        destroyed,
        RelayPeerResponse::LeasedAgentDestroyed { leased_agent_id }
    );

    let (mut client_socket, _) = connect_async(&relay_url)
        .await
        .expect("public client should connect to relay");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config_home.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let daemon_public_key = expect_client_connected(&mut client_socket).await;

    let launch_request = || {
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(remote_agent_id.clone()),
            adapter_key: crate::provider::adapter_key_for_provider(&provider).to_string(),
            provider: provider.clone(),
            account_profile: "default".to_string(),
            model: "default".to_string(),
            variant: Some("medium".to_string()),
            structured_endpoint: None,
            provider_session_id: None,
            native_tui: true,
        })
    };

    let _worker_error_private_key = send_client_request(
        &mut client_socket,
        "worker-business-error-1",
        &config_home.daemon_id,
        &daemon_public_key,
        launch_request(),
    )
    .await;
    let worker_error =
        expect_client_response_error(&mut client_socket, "worker-business-error-1").await;
    assert_eq!(
        worker_error.code, "local_transport_error",
        "stable client transport code should remain unchanged: {worker_error:?}"
    );
    assert!(
        worker_error.message.contains("leased_agent_not_found"),
        "worker-authoritative diagnostic should survive the client boundary: {worker_error:?}"
    );
    assert!(
        !worker_error.retryable,
        "worker-authoritative business rejection must not be projected as retryable: {worker_error:?}"
    );

    let _ = shutdown_worker_tx.send(true);
    connector_worker
        .await
        .expect("worker connector should join");
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon(&config_worker.daemon_id)
            .is_none()
        {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    assert!(
        registry
            .read()
            .await
            .daemon(&config_worker.daemon_id)
            .is_none(),
        "worker should be absent before the transient transport control"
    );

    let _transient_private_key = send_client_request(
        &mut client_socket,
        "relay-disconnect-1",
        &config_home.daemon_id,
        &daemon_public_key,
        launch_request(),
    )
    .await;
    let transient_error =
        expect_client_response_error(&mut client_socket, "relay-disconnect-1").await;
    assert_eq!(
        transient_error.code, "local_transport_error",
        "stable client transport code should remain unchanged: {transient_error:?}"
    );
    assert!(
        transient_error.retryable,
        "an unavailable relay target remains retryable: {transient_error:?}"
    );

    let _ = shutdown_home_tx.send(true);
    connector_home.await.expect("home connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("relay server should join");
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
            provider_run_id: None,
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
