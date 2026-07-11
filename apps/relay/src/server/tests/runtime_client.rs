use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn relay_completes_client_websocket_close_handshake() {
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
    let (mut client_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("client should connect");
    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientMetadataRequest {
                request_id: "close-handshake".to_string(),
                auth_token: "secret".to_string(),
                query: RelayMetadataQuery::ListLiveMachines,
            })
            .expect("metadata request should serialize")
            .into(),
        ))
        .await
        .expect("metadata request should send");
    assert!(matches!(
        client_socket.next().await,
        Some(Ok(Message::Text(_)))
    ));

    client_socket
        .close(None)
        .await
        .expect("client close should send");
    let close = timeout(Duration::from_millis(500), client_socket.next())
        .await
        .expect("relay should answer close promptly");
    assert!(
        matches!(close, Some(Ok(Message::Close(_)))),
        "relay should complete the websocket close handshake, received {close:?}"
    );

    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_client_frames_require_accepted_client_connect() {
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
    let (mut daemon_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon should connect");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration("daemon-1", "machine-1", "Linux", 10),
            })
            .expect("register should serialize")
            .into(),
        ))
        .await
        .expect("register should send");
    sleep(Duration::from_millis(50)).await;

    let (mut client_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("client should connect");
    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientRequest {
                request_id: "request-before-connect".to_string(),
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

    let close_payload = match timeout(Duration::from_millis(500), client_socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => String::new().into(),
        Ok(other) => panic!("unexpected pre-connect client response: {other:?}"),
        Err(_) => panic!("pre-connect client request did not close promptly"),
    };
    if !close_payload.is_empty() {
        match serde_json::from_str::<RelayEnvelope>(&close_payload)
            .expect("relay close should decode")
        {
            RelayEnvelope::Close { reason } => {
                assert_eq!(reason, "client must connect before sending requests");
            }
            other => panic!("unexpected pre-connect response envelope: {other:?}"),
        }
    }
    match timeout(Duration::from_millis(100), daemon_socket.next()).await {
        Err(_) => {}
        Ok(other) => panic!("pre-connect request reached daemon: {other:?}"),
    }
    assert_eq!(registry.read().await.pending_request_count(), 0);

    let _ = daemon_socket.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_client_frames_reject_empty_identifiers_without_pending_state() {
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
    let (mut daemon_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon should connect");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration("daemon-1", "machine-1", "Linux", 10),
            })
            .expect("register should serialize")
            .into(),
        ))
        .await
        .expect("register should send");
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

    let encrypted_request = EncryptedRelayPayload {
        sender_public_key: "client-public".to_string(),
        nonce: "nonce".to_string(),
        ciphertext: "ciphertext".to_string(),
    };
    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientRequest {
                request_id: "   ".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
                encrypted_request: encrypted_request.clone(),
            })
            .expect("invalid request should serialize")
            .into(),
        ))
        .await
        .expect("invalid request should send");
    match timeout(Duration::from_millis(500), client_socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("invalid request response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "   ");
                assert_eq!(error.code, "invalid_runtime_identifier");
                assert!(!error.retryable);
            }
            other => panic!("unexpected invalid request response: {other:?}"),
        },
        Ok(other) => panic!("unexpected invalid request frame: {other:?}"),
        Err(_) => panic!("invalid request response was not delivered"),
    }
    match timeout(Duration::from_millis(100), daemon_socket.next()).await {
        Err(_) => {}
        Ok(other) => panic!("invalid request reached daemon: {other:?}"),
    }
    assert_eq!(registry.read().await.pending_request_count(), 0);

    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                request_id: "invalid-subscription".to_string(),
                subscription_id: "\t".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
                session_id: "session-1".to_string(),
                attachment_id: "terminal".to_string(),
                client_public_key: "client-public".to_string(),
                subscription_scope: None,
                resume_from_event_id: None,
            })
            .expect("invalid subscribe should serialize")
            .into(),
        ))
        .await
        .expect("invalid subscribe should send");
    match timeout(Duration::from_millis(500), client_socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("invalid subscribe response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "invalid-subscription");
                assert_eq!(error.code, "invalid_runtime_identifier");
                assert_eq!(error.message, "subscription_id must not be empty");
                assert!(!error.retryable);
            }
            other => panic!("unexpected invalid subscribe response: {other:?}"),
        },
        Ok(other) => panic!("unexpected invalid subscribe frame: {other:?}"),
        Err(_) => panic!("invalid subscribe response was not delivered"),
    }
    match timeout(Duration::from_millis(100), daemon_socket.next()).await {
        Err(_) => {}
        Ok(other) => panic!("invalid subscribe reached daemon: {other:?}"),
    }
    {
        let guard = registry.read().await;
        assert_eq!(guard.pending_request_count(), 0);
        assert_eq!(guard.subscription_count(), 0);
    }

    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientUnsubscribe {
                request_id: "invalid-unsubscribe".to_string(),
                subscription_id: "".to_string(),
                client_public_key: "client-public".to_string(),
            })
            .expect("invalid unsubscribe should serialize")
            .into(),
        ))
        .await
        .expect("invalid unsubscribe should send");
    match timeout(Duration::from_millis(500), client_socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("invalid unsubscribe response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "invalid-unsubscribe");
                assert_eq!(error.code, "invalid_runtime_identifier");
                assert_eq!(error.message, "subscription_id must not be empty");
                assert!(!error.retryable);
            }
            other => panic!("unexpected invalid unsubscribe response: {other:?}"),
        },
        Ok(other) => panic!("unexpected invalid unsubscribe frame: {other:?}"),
        Err(_) => panic!("invalid unsubscribe response was not delivered"),
    }
    match timeout(Duration::from_millis(100), daemon_socket.next()).await {
        Err(_) => {}
        Ok(other) => panic!("invalid unsubscribe reached daemon: {other:?}"),
    }
    {
        let guard = registry.read().await;
        assert_eq!(guard.pending_request_count(), 0);
        assert_eq!(guard.subscription_count(), 0);
    }

    let _ = client_socket.close(None).await;
    let _ = daemon_socket.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_client_frames_must_match_connected_target() {
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
    let (mut daemon_a, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon A should connect");
    let (mut daemon_b, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon B should connect");
    for (socket, daemon_id) in [(&mut daemon_a, "daemon-a"), (&mut daemon_b, "daemon-b")] {
        socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration(daemon_id, "machine-1", "Linux", 10),
                })
                .expect("register should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
    }
    sleep(Duration::from_millis(50)).await;

    let (mut client_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("client should connect");
    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-a".to_string()),
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

    let encrypted_request = EncryptedRelayPayload {
        sender_public_key: "client-public".to_string(),
        nonce: "nonce".to_string(),
        ciphertext: "ciphertext".to_string(),
    };
    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientRequest {
                request_id: "request-wrong-target".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-b".to_string()),
                    daemon_alias: None,
                },
                encrypted_request: encrypted_request.clone(),
            })
            .expect("wrong-target request should serialize")
            .into(),
        ))
        .await
        .expect("wrong-target request should send");
    match client_socket.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("wrong-target response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "request-wrong-target");
                assert_eq!(error.code, "target_mismatch");
            }
            other => panic!("unexpected wrong-target response: {other:?}"),
        },
        other => panic!("unexpected wrong-target frame: {other:?}"),
    }
    match timeout(Duration::from_millis(100), daemon_b.next()).await {
        Err(_) => {}
        Ok(other) => panic!("wrong-target request reached daemon B: {other:?}"),
    }

    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                request_id: "subscribe-wrong-target".to_string(),
                subscription_id: "wrong-target-subscription".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-b".to_string()),
                    daemon_alias: None,
                },
                session_id: "session-1".to_string(),
                attachment_id: "terminal".to_string(),
                client_public_key: "client-public".to_string(),
                subscription_scope: None,
                resume_from_event_id: None,
            })
            .expect("wrong-target subscribe should serialize")
            .into(),
        ))
        .await
        .expect("wrong-target subscribe should send");
    match client_socket.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("wrong-target subscribe response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "subscribe-wrong-target");
                assert_eq!(error.code, "target_mismatch");
            }
            other => panic!("unexpected wrong-target subscribe response: {other:?}"),
        },
        other => panic!("unexpected wrong-target subscribe frame: {other:?}"),
    }
    assert_eq!(registry.read().await.pending_request_count(), 0);

    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientRequest {
                request_id: "request-right-target".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-a".to_string()),
                    daemon_alias: None,
                },
                encrypted_request,
            })
            .expect("right-target request should serialize")
            .into(),
        ))
        .await
        .expect("right-target request should send");
    match daemon_a.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("right-target daemon request should decode")
        {
            RelayEnvelope::DaemonRequest { .. } => {}
            other => panic!("unexpected right-target daemon envelope: {other:?}"),
        },
        other => panic!("unexpected right-target daemon frame: {other:?}"),
    }

    let _ = client_socket.close(None).await;
    let _ = daemon_a.close(None).await;
    let _ = daemon_b.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
