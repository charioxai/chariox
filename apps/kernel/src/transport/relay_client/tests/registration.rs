#![allow(unused_imports)]
use super::support::*;

#[tokio::test(flavor = "multi_thread")]
async fn daemon_connector_registers_with_relay() {
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
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

    {
        let guard = registry.read().await;
        assert!(guard.daemon(&config.daemon_id).is_some());
    }
    assert!(state.read().await.connected);

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
    sleep(Duration::from_millis(50)).await;
    {
        let guard = registry.read().await;
        assert!(guard.daemon(&config.daemon_id).is_none());
    }
    assert!(!state.read().await.connected);

    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

// A second kernel holding this kernel's relay token and public key, but not its private key,
// is refused by the relay and never displaces it; the real kernel still reconnects.
#[tokio::test(flavor = "multi_thread")]
async fn impersonator_with_the_relay_token_but_not_the_identity_key_cannot_displace_the_kernel() {
    let _relay_test_guard = relay_client_test_guard().await;
    let server = Arc::new(RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    }));
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
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
    let start_connector = |config: DaemonConfig| {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config).expect("daemon should bootstrap"),
        ));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(run_daemon_relay_connector(
            app,
            Arc::clone(&state),
            shutdown_rx,
        ));
        (state, shutdown_tx, task)
    };

    let mut kernel = DaemonConfig::for_tests();
    kernel.relay_url = Some(format!("ws://{addr}"));
    kernel.relay_token = Some("secret".to_string());
    kernel.relay_heartbeat_ms = 50;
    let (kernel_state, kernel_shutdown, kernel_task) = start_connector(kernel.clone());
    wait_for_daemon_registration(registry.clone(), &kernel.daemon_id).await;

    let mut impersonator = DaemonConfig::for_tests();
    impersonator.daemon_id = kernel.daemon_id.clone();
    impersonator.host_machine_id = "impersonator-machine".to_string();
    impersonator.relay_url = kernel.relay_url.clone();
    impersonator.relay_token = kernel.relay_token.clone();
    impersonator.relay_public_key = kernel.relay_public_key.clone();
    impersonator.relay_heartbeat_ms = 50;
    assert_ne!(impersonator.relay_private_key, kernel.relay_private_key);
    let (_, impersonator_shutdown, impersonator_task) = start_connector(impersonator);

    // The impersonator's connector keeps retrying; the kernel must hold its registration throughout.
    for _ in 0..60 {
        sleep(Duration::from_millis(25)).await;
        let registered_machine = registry
            .read()
            .await
            .daemon(&kernel.daemon_id)
            .map(|registration| registration.machine_id.clone());
        assert_eq!(
            registered_machine.as_deref(),
            Some(kernel.host_machine_id.as_str()),
            "the impersonator took the kernel's relay registration"
        );
        assert!(
            kernel_state.read().await.connected,
            "the kernel was displaced"
        );
    }
    let _ = impersonator_shutdown.send(true);
    impersonator_task
        .await
        .expect("impersonator connector should join");

    let _ = kernel_shutdown.send(true);
    kernel_task.await.expect("kernel connector should join");
    sleep(Duration::from_millis(50)).await;
    assert!(registry.read().await.daemon(&kernel.daemon_id).is_none());
    let (_, kernel_shutdown, kernel_task) = start_connector(kernel.clone());
    wait_for_daemon_registration(registry.clone(), &kernel.daemon_id).await;
    let _ = kernel_shutdown.send(true);
    kernel_task.await.expect("kernel connector should join");

    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
