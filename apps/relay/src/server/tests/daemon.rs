use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn daemon_registration_is_tracked_and_removed_on_disconnect() {
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

    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let registry = server.registry();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("relay server should run");
    });

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut socket, _) = connect_async(&url)
        .await
        .expect("client should connect to relay");
    let register = RelayEnvelope::DaemonRegister {
        registration: DaemonRegistration {
            auth_token: "secret".to_string(),
            daemon_id: "daemon-1".to_string(),
            machine_id: "machine-1".to_string(),
            machine_alias: Some("workstation".to_string()),
            os_name: Some("macOS".to_string()),
            kernel_started_at_ms: 10,
            daemon_alias: Some("mbp".to_string()),
            kernel_alias: Some("default".to_string()),
            public_key: "public-key".to_string(),
            capabilities: vec!["kernel_ws".to_string()],
            available_providers: vec!["opencode".to_string()],
            provider_accounts: Vec::new(),
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 1,
        },
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&register)
                .expect("register envelope should serialize")
                .into(),
        ))
        .await
        .expect("register frame should send");
    sleep(Duration::from_millis(50)).await;

    {
        let guard = registry.read().await;
        assert_eq!(guard.daemon_count(), 1);
        assert!(guard.daemon("daemon-1").is_some());
    }

    socket.close(None).await.expect("socket should close");
    sleep(Duration::from_millis(50)).await;

    {
        let guard = registry.read().await;
        assert_eq!(guard.daemon_count(), 0);
    }

    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnecting_daemon_replaces_stale_socket_without_removing_live_registration() {
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

    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let registry = server.registry();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("relay server should run");
    });

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut first_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("first daemon should connect to relay");
    let register = RelayEnvelope::DaemonRegister {
        registration: test_registration("daemon-1", "machine-1", "macOS", 10),
    };
    first_socket
        .send(Message::Text(
            serde_json::to_string(&register)
                .expect("register envelope should serialize")
                .into(),
        ))
        .await
        .expect("first register frame should send");
    sleep(Duration::from_millis(50)).await;

    let (mut second_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("second daemon should connect to relay");
    second_socket
        .send(Message::Text(
            serde_json::to_string(&register)
                .expect("register envelope should serialize")
                .into(),
        ))
        .await
        .expect("second register frame should send");
    sleep(Duration::from_millis(50)).await;

    {
        let guard = registry.read().await;
        assert_eq!(guard.daemon_count(), 1);
        assert_eq!(guard.peer_count(), 1);
        assert!(guard.daemon("daemon-1").is_some());
    }

    first_socket
        .close(None)
        .await
        .expect("first socket should close");
    sleep(Duration::from_millis(50)).await;

    {
        let guard = registry.read().await;
        assert_eq!(guard.daemon_count(), 1);
        assert_eq!(guard.peer_count(), 1);
        assert!(guard.daemon("daemon-1").is_some());
    }

    second_socket
        .close(None)
        .await
        .expect("second socket should close");
    sleep(Duration::from_millis(50)).await;

    {
        let guard = registry.read().await;
        assert_eq!(guard.daemon_count(), 0);
        assert_eq!(guard.peer_count(), 0);
    }

    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_socket_cannot_switch_registered_identity() {
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

    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let registry = server.registry();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("relay server should run");
    });

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut socket, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon should connect to relay");
    let first_registration = test_registration("daemon-1", "machine-1", "macOS", 10);
    socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: first_registration,
            })
            .expect("first register should serialize")
            .into(),
        ))
        .await
        .expect("first register should send");
    sleep(Duration::from_millis(50)).await;

    let mut refreshed_registration = test_registration("daemon-1", "machine-1", "macOS", 20);
    refreshed_registration.public_key = "refreshed-public-key".to_string();
    socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: refreshed_registration,
            })
            .expect("refresh register should serialize")
            .into(),
        ))
        .await
        .expect("refresh register should send");
    sleep(Duration::from_millis(50)).await;
    assert_no_relay_close(&mut socket).await;

    {
        let guard = registry.read().await;
        assert_eq!(guard.daemon_count(), 1);
        assert_eq!(guard.peer_count(), 1);
        assert_eq!(
            guard
                .daemon("daemon-1")
                .map(|registration| registration.public_key.as_str()),
            Some("refreshed-public-key")
        );
    }

    socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration("daemon-2", "machine-1", "macOS", 30),
            })
            .expect("identity switch register should serialize")
            .into(),
        ))
        .await
        .expect("identity switch register should send");

    let close_payload = match timeout(Duration::from_millis(500), socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => String::new().into(),
        Ok(other) => panic!("unexpected identity switch response: {other:?}"),
        Err(_) => panic!("identity switch did not close promptly"),
    };
    if !close_payload.is_empty() {
        match serde_json::from_str::<RelayEnvelope>(&close_payload)
            .expect("relay close should decode")
        {
            RelayEnvelope::Close { reason } => {
                assert_eq!(reason, "daemon connection already registered");
            }
            other => panic!("unexpected identity switch envelope: {other:?}"),
        }
    }
    sleep(Duration::from_millis(50)).await;
    {
        let guard = registry.read().await;
        assert_eq!(guard.daemon_count(), 0);
        assert_eq!(guard.peer_count(), 0);
        assert!(guard.daemon("daemon-1").is_none());
        assert!(guard.daemon("daemon-2").is_none());
    }

    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
