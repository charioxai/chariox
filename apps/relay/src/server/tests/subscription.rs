use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn subscription_ids_are_owned_by_connected_client() {
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

    let (mut client_a, _) = connect_async_with_retry(&url)
        .await
        .expect("client A should connect");
    let (mut client_b, _) = connect_async_with_retry(&url)
        .await
        .expect("client B should connect");
    for (socket, label) in [(&mut client_a, "a"), (&mut client_b, "b")] {
        socket
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
            .unwrap_or_else(|error| panic!("client {label} connect should send: {error}"));
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                assert!(matches!(
                    serde_json::from_str::<RelayEnvelope>(&text)
                        .expect("client connected should decode"),
                    RelayEnvelope::ClientConnected { .. }
                ));
            }
            other => panic!("unexpected client {label} connect response: {other:?}"),
        }
    }

    client_a
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                request_id: "subscribe-a".to_string(),
                subscription_id: "shared-subscription-id".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
                session_id: "session-1".to_string(),
                attachment_id: "terminal".to_string(),
                client_public_key: "client-a-public".to_string(),
                subscription_scope: None,
                resume_from_event_id: None,
            })
            .expect("client A subscribe should serialize")
            .into(),
        ))
        .await
        .expect("client A subscribe should send");

    let subscribe_relay_request_id = match daemon_socket.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("daemon subscribe should decode")
        {
            RelayEnvelope::DaemonSubscribe {
                relay_request_id,
                relay_subscription_id,
                ..
            } => {
                assert_eq!(relay_subscription_id, "shared-subscription-id");
                relay_request_id
            }
            other => panic!("unexpected daemon subscribe envelope: {other:?}"),
        },
        other => panic!("unexpected daemon subscribe frame: {other:?}"),
    };

    client_b
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                request_id: "subscribe-b-pending-collision".to_string(),
                subscription_id: "shared-subscription-id".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
                session_id: "session-1".to_string(),
                attachment_id: "terminal".to_string(),
                client_public_key: "client-b-public".to_string(),
                subscription_scope: None,
                resume_from_event_id: None,
            })
            .expect("client B subscribe should serialize")
            .into(),
        ))
        .await
        .expect("client B subscribe should send");
    match client_b.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("client B subscribe response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "subscribe-b-pending-collision");
                assert_eq!(error.code, "subscription_conflict");
            }
            other => panic!("unexpected client B collision response: {other:?}"),
        },
        other => panic!("unexpected client B collision frame: {other:?}"),
    }
    match timeout(Duration::from_millis(100), daemon_socket.next()).await {
        Err(_) => {}
        Ok(other) => panic!("conflicting subscribe reached daemon: {other:?}"),
    }
    assert_eq!(registry.read().await.pending_request_count(), 1);

    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonResponse {
                relay_request_id: subscribe_relay_request_id,
                encrypted_response: None,
                error: None,
            })
            .expect("subscribe response should serialize")
            .into(),
        ))
        .await
        .expect("subscribe response should send");
    match client_a.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("client A subscribe response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: None,
            } => assert_eq!(request_id, "subscribe-a"),
            other => panic!("unexpected client A subscribe response: {other:?}"),
        },
        other => panic!("unexpected client A subscribe frame: {other:?}"),
    }
    assert_eq!(registry.read().await.subscription_count(), 1);

    client_b
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientUnsubscribe {
                request_id: "unsubscribe-b".to_string(),
                subscription_id: "shared-subscription-id".to_string(),
                client_public_key: "client-b-public".to_string(),
            })
            .expect("client B unsubscribe should serialize")
            .into(),
        ))
        .await
        .expect("client B unsubscribe should send");
    match client_b.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("client B unsubscribe response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "unsubscribe-b");
                assert_eq!(error.code, "subscription_not_found");
            }
            other => panic!("unexpected client B unsubscribe response: {other:?}"),
        },
        other => panic!("unexpected client B unsubscribe frame: {other:?}"),
    }
    match timeout(Duration::from_millis(100), daemon_socket.next()).await {
        Err(_) => {}
        Ok(other) => panic!("cross-client unsubscribe reached daemon: {other:?}"),
    }
    assert_eq!(registry.read().await.subscription_count(), 1);

    client_b
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                request_id: "subscribe-b-active-collision".to_string(),
                subscription_id: "shared-subscription-id".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
                session_id: "session-1".to_string(),
                attachment_id: "terminal".to_string(),
                client_public_key: "client-b-public".to_string(),
                subscription_scope: None,
                resume_from_event_id: None,
            })
            .expect("client B subscribe should serialize")
            .into(),
        ))
        .await
        .expect("client B subscribe should send");
    match client_b.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("client B active collision response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "subscribe-b-active-collision");
                assert_eq!(error.code, "subscription_conflict");
            }
            other => panic!("unexpected client B active collision response: {other:?}"),
        },
        other => panic!("unexpected client B active collision frame: {other:?}"),
    }

    let encrypted_event = EncryptedRelayPayload {
        sender_public_key: "daemon-public".to_string(),
        nonce: "nonce-event".to_string(),
        ciphertext: "ciphertext-event".to_string(),
    };
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonEvent {
                subscription_id: "shared-subscription-id".to_string(),
                event_id: 1,
                encrypted_event: encrypted_event.clone(),
            })
            .expect("daemon event should serialize")
            .into(),
        ))
        .await
        .expect("daemon event should send");
    match client_a.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("client A event should decode")
        {
            RelayEnvelope::ClientEvent {
                subscription_id,
                event_id,
                encrypted_event: response,
            } => {
                assert_eq!(subscription_id, "shared-subscription-id");
                assert_eq!(event_id, 1);
                assert_eq!(response, encrypted_event);
            }
            other => panic!("unexpected client A event response: {other:?}"),
        },
        other => panic!("unexpected client A event frame: {other:?}"),
    }
    match timeout(Duration::from_millis(100), client_b.next()).await {
        Err(_) => {}
        Ok(other) => panic!("event leaked to non-owner client: {other:?}"),
    }

    let _ = client_a.close(None).await;
    let _ = client_b.close(None).await;
    let _ = daemon_socket.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn disconnecting_client_drops_pending_requests_before_reconnect() {
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

    let (mut first_client, _) = connect_async_with_retry(&url)
        .await
        .expect("first client should connect");
    first_client
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
            })
            .expect("first client connect should serialize")
            .into(),
        ))
        .await
        .expect("first client connect should send");
    match first_client.next().await {
        Some(Ok(Message::Text(text))) => assert!(matches!(
            serde_json::from_str::<RelayEnvelope>(&text).expect("connect should decode"),
            RelayEnvelope::ClientConnected { .. }
        )),
        other => panic!("unexpected first client connect response: {other:?}"),
    }

    first_client
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                request_id: "first-subscribe".to_string(),
                subscription_id: "recoverable-subscription".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
                session_id: "session-1".to_string(),
                attachment_id: "terminal".to_string(),
                client_public_key: "first-client-public".to_string(),
                subscription_scope: None,
                resume_from_event_id: None,
            })
            .expect("first subscribe should serialize")
            .into(),
        ))
        .await
        .expect("first subscribe should send");
    let stale_relay_request_id = match daemon_socket.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("daemon subscribe should decode")
        {
            RelayEnvelope::DaemonSubscribe {
                relay_request_id,
                relay_subscription_id,
                ..
            } => {
                assert_eq!(relay_subscription_id, "recoverable-subscription");
                relay_request_id
            }
            other => panic!("unexpected first subscribe envelope: {other:?}"),
        },
        other => panic!("unexpected first subscribe frame: {other:?}"),
    };
    assert_eq!(registry.read().await.pending_request_count(), 1);

    let _ = first_client.close(None).await;
    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        registry.read().await.pending_request_count(),
        0,
        "disconnecting clients must not leave stale pending relay requests"
    );
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonResponse {
                relay_request_id: stale_relay_request_id,
                encrypted_response: None,
                error: None,
            })
            .expect("stale subscribe response should serialize")
            .into(),
        ))
        .await
        .expect("stale subscribe response should send");
    sleep(Duration::from_millis(50)).await;
    assert_eq!(registry.read().await.subscription_count(), 0);

    let (mut reconnecting_client, _) = connect_async_with_retry(&url)
        .await
        .expect("reconnecting client should connect");
    reconnecting_client
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
            })
            .expect("reconnecting client connect should serialize")
            .into(),
        ))
        .await
        .expect("reconnecting client connect should send");
    match reconnecting_client.next().await {
        Some(Ok(Message::Text(text))) => assert!(matches!(
            serde_json::from_str::<RelayEnvelope>(&text).expect("connect should decode"),
            RelayEnvelope::ClientConnected { .. }
        )),
        other => panic!("unexpected reconnecting client connect response: {other:?}"),
    }
    reconnecting_client
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                request_id: "reconnect-subscribe".to_string(),
                subscription_id: "recoverable-subscription".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
                session_id: "session-1".to_string(),
                attachment_id: "terminal".to_string(),
                client_public_key: "reconnecting-client-public".to_string(),
                subscription_scope: None,
                resume_from_event_id: None,
            })
            .expect("reconnect subscribe should serialize")
            .into(),
        ))
        .await
        .expect("reconnect subscribe should send");
    let relay_request_id = match daemon_socket.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("reconnect daemon subscribe should decode")
        {
            RelayEnvelope::DaemonSubscribe {
                relay_request_id,
                relay_subscription_id,
                ..
            } => {
                assert_eq!(relay_subscription_id, "recoverable-subscription");
                relay_request_id
            }
            other => panic!("unexpected reconnect subscribe envelope: {other:?}"),
        },
        other => panic!("unexpected reconnect subscribe frame: {other:?}"),
    };
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonResponse {
                relay_request_id,
                encrypted_response: None,
                error: None,
            })
            .expect("reconnect subscribe response should serialize")
            .into(),
        ))
        .await
        .expect("reconnect subscribe response should send");
    match reconnecting_client.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("reconnect subscribe response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: None,
            } => assert_eq!(request_id, "reconnect-subscribe"),
            other => panic!("unexpected reconnect subscribe response: {other:?}"),
        },
        other => panic!("unexpected reconnect subscribe response frame: {other:?}"),
    }
    assert_eq!(registry.read().await.subscription_count(), 1);

    let _ = reconnecting_client.close(None).await;
    let _ = daemon_socket.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
