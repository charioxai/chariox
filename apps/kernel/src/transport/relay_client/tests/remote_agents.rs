#![allow(unused_imports)]
use super::support::*;

#[tokio::test(flavor = "multi_thread")]
async fn agents_can_be_spawned_on_a_remote_machine_and_cleaned_up() {
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
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
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

    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let worker_kernels =
        relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
            .await
            .expect("worker kernels should be discoverable");
    let provider = worker_kernels
        .first()
        .and_then(|kernel| {
            kernel
                .available_providers
                .iter()
                .find(|provider| provider.as_str() == "managed-dev-stub")
        })
        .cloned()
        .expect("worker should advertise managed-dev-stub");
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");

    let session_id = {
        let mut app = app_home.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        session.id().to_string()
    };

    let remote_agent = {
        let mut app = app_home.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("default")
                    .with_effort("medium")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn")
    };

    let remote_execution = remote_agent
        .remote_execution()
        .cloned()
        .expect("remote binding should be present");
    assert_eq!(remote_execution.worker_kernel_id, config_worker.daemon_id);
    assert_eq!(
        remote_execution.worker_machine_id,
        config_worker.host_machine_id
    );

    {
        let mut app = app_worker.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);
        let worker_agents = app.agents().list_agents();
        assert_eq!(
            worker_agents
                .iter()
                .filter(|agent| agent.is_metaagent())
                .count(),
            0,
            "remote agents start regular until /meta activates temporary meta mode"
        );
    }

    let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
        &config_home,
        ClientTarget {
            daemon_id: Some(config_worker.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::UpdateLeasedAgentMetaMode {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            active: true,
        },
    )
    .await
    .expect("remote meta mode activation should be sent");
    assert!(matches!(
        response,
        RelayPeerResponse::LeasedAgentMetaModeUpdated { .. }
    ));

    {
        let app = app_worker.lock().await;
        let worker_agents = app.agents().list_agents();
        assert_eq!(
            worker_agents
                .iter()
                .filter(|agent| agent.is_metaagent())
                .count(),
            1,
            "remote meta mode update should activate the backing agent"
        );
    }

    let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
        &config_home,
        ClientTarget {
            daemon_id: Some(config_worker.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::UpdateLeasedAgentMetaMode {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            active: false,
        },
    )
    .await
    .expect("remote meta mode deactivation should be sent");
    assert!(matches!(
        response,
        RelayPeerResponse::LeasedAgentMetaModeUpdated { .. }
    ));

    {
        let app = app_worker.lock().await;
        let worker_agents = app.agents().list_agents();
        assert_eq!(
            worker_agents
                .iter()
                .filter(|agent| agent.is_metaagent())
                .count(),
            0,
            "remote meta mode deactivation should restore the backing agent"
        );
    }

    {
        let mut app = app_home.lock().await;
        let destroyed = crate::app::KernelSessionService::new(&mut app)
            .destroy_agent(remote_agent.id())
            .expect("remote agent should destroy");
        assert_eq!(destroyed.id(), remote_agent.id());
    }

    {
        let mut app = app_worker.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 0);
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
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
#[tokio::test(flavor = "multi_thread")]
async fn remote_machine_agents_execute_prompts_through_the_home_session() {
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
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
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

    let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
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

    let (session_id, attachment_id) = {
        let mut app_home = app_home.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app_home)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should attach");
        (session.id().to_string(), attachment.id().to_string())
    };

    let remote_agent_id = {
        let mut app_home = app_home.lock().await;
        crate::app::KernelSessionService::new(&mut app_home)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("default")
                    .with_effort("medium")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn")
            .id()
            .to_string()
    };

    let outcome = app_home
        .lock()
        .await
        .submit_prompt(
            &session_id,
            &attachment_id,
            Some(&remote_agent_id),
            "remote prompt over home session\n",
            Vec::new(),
        )
        .expect("remote prompt should submit");
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));

    let completion = app_home
        .lock()
        .await
        .complete_active_prompt(&session_id, &remote_agent_id, None)
        .expect("remote prompt should complete");
    assert_eq!(completion.completed.target_agent_id(), remote_agent_id);
    assert_eq!(
        completion.completed.prompt(),
        "remote prompt over home session\n"
    );

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
#[tokio::test(flavor = "multi_thread")]
async fn remote_machine_agents_materialize_file_attachments_on_the_worker() {
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
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
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

    let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
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
    let (session_id, attachment_id, remote_agent_id, remote_leased_agent_id) = {
        let mut app_home = app_home.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app_home)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should attach");
        let remote_agent = crate::app::KernelSessionService::new(&mut app_home)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), &provider)
                    .with_alias("remote-reviewer")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn");
        let leased_agent_id = remote_agent
            .remote_execution()
            .expect("remote binding should exist")
            .leased_agent_id
            .clone();
        (
            session.id().to_string(),
            attachment.id().to_string(),
            remote_agent.id().to_string(),
            leased_agent_id,
        )
    };

    let source_path = std::env::temp_dir().join(format!(
        "arroba-remote-attachment-{}.txt",
        crate::session::unix_epoch_ms()
    ));
    std::fs::write(&source_path, b"remote attachment body")
        .expect("source attachment should be written");

    let outcome = app_home
        .lock()
        .await
        .submit_prompt(
            &session_id,
            &attachment_id,
            Some(&remote_agent_id),
            "prompt with attachment\n",
            vec![crate::session::PromptAttachment::new(
                format!("file://{}", source_path.display()),
                "text/plain",
                Some("note.txt".to_string()),
            )],
        )
        .expect("remote prompt should submit");
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));

    let worker_attachments = {
        let mut app = app_worker.lock().await;
        RemoteLeaseRuntime::new(&mut app)
            .leased_agent_active_prompt_attachments(&remote_leased_agent_id)
            .expect("worker prompt attachments should be available")
    };
    assert_eq!(worker_attachments.len(), 1);
    let materialized = &worker_attachments[0];
    assert_eq!(materialized.filename(), Some("note.txt"));
    assert_eq!(materialized.mime(), "text/plain");
    assert!(materialized.url().starts_with("file://"));
    assert_ne!(
        materialized.url(),
        format!("file://{}", source_path.display())
    );
    let worker_path = materialized.url().trim_start_matches("file://");
    let worker_bytes = std::fs::read(worker_path).expect("worker attachment should exist");
    assert_eq!(worker_bytes, b"remote attachment body");

    let _ = std::fs::remove_file(&source_path);
    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
#[tokio::test(flavor = "multi_thread")]
async fn remote_machine_agents_cancel_prompts_through_the_home_session() {
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
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
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

    let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
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
    let (session_id, attachment_id) = {
        let mut app_home = app_home.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app_home)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should attach");
        (session.id().to_string(), attachment.id().to_string())
    };
    let remote_agent_id = {
        let mut app_home = app_home.lock().await;
        crate::app::KernelSessionService::new(&mut app_home)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("default")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn")
            .id()
            .to_string()
    };

    let outcome = app_home
        .lock()
        .await
        .submit_prompt(
            &session_id,
            &attachment_id,
            Some(&remote_agent_id),
            "cancel this remote prompt\n",
            Vec::new(),
        )
        .expect("remote prompt should submit");
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));

    let cancellation = app_home
        .lock()
        .await
        .cancel_active_prompt(&session_id, &attachment_id)
        .expect("remote prompt should cancel");
    assert_eq!(cancellation.prompt.target_agent_id(), remote_agent_id);
    assert_eq!(
        cancellation.prompt.status(),
        crate::session::PromptStatus::Cancelling
    );

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
