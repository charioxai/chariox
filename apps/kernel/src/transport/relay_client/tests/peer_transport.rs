#![allow(unused_imports)]
use super::support::*;

#[tokio::test(flavor = "multi_thread")]
async fn proxied_peer_requests_are_handled_through_relay() {
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
    drop(listener);

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
                .run_until(async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_a = DaemonConfig::for_tests();
    config_a.daemon_id = "daemon-a".to_string();
    config_a.daemon_alias = Some("alpha".to_string());
    config_a.host_machine_id = "machine-a".to_string();
    config_a.host_machine_alias = Some("machine-alpha".to_string());
    config_a.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_a.relay_token = Some("secret".to_string());
    config_a.relay_heartbeat_ms = 50;
    let app_a = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_a.clone()).expect("daemon A should bootstrap"),
    ));
    let state_a = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_a_tx, shutdown_a_rx) = watch::channel(false);
    let connector_a = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_a),
        Arc::clone(&state_a),
        shutdown_a_rx,
    ));

    let mut config_b = DaemonConfig::for_tests();
    config_b.daemon_id = "daemon-b".to_string();
    config_b.daemon_alias = Some("beta".to_string());
    config_b.host_machine_id = "machine-b".to_string();
    config_b.host_machine_alias = Some("machine-beta".to_string());
    config_b.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_b.relay_token = Some("secret".to_string());
    config_b.relay_heartbeat_ms = 50;
    let app_b = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_b.clone()).expect("daemon B should bootstrap"),
    ));
    let state_b = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_b_tx, shutdown_b_rx) = watch::channel(false);
    let connector_b = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_b),
        Arc::clone(&state_b),
        shutdown_b_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_a.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_b.daemon_id).await;

    let kernel = relay_discovery::get_live_kernel(&config_a, "beta")
        .await
        .expect("live kernel lookup should succeed");
    assert_eq!(kernel.kernel_id, config_b.daemon_id);
    assert_eq!(kernel.public_key, config_b.relay_public_key);

    let response = send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: None,
            daemon_alias: Some("beta".to_string()),
        },
        RelayPeerRequest::Ping {
            value: "hello-remote-kernel".to_string(),
        },
    )
    .await
    .expect("peer request should succeed");
    assert_eq!(
        response,
        RelayPeerResponse::Pong {
            value: "hello-remote-kernel".to_string(),
            daemon_id: config_b.daemon_id.clone(),
        }
    );

    let _ = shutdown_a_tx.send(true);
    let _ = shutdown_b_tx.send(true);
    connector_a.await.expect("connector A should join");
    connector_b.await.expect("connector B should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
#[tokio::test(flavor = "multi_thread")]
async fn execution_leases_are_managed_through_peer_transport() {
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
    drop(listener);

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
                .run_until(async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_a = DaemonConfig::for_tests();
    config_a.daemon_id = "daemon-home".to_string();
    config_a.daemon_alias = Some("home".to_string());
    config_a.host_machine_id = "machine-home".to_string();
    config_a.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_a.relay_token = Some("secret".to_string());
    config_a.relay_heartbeat_ms = 50;
    let app_a = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_a.clone()).expect("home daemon should bootstrap"),
    ));
    let state_a = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_a_tx, shutdown_a_rx) = watch::channel(false);
    let connector_a = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_a),
        Arc::clone(&state_a),
        shutdown_a_rx,
    ));

    let mut config_b = DaemonConfig::for_tests();
    config_b.daemon_id = "daemon-worker".to_string();
    config_b.daemon_alias = Some("worker".to_string());
    config_b.host_machine_id = "machine-worker".to_string();
    config_b.host_machine_alias = Some("remote-builder".to_string());
    config_b.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_b.relay_token = Some("secret".to_string());
    config_b.relay_heartbeat_ms = 50;
    config_b.accept_remote_leases = true;
    let app_b = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_b.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_b = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_b_tx, shutdown_b_rx) = watch::channel(false);
    let connector_b = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_b),
        Arc::clone(&state_b),
        shutdown_b_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_a.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_b.daemon_id).await;

    let created = send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: None,
            daemon_alias: Some("worker".to_string()),
        },
        RelayPeerRequest::CreateExecutionLease {
            home_kernel_id: config_a.daemon_id.clone(),
            home_session_id: "session-remote-1".to_string(),
            home_agent_id: "agent-remote-1".to_string(),
            owner_user_id: "user-home".to_string(),
        },
    )
    .await
    .expect("execution lease should be created remotely");
    let lease = match created {
        RelayPeerResponse::ExecutionLeaseCreated { lease } => lease,
        other => panic!("unexpected peer response: {other:?}"),
    };
    assert_eq!(lease.home_kernel_id, config_a.daemon_id);
    assert_eq!(lease.worker_kernel_id, config_b.daemon_id);
    assert_eq!(lease.machine_id, config_b.host_machine_id);
    {
        let mut app = app_b.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);
    }

    let destroyed = send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: Some(config_b.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::DestroyExecutionLease {
            lease_id: lease.id.clone(),
        },
    )
    .await
    .expect("execution lease should be destroyed remotely");
    assert_eq!(
        destroyed,
        RelayPeerResponse::ExecutionLeaseDestroyed {
            lease_id: lease.id.clone(),
        }
    );
    {
        let mut app = app_b.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 0);
    }

    let _ = shutdown_a_tx.send(true);
    let _ = shutdown_b_tx.send(true);
    connector_a.await.expect("connector A should join");
    connector_b.await.expect("connector B should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
#[tokio::test(flavor = "multi_thread")]
async fn leased_agents_are_spawned_and_destroyed_through_peer_transport() {
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
    drop(listener);

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
                .run_until(async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_a = DaemonConfig::for_tests();
    config_a.daemon_id = "daemon-home".to_string();
    config_a.daemon_alias = Some("home".to_string());
    config_a.host_machine_id = "machine-home".to_string();
    config_a.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_a.relay_token = Some("secret".to_string());
    config_a.relay_heartbeat_ms = 50;
    let app_a = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_a.clone()).expect("home daemon should bootstrap"),
    ));
    let (home_session_id, home_agent_id) = {
        let mut app = app_a.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        (session.id().to_string(), agent.id().to_string())
    };
    let state_a = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_a_tx, shutdown_a_rx) = watch::channel(false);
    let connector_a = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_a),
        Arc::clone(&state_a),
        shutdown_a_rx,
    ));

    let mut config_b = DaemonConfig::for_tests();
    config_b.daemon_id = "daemon-worker".to_string();
    config_b.daemon_alias = Some("worker".to_string());
    config_b.host_machine_id = "machine-worker".to_string();
    config_b.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_b.relay_token = Some("secret".to_string());
    config_b.relay_heartbeat_ms = 50;
    config_b.accept_remote_leases = true;
    let app_b = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_b.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_b = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_b_tx, shutdown_b_rx) = watch::channel(false);
    let connector_b = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_b),
        Arc::clone(&state_b),
        shutdown_b_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_a.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_b.daemon_id).await;

    let lease = match send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: Some(config_b.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::CreateExecutionLease {
            home_kernel_id: config_a.daemon_id.clone(),
            home_session_id: home_session_id.clone(),
            home_agent_id: home_agent_id.clone(),
            owner_user_id: "user-home".to_string(),
        },
    )
    .await
    .expect("execution lease should be created remotely")
    {
        RelayPeerResponse::ExecutionLeaseCreated { lease } => lease,
        other => panic!("unexpected peer response: {other:?}"),
    };

    let leased_agent = match send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: None,
            daemon_alias: Some("worker".to_string()),
        },
        RelayPeerRequest::SpawnLeasedAgent {
            lease_id: lease.id.clone(),
            provider: "opencode".to_string(),
            model: Some("kimi2.5".to_string()),
            effort: Some("medium".to_string()),
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            worktree_placement: None,
        },
    )
    .await
    .expect("leased agent should be spawned remotely")
    {
        RelayPeerResponse::LeasedAgentSpawned { leased_agent } => leased_agent,
        other => panic!("unexpected peer response: {other:?}"),
    };
    assert_eq!(leased_agent.lease_id, lease.id);
    assert_eq!(leased_agent.provider, "opencode");
    {
        let mut app = app_b.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);
    }

    let destroyed = send_peer_request_via_relay(
        &app_a,
        &state_a,
        ClientTarget {
            daemon_id: Some(config_b.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::DestroyLeasedAgent {
            leased_agent_id: leased_agent.id.clone(),
        },
    )
    .await
    .expect("leased agent should be destroyed remotely");
    assert_eq!(
        destroyed,
        RelayPeerResponse::LeasedAgentDestroyed {
            leased_agent_id: leased_agent.id.clone(),
        }
    );
    {
        let mut app = app_b.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
    }

    let _ = shutdown_a_tx.send(true);
    let _ = shutdown_b_tx.send(true);
    connector_a.await.expect("connector A should join");
    connector_b.await.expect("connector B should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
