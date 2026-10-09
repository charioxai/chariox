use super::*;

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;
async fn send(socket: &mut Socket, envelope: RelayEnvelope) {
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope).unwrap().into(),
        ))
        .await
        .unwrap();
}
async fn receive(socket: &mut Socket) -> RelayEnvelope {
    match timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    {
        Message::Text(text) => serde_json::from_str(&text).unwrap(),
        _ => panic!("expected relay envelope"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn client_renewal_preserves_one_subscription_across_expiries_and_revocation_stops_it() {
    renewal_drill(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn client_renewal_removing_packet_route_stops_existing_subscription_events() {
    renewal_drill(true).await;
}

async fn renewal_drill(reduce_permissions: bool) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let mut claims = BTreeMap::new();
    let mut daemon = scoped_claim(
        "daemon",
        "daemon-1",
        RelaySubjectKind::Kernel,
        "realm-a",
        vec![RelayAction::DaemonRegister, RelayAction::PacketRoute],
        None,
    );
    daemon.issued_at_ms = now;
    daemon.expires_at_ms = now + 15_000;
    claims.insert("daemon".into(), daemon);
    for i in 0..=6 {
        let token = format!("client-{i}");
        let mut claim = scoped_claim(
            &token,
            "terminal-profile",
            RelaySubjectKind::Client,
            "realm-a",
            vec![RelayAction::ClientConnect, RelayAction::PacketRoute],
            Some(vec!["daemon-1"]),
        );
        claim.issued_at_ms = now;
        claim.expires_at_ms = now + 400 + i * 300;
        claims.insert(token, claim);
    }
    let mut different_key = claims["client-6"].clone();
    different_key.token_id = "different-key".into();
    different_key.public_key_thumbprint = Some("a".repeat(64));
    claims.insert("different-key".into(), different_key);
    let mut reduced = claims["client-6"].clone();
    reduced.token_id = "reduced-actions".into();
    reduced.allowed_actions = vec![RelayAction::ClientConnect];
    claims.insert("reduced-actions".into(), reduced);
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: None,
        },
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), None)),
    );
    let listener = server.bind_listener().await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = server.registry();
    let revocations = server.revocations();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = stopped.await;
            })
            .await
            .unwrap()
    });
    let url = format!("ws://{address}");
    let (mut daemon_socket, _) = connect_async_with_retry(&url).await.unwrap();
    send(
        &mut daemon_socket,
        RelayEnvelope::DaemonRegister {
            registration: test_registration_with_token(
                "daemon-1",
                "machine-a",
                "Linux",
                10,
                "daemon",
            ),
        },
    )
    .await;
    sleep(Duration::from_millis(25)).await;
    let (mut client, _) = connect_async_with_retry(&url).await.unwrap();
    let target = ClientTarget {
        daemon_id: Some("daemon-1".into()),
        daemon_alias: None,
    };
    send(
        &mut client,
        RelayEnvelope::ClientConnect {
            auth_token: "client-0".into(),
            target: target.clone(),
        },
    )
    .await;
    assert!(matches!(
        receive(&mut client).await,
        RelayEnvelope::ClientConnected { .. }
    ));
    send(
        &mut client,
        RelayEnvelope::ClientSubscribe {
            request_id: "subscribe".into(),
            subscription_id: "terminal-stream".into(),
            target: target.clone(),
            session_id: "session".into(),
            attachment_id: "attachment".into(),
            client_public_key: "fixture-terminal-key".into(),
            subscription_scope: None,
            resume_from_event_id: None,
        },
    )
    .await;
    let request = match receive(&mut daemon_socket).await {
        RelayEnvelope::DaemonSubscribe {
            relay_request_id, ..
        } => relay_request_id,
        _ => panic!("expected subscription"),
    };
    send(
        &mut daemon_socket,
        RelayEnvelope::DaemonResponse {
            relay_request_id: request,
            encrypted_response: None,
            error: None,
        },
    )
    .await;
    assert!(matches!(
        receive(&mut client).await,
        RelayEnvelope::ClientResponse { .. }
    ));
    let (mut foreign, _) = connect_async_with_retry(&url).await.unwrap();
    send(
        &mut foreign,
        RelayEnvelope::ClientConnect {
            auth_token: "client-0".into(),
            target: target.clone(),
        },
    )
    .await;
    assert!(matches!(
        receive(&mut foreign).await,
        RelayEnvelope::ClientConnected { .. }
    ));
    send(
        &mut foreign,
        RelayEnvelope::ClientConnect {
            auth_token: "different-key".into(),
            target: target.clone(),
        },
    )
    .await;
    assert!(
        matches!(receive(&mut foreign).await, RelayEnvelope::Close { reason } if reason == "relay authorization renewal changed identity or reduced permissions")
    );
    let _ = foreign.close(None).await;
    for i in 1..=6 {
        sleep(Duration::from_millis(300)).await;
        send(
            &mut client,
            RelayEnvelope::ClientConnect {
                auth_token: format!("client-{i}"),
                target: target.clone(),
            },
        )
        .await;
        assert!(matches!(
            receive(&mut client).await,
            RelayEnvelope::ClientConnected { .. }
        ));
        send(
            &mut daemon_socket,
            RelayEnvelope::DaemonEvent {
                subscription_id: "terminal-stream".into(),
                event_id: i,
                encrypted_event: EncryptedRelayPayload {
                    sender_public_key: "fixture-daemon-key".into(),
                    nonce: "fixture-nonce".into(),
                    ciphertext: "fixture-event".into(),
                },
            },
        )
        .await;
        assert!(
            matches!(receive(&mut client).await, RelayEnvelope::ClientEvent { event_id, .. } if event_id == i)
        );
        assert_eq!(registry.read().await.subscription_count(), 1);
    }
    if reduce_permissions {
        send(
            &mut client,
            RelayEnvelope::ClientConnect {
                auth_token: "reduced-actions".into(),
                target: target.clone(),
            },
        )
        .await;
        assert!(matches!(
            receive(&mut client).await,
            RelayEnvelope::Close { .. }
        ));
        sleep(Duration::from_millis(25)).await;
        assert_eq!(registry.read().await.subscription_count(), 0);
        send(
            &mut daemon_socket,
            RelayEnvelope::DaemonEvent {
                subscription_id: "terminal-stream".into(),
                event_id: 99,
                encrypted_event: EncryptedRelayPayload {
                    sender_public_key: "fixture-daemon-key".into(),
                    nonce: "fixture-nonce".into(),
                    ciphertext: "fixture-event".into(),
                },
            },
        )
        .await;
        // The old route has gone; no event can be forwarded after reduction.
        while let Ok(Some(Ok(message))) = timeout(Duration::from_millis(100), client.next()).await {
            if let Message::Text(text) = message {
                assert!(!matches!(
                    serde_json::from_str::<RelayEnvelope>(&text).unwrap(),
                    RelayEnvelope::ClientEvent { .. }
                ));
            }
        }
    } else {
        // The active token's revocation immediately closes the existing connection.
        revocations.revoke_token_id("client-6", now + 2_200);
        assert!(
            matches!(receive(&mut client).await, RelayEnvelope::Close { reason } if reason == "relay token revoked")
        );
    }
    let _ = client.close(None).await;
    let _ = daemon_socket.close(None).await;
    let _ = stop.send(());
    task.await.unwrap();
}
