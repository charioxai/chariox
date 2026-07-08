use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_daemon_aliases_do_not_bind_clients_arbitrarily() {
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
    let (mut daemon_a, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon A should connect");
    let (mut daemon_b, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon B should connect");
    let mut registration_a = test_registration("daemon-a", "machine-1", "Linux", 10);
    registration_a.daemon_alias = Some("shared-alias".to_string());
    registration_a.public_key = "public-key-a".to_string();
    let mut registration_b = test_registration("daemon-b", "machine-2", "Linux", 20);
    registration_b.daemon_alias = Some("shared-alias".to_string());
    registration_b.public_key = "public-key-b".to_string();
    for (socket, registration) in [
        (&mut daemon_a, registration_a),
        (&mut daemon_b, registration_b),
    ] {
        socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister { registration })
                    .expect("register should serialize")
                    .into(),
            ))
            .await
            .expect("register should send");
    }
    sleep(Duration::from_millis(50)).await;

    let (mut ambiguous_client, _) = connect_async_with_retry(&url)
        .await
        .expect("ambiguous client should connect");
    ambiguous_client
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: None,
                    daemon_alias: Some("shared-alias".to_string()),
                },
            })
            .expect("ambiguous connect should serialize")
            .into(),
        ))
        .await
        .expect("ambiguous connect should send");
    let close_payload = match timeout(Duration::from_millis(500), ambiguous_client.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => String::new().into(),
        Ok(other) => panic!("unexpected ambiguous connect response: {other:?}"),
        Err(_) => panic!("ambiguous connect did not close promptly"),
    };
    if !close_payload.is_empty() {
        match serde_json::from_str::<RelayEnvelope>(&close_payload)
            .expect("ambiguous close should decode")
        {
            RelayEnvelope::Close { reason } => {
                assert_eq!(reason, "target daemon is not connected to relay");
            }
            RelayEnvelope::ClientConnected {
                daemon_public_key, ..
            } => {
                panic!("ambiguous alias connected to daemon key {daemon_public_key}")
            }
            other => panic!("unexpected ambiguous connect envelope: {other:?}"),
        }
    }

    let (mut exact_client, _) = connect_async_with_retry(&url)
        .await
        .expect("exact client should connect");
    exact_client
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-a".to_string()),
                    daemon_alias: None,
                },
            })
            .expect("exact connect should serialize")
            .into(),
        ))
        .await
        .expect("exact connect should send");
    match exact_client.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("exact connect response should decode")
        {
            RelayEnvelope::ClientConnected {
                daemon_public_key, ..
            } => assert_eq!(daemon_public_key, "public-key-a"),
            other => panic!("unexpected exact connect envelope: {other:?}"),
        },
        other => panic!("unexpected exact connect response: {other:?}"),
    }

    let _ = ambiguous_client.close(None).await;
    let _ = exact_client.close(None).await;
    let _ = daemon_a.close(None).await;
    let _ = daemon_b.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn scoped_client_tokens_gate_packet_routing() {
    let mut claims = BTreeMap::new();
    claims.insert(
        "daemon-token".to_string(),
        scoped_claim(
            "daemon-token",
            "daemon-1",
            RelaySubjectKind::Kernel,
            "realm-a",
            vec![RelayAction::DaemonRegister],
            None,
        ),
    );
    claims.insert(
        "client-connect-only-token".to_string(),
        scoped_claim(
            "client-connect-only-token",
            "client-1",
            RelaySubjectKind::Client,
            "realm-a",
            vec![RelayAction::ClientConnect],
            Some(vec!["daemon-1"]),
        ),
    );
    let auth_verifier =
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), Some(10)));
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: None,
        },
        auth_verifier,
    );
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let auth_verifier = server.auth_verifier.clone();
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: None,
        },
        auth_verifier,
    );
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
                registration: test_registration_with_token(
                    "daemon-1",
                    "machine-1",
                    "Linux",
                    10,
                    "daemon-token",
                ),
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
                auth_token: "client-connect-only-token".to_string(),
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
                request_id: "packet-route-denied".to_string(),
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
    match client_socket.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("packet-route denial should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "packet-route-denied");
                assert_eq!(error.code, "action_not_allowed");
            }
            other => panic!("unexpected packet-route denial envelope: {other:?}"),
        },
        other => panic!("unexpected packet-route denial frame: {other:?}"),
    }
    match timeout(Duration::from_millis(100), daemon_socket.next()).await {
        Err(_) => {}
        Ok(other) => panic!("unauthorized packet route reached daemon: {other:?}"),
    }
    assert_eq!(registry.read().await.pending_request_count(), 0);

    let _ = client_socket.close(None).await;
    let _ = daemon_socket.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_responses_must_match_pending_request_owner() {
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
        Some(Ok(Message::Text(text))) => {
            assert!(matches!(
                serde_json::from_str::<RelayEnvelope>(&text)
                    .expect("client connected should decode"),
                RelayEnvelope::ClientConnected { .. }
            ));
        }
        other => panic!("unexpected client connect response: {other:?}"),
    }

    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientRequest {
                request_id: "client-request-1".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-a".to_string()),
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

    let relay_request_id = match daemon_a.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("daemon request should decode")
        {
            RelayEnvelope::DaemonRequest {
                relay_request_id, ..
            } => relay_request_id,
            other => panic!("unexpected daemon request envelope: {other:?}"),
        },
        other => panic!("unexpected daemon request frame: {other:?}"),
    };

    let encrypted_response = EncryptedRelayPayload {
        sender_public_key: "daemon-public".to_string(),
        nonce: "nonce-response".to_string(),
        ciphertext: "ciphertext-response".to_string(),
    };
    daemon_b
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonResponse {
                relay_request_id: relay_request_id.clone(),
                encrypted_response: Some(encrypted_response.clone()),
                error: None,
            })
            .expect("wrong daemon response should serialize")
            .into(),
        ))
        .await
        .expect("wrong daemon response should send");
    match timeout(Duration::from_millis(100), client_socket.next()).await {
        Err(_) => {}
        Ok(other) => panic!("wrong daemon completed client request: {other:?}"),
    }
    assert_eq!(registry.read().await.pending_request_count(), 1);

    daemon_a
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonResponse {
                relay_request_id,
                encrypted_response: Some(encrypted_response.clone()),
                error: None,
            })
            .expect("owner daemon response should serialize")
            .into(),
        ))
        .await
        .expect("owner daemon response should send");
    match timeout(Duration::from_millis(500), client_socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("client response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: Some(response),
                error: None,
            } => {
                assert_eq!(request_id, "client-request-1");
                assert_eq!(response, encrypted_response);
            }
            other => panic!("unexpected client response envelope: {other:?}"),
        },
        Ok(other) => panic!("unexpected client response frame: {other:?}"),
        Err(_) => panic!("owner daemon response was not delivered"),
    }
    assert_eq!(registry.read().await.pending_request_count(), 0);

    let _ = client_socket.close(None).await;
    let _ = daemon_a.close(None).await;
    let _ = daemon_b.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_events_must_match_subscription_owner() {
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
        Some(Ok(Message::Text(text))) => {
            assert!(matches!(
                serde_json::from_str::<RelayEnvelope>(&text)
                    .expect("client connected should decode"),
                RelayEnvelope::ClientConnected { .. }
            ));
        }
        other => panic!("unexpected client connect response: {other:?}"),
    }

    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                request_id: "subscribe-1".to_string(),
                subscription_id: "subscription-1".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-a".to_string()),
                    daemon_alias: None,
                },
                session_id: "session-1".to_string(),
                attachment_id: "terminal".to_string(),
                client_public_key: "client-public".to_string(),
                subscription_scope: None,
                resume_from_event_id: None,
            })
            .expect("client subscribe should serialize")
            .into(),
        ))
        .await
        .expect("client subscribe should send");

    let subscribe_relay_request_id = match daemon_a.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("daemon subscribe should decode")
        {
            RelayEnvelope::DaemonSubscribe {
                relay_request_id,
                relay_subscription_id,
                ..
            } => {
                assert_eq!(relay_subscription_id, "subscription-1");
                relay_request_id
            }
            other => panic!("unexpected daemon subscribe envelope: {other:?}"),
        },
        other => panic!("unexpected daemon subscribe frame: {other:?}"),
    };
    daemon_a
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
    match timeout(Duration::from_millis(500), client_socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("subscribe response should decode")
        {
            RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: None,
            } => assert_eq!(request_id, "subscribe-1"),
            other => panic!("unexpected subscribe response envelope: {other:?}"),
        },
        Ok(other) => panic!("unexpected subscribe response frame: {other:?}"),
        Err(_) => panic!("subscribe response was not delivered"),
    }
    assert_eq!(registry.read().await.subscription_count(), 1);

    let encrypted_event = EncryptedRelayPayload {
        sender_public_key: "daemon-public".to_string(),
        nonce: "nonce-event".to_string(),
        ciphertext: "ciphertext-event".to_string(),
    };
    daemon_b
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonEvent {
                subscription_id: "subscription-1".to_string(),
                event_id: 1,
                encrypted_event: encrypted_event.clone(),
            })
            .expect("wrong daemon event should serialize")
            .into(),
        ))
        .await
        .expect("wrong daemon event should send");
    match timeout(Duration::from_millis(100), client_socket.next()).await {
        Err(_) => {}
        Ok(other) => panic!("wrong daemon emitted subscription event: {other:?}"),
    }

    daemon_a
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonEvent {
                subscription_id: "subscription-1".to_string(),
                event_id: 2,
                encrypted_event: encrypted_event.clone(),
            })
            .expect("owner daemon event should serialize")
            .into(),
        ))
        .await
        .expect("owner daemon event should send");
    match timeout(Duration::from_millis(500), client_socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("client event should decode")
        {
            RelayEnvelope::ClientEvent {
                subscription_id,
                event_id,
                encrypted_event: response,
            } => {
                assert_eq!(subscription_id, "subscription-1");
                assert_eq!(event_id, 2);
                assert_eq!(response, encrypted_event);
            }
            other => panic!("unexpected client event envelope: {other:?}"),
        },
        Ok(other) => panic!("unexpected client event frame: {other:?}"),
        Err(_) => panic!("owner daemon event was not delivered"),
    }

    let _ = client_socket.close(None).await;
    let _ = daemon_a.close(None).await;
    let _ = daemon_b.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
