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
