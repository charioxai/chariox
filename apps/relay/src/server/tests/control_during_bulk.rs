//! A control request on a connection that is receiving a bulk answer is answered while the bulk
//! transfer is still in flight, whatever pace the receiver drains it at.
use super::*;
use crate::frame_transport::{transport_url, FrameReceiver, TransportFrame};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn payload(len: usize) -> EncryptedRelayPayload {
    EncryptedRelayPayload {
        sender_public_key: "opaque-kernel-key".into(),
        nonce: "abcdefghijklmnop".into(),
        ciphertext: base64::engine::general_purpose::STANDARD.encode(vec![0x5a; len]),
    }
}

async fn send(socket: &mut Socket, envelope: RelayEnvelope) {
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope).unwrap().into(),
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_control_request_is_answered_while_a_bulk_answer_drains_slowly() {
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".into(),
        port: 0,
        shared_token: Some("secret".into()),
    });
    let listener = server.bind_listener().await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = server.registry();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = stopped.await;
            })
            .await
            .unwrap()
    });
    let url = format!("ws://{addr}");
    let (mut daemon, _) = connect_async(url.as_str()).await.unwrap();
    send(
        &mut daemon,
        RelayEnvelope::DaemonRegister {
            registration: test_registration("kernel", "machine", "Linux", 10),
        },
    )
    .await;
    timeout(Duration::from_secs(3), async {
        while registry
            .read()
            .await
            .peers
            .values()
            .all(|p| p.daemon_registration.is_none())
        {
            sleep(Duration::from_millis(1)).await
        }
    })
    .await
    .unwrap();
    // The daemon answers every request: a 3 MiB file for "bulk", a few bytes for the rest.
    let daemon_task = tokio::spawn(async move {
        loop {
            let Some(Ok(Message::Text(text))) = daemon.next().await else {
                return daemon;
            };
            let Ok(RelayEnvelope::DaemonRequest {
                relay_request_id,
                encrypted_request,
                ..
            }) = serde_json::from_str(&text)
            else {
                continue;
            };
            let len = if encrypted_request.nonce.starts_with("bulk") {
                3 * 1024 * 1024
            } else {
                64
            };
            send(
                &mut daemon,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: Some(payload(len)),
                    error: None,
                },
            )
            .await;
        }
    });
    let (mut client, _) = connect_async(transport_url(&url)).await.unwrap();
    let target = ClientTarget {
        daemon_id: Some("kernel".into()),
        daemon_alias: None,
    };
    send(
        &mut client,
        RelayEnvelope::ClientConnect {
            auth_token: "secret".into(),
            target: target.clone(),
        },
    )
    .await;
    let request = |id: &str, kind: &str| RelayEnvelope::ClientRequest {
        request_id: id.into(),
        target: target.clone(),
        encrypted_request: EncryptedRelayPayload {
            sender_public_key: "opaque-client-key".into(),
            nonce: format!("{kind}-nonce-000000")[..16].into(),
            ciphertext: base64::engine::general_purpose::STANDARD.encode([1u8; 32]),
        },
    };
    // A receiver on a slow link: each chunk is acknowledged 80 ms after it arrived.
    let mut receiver = FrameReceiver::default();
    let mut asked_at = None;
    let mut answered_in = None;
    let mut sent_bulk = false;
    let started = tokio::time::Instant::now();
    while answered_in.is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the control request was not answered while the bulk answer was in flight"
        );
        let message = timeout(Duration::from_secs(5), client.next())
            .await
            .expect("the relay keeps writing")
            .unwrap()
            .unwrap();
        let Message::Text(text) = message else {
            continue;
        };
        let (complete, receipt) = receiver.receive(&text).unwrap();
        if let Some(receipt) = receipt {
            let _ = serde_json::from_str::<TransportFrame>(&receipt).unwrap();
            sleep(Duration::from_millis(80)).await;
            client.send(Message::Text(receipt.into())).await.unwrap();
        }
        let envelope = complete.and_then(|text| serde_json::from_str::<RelayEnvelope>(&text).ok());
        match envelope {
            Some(RelayEnvelope::ClientConnected { .. }) if !sent_bulk => {
                sent_bulk = true;
                send(&mut client, request("bulk-1", "bulk")).await;
            }
            Some(RelayEnvelope::ClientResponse { request_id, .. }) if request_id == "control-1" => {
                answered_in = Some(asked_at.take().unwrap_or(started).elapsed());
            }
            Some(RelayEnvelope::ClientResponse { request_id, .. }) if request_id == "bulk-1" => {
                panic!("the bulk answer finished before the control request was answered");
            }
            _ => {}
        }
        if asked_at.is_none()
            && started.elapsed() >= Duration::from_secs(3)
            && answered_in.is_none()
        {
            asked_at = Some(tokio::time::Instant::now());
            send(&mut client, request("control-1", "control")).await;
        }
    }
    let answered_in = answered_in.unwrap();
    assert!(
        answered_in < Duration::from_millis(500),
        "the control request waited {answered_in:?} behind the bulk answer"
    );
    daemon_task.abort();
    client.close(None).await.ok();
    let _ = stop.send(());
    task.await.unwrap();
}
