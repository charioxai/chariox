use super::*;

use crate::meter::{UsageCounters, CONTROL_SCOPE};

struct Tally {
    sent: u64,
    received: u64,
}

async fn send(socket: &mut Socket, tally: &mut Tally, envelope: &RelayEnvelope) {
    let text = serde_json::to_string(envelope).expect("envelope should serialize");
    tally.sent += text.len() as u64;
    socket
        .send(Message::Text(text.into()))
        .await
        .expect("frame should send");
}

async fn receive(socket: &mut Socket, tally: &mut Tally) -> (RelayEnvelope, u64) {
    match timeout(Duration::from_secs(5), socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            tally.received += text.len() as u64;
            let envelope = serde_json::from_str(&text).expect("frame should decode");
            (envelope, text.len() as u64)
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn target() -> ClientTarget {
    ClientTarget {
        daemon_id: Some("daemon-1".to_string()),
        daemon_alias: None,
    }
}

fn payload(size: usize) -> EncryptedRelayPayload {
    EncryptedRelayPayload {
        sender_public_key: "daemon-public".to_string(),
        nonce: "nonce".to_string(),
        ciphertext: "c".repeat(size),
    }
}

/// MP-08 / MP-10: every byte a connection exchanges with the relay is counted once
/// under the realm and the scope of the stream it belongs to, and nothing else is.
#[tokio::test(flavor = "multi_thread")]
async fn relay_counts_bytes_per_realm_and_stream_scope() {
    let probe = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = probe
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let meter = server.usage_meter();
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

    let mut daemon_tally = Tally {
        sent: 0,
        received: 0,
    };
    let mut client_tally = Tally {
        sent: 0,
        received: 0,
    };
    let (mut daemon, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon should connect");
    send(
        &mut daemon,
        &mut daemon_tally,
        &RelayEnvelope::DaemonRegister {
            registration: test_registration("daemon-1", "machine-1", "Linux", 10),
        },
    )
    .await;
    sleep(Duration::from_millis(50)).await;
    let (mut client, _) = connect_async_with_retry(&url)
        .await
        .expect("client should connect");
    send(
        &mut client,
        &mut client_tally,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: target(),
        },
    )
    .await;
    receive(&mut client, &mut client_tally).await;

    send(
        &mut client,
        &mut client_tally,
        &RelayEnvelope::ClientSubscribe {
            request_id: "subscribe".to_string(),
            subscription_id: "display-1".to_string(),
            target: target(),
            session_id: "session-1".to_string(),
            attachment_id: "display".to_string(),
            client_public_key: "client-public".to_string(),
            subscription_scope: Some("kernel_browser_display".to_string()),
            resume_from_event_id: None,
            heartbeat_interval_ms: None,
        },
    )
    .await;
    let relay_request_id = match receive(&mut daemon, &mut daemon_tally).await.0 {
        RelayEnvelope::DaemonSubscribe {
            relay_request_id, ..
        } => relay_request_id,
        other => panic!("unexpected daemon frame: {other:?}"),
    };
    send(
        &mut daemon,
        &mut daemon_tally,
        &RelayEnvelope::DaemonResponse {
            relay_request_id,
            encrypted_response: None,
            error: None,
        },
    )
    .await;
    receive(&mut client, &mut client_tally).await;

    let mut event_in = 0;
    let mut event_out = 0;
    for event_id in 1..=3 {
        let mut before = daemon_tally.sent;
        send(
            &mut daemon,
            &mut daemon_tally,
            &RelayEnvelope::DaemonEvent {
                subscription_id: "display-1".to_string(),
                event_id,
                encrypted_event: payload(2_000 * event_id as usize),
            },
        )
        .await;
        event_in += daemon_tally.sent - before;
        before = client_tally.received;
        receive(&mut client, &mut client_tally).await;
        event_out += client_tally.received - before;
    }

    sleep(Duration::from_millis(100)).await;
    let counters = meter.snapshot();
    let display = counters
        .get(&(
            DEFAULT_RELAY_REALM_ID.to_string(),
            "kernel_browser_display".to_string(),
        ))
        .expect("display scope should be counted");
    assert_eq!(
        *display,
        UsageCounters {
            bytes_in: event_in,
            bytes_out: event_out,
            frames_in: 3,
            frames_out: 3
        }
    );
    let sum = |select: fn(&UsageCounters) -> u64| counters.values().map(select).sum::<u64>();
    assert_eq!(sum(|c| c.bytes_in), daemon_tally.sent + client_tally.sent);
    assert_eq!(
        sum(|c| c.bytes_out),
        daemon_tally.received + client_tally.received
    );
    assert!(counters
        .get(&(
            DEFAULT_RELAY_REALM_ID.to_string(),
            CONTROL_SCOPE.to_string()
        ))
        .is_some_and(|control| control.bytes_in > 0 && control.bytes_out > 0));

    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

/// A daemon reply sent as one binary frame is counted once, at the size it had on the wire.
#[tokio::test(flavor = "multi_thread")]
async fn a_binary_daemon_reply_is_counted_once_at_its_wire_size() {
    let probe = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = probe
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let meter = server.usage_meter();
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
    let mut daemon_tally = Tally {
        sent: 0,
        received: 0,
    };
    let mut client_tally = Tally {
        sent: 0,
        received: 0,
    };
    let (mut daemon, _) = connect_async(crate::binary_event::connection_request(&url).unwrap())
        .await
        .expect("binary daemon should connect");
    send(
        &mut daemon,
        &mut daemon_tally,
        &RelayEnvelope::DaemonRegister {
            registration: test_registration("daemon-1", "machine-1", "Linux", 10),
        },
    )
    .await;
    sleep(Duration::from_millis(50)).await;
    let (mut client, _) = connect_async_with_retry(&url)
        .await
        .expect("client should connect");
    send(
        &mut client,
        &mut client_tally,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: target(),
        },
    )
    .await;
    receive(&mut client, &mut client_tally).await;
    send(
        &mut client,
        &mut client_tally,
        &RelayEnvelope::ClientRequest {
            request_id: "request-1".to_string(),
            target: target(),
            encrypted_request: payload(100),
        },
    )
    .await;
    let relay_request_id = match receive(&mut daemon, &mut daemon_tally).await.0 {
        RelayEnvelope::DaemonRequest {
            relay_request_id, ..
        } => relay_request_id,
        other => panic!("unexpected daemon frame: {other:?}"),
    };
    let reply = crate::binary_response::from_envelope(&RelayEnvelope::DaemonResponse {
        relay_request_id,
        encrypted_response: Some(EncryptedRelayPayload {
            sender_public_key: "opaque-key".to_string(),
            nonce: "abcdefghijklmnop".to_string(),
            ciphertext: base64::engine::general_purpose::STANDARD.encode(vec![0xfe; 5_000]),
        }),
        error: None,
    })
    .unwrap()
    .expect("a response with ciphertext has a binary form");
    daemon_tally.sent += reply.len() as u64;
    daemon.send(Message::Binary(reply.into())).await.unwrap();
    receive(&mut client, &mut client_tally).await;

    sleep(Duration::from_millis(100)).await;
    let counted = meter.snapshot();
    let control = &counted[&(
        DEFAULT_RELAY_REALM_ID.to_string(),
        CONTROL_SCOPE.to_string(),
    )];
    assert_eq!(control.bytes_in, daemon_tally.sent + client_tally.sent);
    assert_eq!(
        control.bytes_out,
        daemon_tally.received + client_tally.received
    );
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

/// Two accounts on one relay are counted apart: each connection's bytes land under the realm its own token names.
#[tokio::test(flavor = "multi_thread")]
async fn two_realms_on_one_relay_are_counted_apart() {
    let issuer_secret = "hosted-issuer-secret";
    let mut issuers = BTreeMap::new();
    issuers.insert("hosted-cloud".to_string(), issuer_secret.to_string());
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: None,
        },
        RelayAuthVerifier::scoped_hmac(issuers, Some(10_000)),
    );
    let meter = server.usage_meter();
    let listener = server.bind_listener().await.expect("relay listener");
    let addr = listener.local_addr().expect("relay address");
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

    let mut sent = BTreeMap::new();
    let mut sockets = Vec::new();
    for (realm, heartbeats) in [("realm-a", 3), ("realm-b", 1)] {
        let public_key = format!("{realm}-public-key");
        let mut claims = scoped_claim(
            &format!("{realm}-token"),
            &format!("{realm}-kernel"),
            RelaySubjectKind::Kernel,
            realm,
            vec![RelayAction::DaemonRegister],
            None,
        );
        claims.issuer = "hosted-cloud".to_string();
        claims.expires_at_ms = 20_000;
        claims.account_id = Some(format!("account-{realm}"));
        claims.user_id = Some("user".to_string());
        claims.machine_id = Some("machine".to_string());
        claims.public_key_thumbprint = Some({
            use sha2::Digest;
            Sha256::digest(public_key.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        });
        let mut registration =
            test_registration(&format!("{realm}-kernel"), "machine", "Linux", 10);
        registration.auth_token = hosted_kernel_jwt(&claims, issuer_secret);
        registration.public_key = public_key;
        let mut tally = Tally {
            sent: 0,
            received: 0,
        };
        let (mut socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon connects");
        send(
            &mut socket,
            &mut tally,
            &RelayEnvelope::DaemonRegister { registration },
        )
        .await;
        for _ in 0..heartbeats {
            send(
                &mut socket,
                &mut tally,
                &RelayEnvelope::DaemonHeartbeat {
                    daemon_id: format!("{realm}-kernel"),
                    registration: None,
                },
            )
            .await;
        }
        sent.insert(realm.to_string(), tally.sent);
        sockets.push(socket);
    }

    sleep(Duration::from_millis(200)).await;
    let counted = meter.snapshot();
    let realms: Vec<_> = counted.keys().map(|(realm, _)| realm.as_str()).collect();
    assert_eq!(realms, ["realm-a", "realm-b"]);
    for (realm, bytes) in sent {
        let total: u64 = counted
            .iter()
            .filter(|((r, _), _)| *r == realm)
            .map(|(_, c)| c.bytes_in)
            .sum();
        assert_eq!(total, bytes, "bytes counted for {realm}");
    }
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

/// A connection that only asks for metadata is authenticated by the token in each request, and its bytes count for that realm.
#[tokio::test(flavor = "multi_thread")]
async fn a_metadata_only_connection_is_counted_for_its_realm() {
    let probe = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = probe
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let meter = server.usage_meter();
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
    let mut tally = Tally {
        sent: 0,
        received: 0,
    };
    let (mut client, _) = connect_async_with_retry(&url)
        .await
        .expect("client should connect");
    send(
        &mut client,
        &mut tally,
        &RelayEnvelope::ClientMetadataRequest {
            request_id: "machines".to_string(),
            auth_token: "secret".to_string(),
            query: RelayMetadataQuery::ListLiveMachines,
        },
    )
    .await;
    receive(&mut client, &mut tally).await;
    drop(client);
    sleep(Duration::from_millis(200)).await;

    let counted = meter.snapshot();
    let control = counted
        .get(&(
            DEFAULT_RELAY_REALM_ID.to_string(),
            CONTROL_SCOPE.to_string(),
        ))
        .expect("the realm's bytes reach the meter, not only the connection");
    assert_eq!(control.bytes_in, tally.sent);
    assert_eq!(control.bytes_out, tally.received);
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

/// A connection belongs to the first realm it authenticates in. A later request
/// that proves another realm is refused, so no traffic of realm B is ever
/// counted for realm A.
#[tokio::test(flavor = "multi_thread")]
async fn a_connection_that_authenticated_one_realm_refuses_another() {
    let mut claims = BTreeMap::new();
    for realm in ["realm-a", "realm-b"] {
        claims.insert(
            format!("client-{realm}-token"),
            scoped_claim(
                &format!("client-{realm}-token"),
                &format!("client-{realm}"),
                RelaySubjectKind::Client,
                realm,
                vec![RelayAction::ClientConnect, RelayAction::ClientMetadataRead],
                None,
            ),
        );
    }
    let auth_verifier =
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), Some(10)));
    let probe = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: None,
        },
        auth_verifier.clone(),
    );
    let listener = probe.bind_listener().await.expect("relay listener");
    let addr = listener.local_addr().expect("relay address");
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: None,
        },
        auth_verifier,
    );
    let meter = server.usage_meter();
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
    let metadata = |realm: &str| RelayEnvelope::ClientMetadataRequest {
        request_id: format!("machines-{realm}"),
        auth_token: format!("client-{realm}-token"),
        query: RelayMetadataQuery::ListLiveMachines,
    };
    let connect = |realm: &str| RelayEnvelope::ClientConnect {
        auth_token: format!("client-{realm}-token"),
        target: target(),
    };

    for (second_name, second) in [
        ("metadata", metadata("realm-b")),
        ("connect", connect("realm-b")),
    ] {
        let mut tally = Tally {
            sent: 0,
            received: 0,
        };
        let (mut client, _) = connect_async_with_retry(&url)
            .await
            .expect("client should connect");
        send(&mut client, &mut tally, &metadata("realm-a")).await;
        let (answer, _) = receive(&mut client, &mut tally).await;
        assert!(
            matches!(answer, RelayEnvelope::ClientMetadataResponse { .. }),
            "{second_name}: realm A's own request is answered"
        );
        send(&mut client, &mut tally, &second).await;
        let (refusal, _) = receive(&mut client, &mut tally).await;
        match refusal {
            RelayEnvelope::Close { reason, .. } => assert!(
                reason.contains("another realm"),
                "{second_name}: refused for its realm, not for something else: {reason}"
            ),
            other => panic!("{second_name}: realm B must be refused, got {other:?}"),
        }
    }
    sleep(Duration::from_millis(200)).await;
    let counted = meter.snapshot();
    assert!(
        counted.keys().all(|(realm, _)| realm == "realm-a"),
        "nothing is counted for a realm whose request was refused: {:?}",
        counted.keys().collect::<Vec<_>>()
    );
    assert!(!counted.is_empty(), "realm A's own bytes are counted");
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}

/// A kernel registration is checked against the connection's realm before it
/// changes anything: a socket bound to realm A cannot replace realm B's
/// daemon, and realm B's traffic is never counted for A.
#[tokio::test(flavor = "multi_thread")]
async fn a_connection_bound_to_one_realm_cannot_register_a_daemon_of_another() {
    let mut claims = BTreeMap::new();
    for realm in ["realm-a", "realm-b"] {
        claims.insert(
            format!("client-{realm}-token"),
            scoped_claim(
                &format!("client-{realm}-token"),
                &format!("client-{realm}"),
                RelaySubjectKind::Client,
                realm,
                vec![RelayAction::ClientConnect, RelayAction::ClientMetadataRead],
                None,
            ),
        );
        claims.insert(
            format!("kernel-{realm}-token"),
            scoped_claim(
                &format!("kernel-{realm}-token"),
                &format!("kernel-{realm}"),
                RelaySubjectKind::Kernel,
                realm,
                vec![RelayAction::DaemonRegister],
                None,
            ),
        );
    }
    let auth_verifier =
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), Some(10)));
    let probe = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: None,
        },
        auth_verifier.clone(),
    );
    let listener = probe.bind_listener().await.expect("relay listener");
    let addr = listener.local_addr().expect("relay address");
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
    let mut tally = Tally {
        sent: 0,
        received: 0,
    };
    let register = |realm: &str| RelayEnvelope::DaemonRegister {
        registration: test_registration_with_token(
            &format!("kernel-{realm}"),
            "machine",
            "Linux",
            10,
            &format!("kernel-{realm}-token"),
        ),
    };
    let metadata = |realm: &str| RelayEnvelope::ClientMetadataRequest {
        request_id: format!("machines-{realm}"),
        auth_token: format!("client-{realm}-token"),
        query: RelayMetadataQuery::ListLiveMachines,
    };

    // Realm B's kernel is live on its own connection, and realm A has one for a ClientConnect.
    let (mut kernel_b, _) = connect_async_with_retry(&url).await.expect("kernel B");
    send(&mut kernel_b, &mut tally, &register("realm-b")).await;
    let (mut kernel_a, _) = connect_async_with_retry(&url).await.expect("kernel A");
    send(&mut kernel_a, &mut tally, &register("realm-a")).await;
    sleep(Duration::from_millis(200)).await;

    for first_name in ["metadata", "connect"] {
        let (mut socket, _) = connect_async_with_retry(&url).await.expect("socket");
        if first_name == "metadata" {
            send(&mut socket, &mut tally, &metadata("realm-a")).await;
            let (answer, _) = receive(&mut socket, &mut tally).await;
            assert!(matches!(
                answer,
                RelayEnvelope::ClientMetadataResponse { .. }
            ));
        } else {
            send(
                &mut socket,
                &mut tally,
                &RelayEnvelope::ClientConnect {
                    auth_token: "client-realm-a-token".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("kernel-realm-a".to_string()),
                        daemon_alias: None,
                    },
                },
            )
            .await;
            let (answer, _) = receive(&mut socket, &mut tally).await;
            assert!(
                matches!(answer, RelayEnvelope::ClientConnected { .. }),
                "realm A's connect is admitted: {answer:?}"
            );
        }
        send(&mut socket, &mut tally, &register("realm-b")).await;
        let (refusal, _) = receive(&mut socket, &mut tally).await;
        match refusal {
            RelayEnvelope::Close { reason, .. } => assert!(
                reason.contains("another realm"),
                "{first_name}: refused for its realm: {reason}"
            ),
            other => panic!("{first_name}: realm B's registration must be refused, got {other:?}"),
        }
        // The refusal left realm B's own registration alone.
        assert!(
            timeout(Duration::from_millis(300), kernel_b.next())
                .await
                .is_err(),
            "{first_name}: realm B's kernel was not replaced"
        );
    }

    let (mut probe_b, _) = connect_async_with_retry(&url).await.expect("probe B");
    send(&mut probe_b, &mut tally, &metadata("realm-b")).await;
    match receive(&mut probe_b, &mut tally).await.0 {
        RelayEnvelope::ClientMetadataResponse { machines, .. } => assert_eq!(
            machines.map(|machines| machines.len()),
            Some(1),
            "realm B's kernel is still live"
        ),
        other => panic!("unexpected answer: {other:?}"),
    }
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}
