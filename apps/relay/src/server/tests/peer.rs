use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn daemon_peer_requests_are_routed_between_registered_kernels() {
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
        .expect("daemon A should connect to relay");
    let (mut daemon_b, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon B should connect to relay");

    for (socket, daemon_id, daemon_alias, public_key) in [
        (&mut daemon_a, "daemon-a", "alpha", "public-key-a"),
        (&mut daemon_b, "daemon-b", "beta", "public-key-b"),
    ] {
        let register = RelayEnvelope::DaemonRegister {
            registration: DaemonRegistration {
                auth_token: "secret".to_string(),
                daemon_id: daemon_id.to_string(),
                machine_id: format!("machine-{daemon_id}"),
                machine_alias: None,
                os_name: Some("Linux".to_string()),
                kernel_started_at_ms: 10,
                daemon_alias: Some(daemon_alias.to_string()),
                kernel_alias: Some(daemon_alias.to_string()),
                public_key: public_key.to_string(),
                capabilities: vec!["kernel_ws".to_string()],
                available_providers: vec!["opencode".to_string()],
                provider_accounts: Vec::new(),
                accepting_remote_leases: true,
                leased_agent_count: 0,
                local_session_count: 0,
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
    }
    sleep(Duration::from_millis(50)).await;

    let encrypted_request = EncryptedRelayPayload {
        sender_public_key: "client-public".to_string(),
        nonce: "nonce".to_string(),
        ciphertext: "ciphertext".to_string(),
    };
    daemon_a
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonPeerRequest {
                request_id: "".to_string(),
                target: ClientTarget {
                    daemon_id: None,
                    daemon_alias: Some("beta".to_string()),
                },
                encrypted_request: encrypted_request.clone(),
            })
            .expect("invalid peer request should serialize")
            .into(),
        ))
        .await
        .expect("invalid peer request should send");
    match timeout(Duration::from_millis(500), daemon_a.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("invalid peer response should decode")
        {
            RelayEnvelope::DaemonPeerResponse {
                request_id,
                from_daemon_id,
                encrypted_response: None,
                error: Some(error),
            } => {
                assert_eq!(request_id, "");
                assert_eq!(from_daemon_id, "");
                assert_eq!(error.code, "invalid_runtime_identifier");
                assert!(!error.retryable);
            }
            other => panic!("unexpected invalid peer response: {other:?}"),
        },
        Ok(other) => panic!("unexpected invalid peer response frame: {other:?}"),
        Err(_) => panic!("invalid peer response was not delivered"),
    }
    match timeout(Duration::from_millis(100), daemon_b.next()).await {
        Err(_) => {}
        Ok(other) => panic!("invalid peer request reached target daemon: {other:?}"),
    }
    assert_eq!(registry.read().await.pending_request_count(), 0);

    daemon_a
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonPeerRequest {
                request_id: "peer-1".to_string(),
                target: ClientTarget {
                    daemon_id: None,
                    daemon_alias: Some("beta".to_string()),
                },
                encrypted_request: encrypted_request.clone(),
            })
            .expect("peer request should serialize")
            .into(),
        ))
        .await
        .expect("peer request should send");

    let incoming_payload = match daemon_b.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected incoming peer request: {other:?}"),
    };
    let relay_request_id = match serde_json::from_str::<RelayEnvelope>(&incoming_payload)
        .expect("incoming peer request should decode")
    {
        RelayEnvelope::DaemonIncomingPeerRequest {
            relay_request_id,
            from_daemon_id,
            caller_identity,
            encrypted_request: forwarded,
        } => {
            assert_eq!(from_daemon_id, "daemon-a");
            assert_eq!(
                caller_identity
                    .as_ref()
                    .map(|identity| identity.subject.as_str()),
                Some("shared-token-bootstrap")
            );
            assert_eq!(forwarded, encrypted_request);
            relay_request_id
        }
        other => panic!("unexpected incoming peer envelope: {other:?}"),
    };

    let encrypted_response = EncryptedRelayPayload {
        sender_public_key: "daemon-b-public".to_string(),
        nonce: "nonce-2".to_string(),
        ciphertext: "ciphertext-2".to_string(),
    };
    daemon_b
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonIncomingPeerResponse {
                relay_request_id,
                encrypted_response: Some(encrypted_response.clone()),
                error: None,
            })
            .expect("peer response should serialize")
            .into(),
        ))
        .await
        .expect("peer response should send");

    let response_payload = match daemon_a.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected routed peer response: {other:?}"),
    };
    match serde_json::from_str::<RelayEnvelope>(&response_payload)
        .expect("routed peer response should decode")
    {
        RelayEnvelope::DaemonPeerResponse {
            request_id,
            from_daemon_id,
            encrypted_response: Some(forwarded),
            error: None,
        } => {
            assert_eq!(request_id, "peer-1");
            assert_eq!(from_daemon_id, "daemon-b");
            assert_eq!(forwarded, encrypted_response);
        }
        other => panic!("unexpected routed peer response envelope: {other:?}"),
    }

    let _ = daemon_a.close(None).await;
    let _ = daemon_b.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn scoped_daemon_tokens_gate_peer_routing_actions() {
    let mut claims = BTreeMap::new();
    claims.insert(
        "daemon-a-token".to_string(),
        scoped_claim(
            "daemon-a-token",
            "daemon-a",
            RelaySubjectKind::Kernel,
            "realm-a",
            vec![RelayAction::DaemonRegister],
            None,
        ),
    );
    claims.insert(
        "daemon-b-token".to_string(),
        scoped_claim(
            "daemon-b-token",
            "daemon-b",
            RelaySubjectKind::Kernel,
            "realm-a",
            vec![RelayAction::DaemonRegister],
            None,
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
    for (socket, daemon_id, auth_token) in [
        (&mut daemon_a, "daemon-a", "daemon-a-token"),
        (&mut daemon_b, "daemon-b", "daemon-b-token"),
    ] {
        socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration_with_token(
                        daemon_id,
                        "machine-1",
                        "Linux",
                        10,
                        auth_token,
                    ),
                })
                .expect("register should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
    }
    sleep(Duration::from_millis(50)).await;

    let encrypted_request = EncryptedRelayPayload {
        sender_public_key: "daemon-a-public".to_string(),
        nonce: "nonce".to_string(),
        ciphertext: "ciphertext".to_string(),
    };
    daemon_a
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonPeerRequest {
                request_id: "peer-denied".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-b".to_string()),
                    daemon_alias: None,
                },
                encrypted_request: encrypted_request.clone(),
            })
            .expect("peer request should serialize")
            .into(),
        ))
        .await
        .expect("peer request should send");
    match daemon_a.next().await {
        Some(Ok(Message::Text(text))) => {
            match serde_json::from_str::<RelayEnvelope>(&text).expect("peer denial should decode") {
                RelayEnvelope::DaemonPeerResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                    ..
                } => {
                    assert_eq!(request_id, "peer-denied");
                    assert_eq!(error.code, "action_not_allowed");
                }
                other => panic!("unexpected peer denial envelope: {other:?}"),
            }
        }
        other => panic!("unexpected peer denial frame: {other:?}"),
    }
    match timeout(Duration::from_millis(100), daemon_b.next()).await {
        Err(_) => {}
        Ok(other) => panic!("unauthorized peer request reached target daemon: {other:?}"),
    }

    daemon_a
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonPeerEvent {
                target: ClientTarget {
                    daemon_id: Some("daemon-b".to_string()),
                    daemon_alias: None,
                },
                encrypted_event: encrypted_request,
            })
            .expect("peer event should serialize")
            .into(),
        ))
        .await
        .expect("peer event should send");
    match timeout(Duration::from_millis(100), daemon_b.next()).await {
        Err(_) => {}
        Ok(other) => panic!("unauthorized peer event reached target daemon: {other:?}"),
    }

    let _ = daemon_a.close(None).await;
    let _ = daemon_b.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
