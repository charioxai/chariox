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

// A kernel whose network dropped silently registers again on a new socket while
// the relay still routes to the old one. Requests already sent down the old
// socket are lost: the client must hear so at once instead of at its timeout.
#[tokio::test(flavor = "multi_thread")]
async fn reregistering_daemon_fails_requests_sent_to_its_stale_socket() {
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
    let register = Message::Text(
        serde_json::to_string(&RelayEnvelope::DaemonRegister {
            registration: test_registration("daemon-1", "machine-1", "Linux", 10),
        })
        .expect("register envelope should serialize")
        .into(),
    );
    let (mut stale_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("first daemon socket should connect");
    stale_socket
        .send(register.clone())
        .await
        .expect("first register should send");
    sleep(Duration::from_millis(50)).await;

    let (mut client_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("client should connect");
    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
            })
            .expect("client connect should serialize")
            .into(),
        ))
        .await
        .expect("client connect should send");
    match client_socket.next().await {
        Some(Ok(Message::Text(text))) => assert!(matches!(
            serde_json::from_str::<RelayEnvelope>(&text).expect("connect should decode"),
            RelayEnvelope::ClientConnected { .. }
        )),
        other => panic!("unexpected client connect response: {other:?}"),
    }
    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientRequest {
                request_id: "sent-before-reregistration".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
                encrypted_request: EncryptedRelayPayload {
                    sender_public_key: "client-public".to_string(),
                    nonce: "nonce".to_string(),
                    ciphertext: "ciphertext".to_string(),
                },
            })
            .expect("client request should serialize")
            .into(),
        ))
        .await
        .expect("client request should send");
    match stale_socket.next().await {
        Some(Ok(Message::Text(text))) => assert!(matches!(
            serde_json::from_str::<RelayEnvelope>(&text).expect("daemon request should decode"),
            RelayEnvelope::DaemonRequest { .. }
        )),
        other => panic!("unexpected daemon request frame: {other:?}"),
    }

    // Keep a full old display queue alive without ever polling its receiver.
    let (display_tx, _stalled_display_rx) = tokio::sync::mpsc::channel(1);
    display_tx
        .try_send(crate::registry::DisplayStreamEvent::Chunk {
            data: "pending".to_string(),
            message_kind: None,
        })
        .unwrap();
    registry.write().await.insert_pending_display_stream(
        "stalled-old-display".to_string(),
        crate::registry::DaemonKey {
            realm_id: crate::auth::DEFAULT_RELAY_REALM_ID.to_string(),
            daemon_id: "daemon-1".to_string(),
        },
        display_tx,
    );

    let (mut fresh_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("second daemon socket should connect");
    fresh_socket
        .send(register)
        .await
        .expect("second register should send");

    match timeout(Duration::from_secs(1), client_socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("client response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "sent-before-reregistration");
                assert_eq!(error.code, "target_disconnected");
                assert!(error.retryable);
            }
            other => panic!("unexpected client envelope: {other:?}"),
        },
        other => panic!("request on the stale socket was never failed: {other:?}"),
    }
    assert_eq!(registry.read().await.pending_request_count(), 0);
    assert!(registry.read().await.daemon("daemon-1").is_some());

    // This must be read by the replacement socket's reader even with the old
    // display receiver still alive and its queue full.
    fresh_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonHeartbeat {
                daemon_id: "daemon-1".to_string(),
                registration: Some(test_registration("daemon-1", "machine-1", "recovered", 10)),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    timeout(Duration::from_millis(500), async {
        loop {
            if registry
                .read()
                .await
                .daemon("daemon-1")
                .is_some_and(|daemon| daemon.os_name.as_deref() == Some("recovered"))
            {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("new daemon reader waited on the old display queue");

    let _ = client_socket.close(None).await;
    let _ = fresh_socket.close(None).await;
    drop(stale_socket);
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
