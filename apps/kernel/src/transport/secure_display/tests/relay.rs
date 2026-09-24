use super::*;
use chariox_relay::protocol::{
    RelayDisplayTunnelResponseStart, RelayDisplayTunnelStreamChunk, RelayEnvelope,
};
use chariox_relay::{RelayConfig, RelayServer};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
async fn encrypted_display_fragments_round_trip_through_real_relay_websockets() {
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_owned(),
        port: 0,
        shared_token: Some("drill-token".to_owned()),
    });
    let listener = server.bind_listener().await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
    });
    let run = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let (mut daemon, _) = connect_async(format!("ws://{address}")).await.unwrap();
        daemon.send(Message::Text(serde_json::json!({
            "kind": "daemon_register", "registration": {
                "auth_token": "drill-token", "daemon_id": "display-kernel", "machine_id": "local-drill",
                "public_key": "registration-key", "capabilities": [],
            }
        }).to_string().into())).await.unwrap();
        daemon.send(Message::Text(serde_json::json!({
            "kind": "daemon_display_tunnel_register", "registration": {
                "tunnel_id": "encrypted-display", "expires_at_ms": u64::MAX,
                "capabilities": ["view", "encrypted_websocket"],
            }
        }).to_string().into())).await.unwrap();
        let registered = daemon.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(matches!(serde_json::from_str::<RelayEnvelope>(&registered).unwrap(),
            RelayEnvelope::DaemonDisplayTunnelRegistered { error: None, .. }));

        let browser_connection = connect_async(format!("ws://{address}/display/encrypted-display/stream"));
        let admit_transport = async {
            let incoming = daemon.next().await.unwrap().unwrap().into_text().unwrap();
            let RelayEnvelope::DaemonDisplayTunnelOpen { request } = serde_json::from_str(&incoming).unwrap() else {
                panic!("expected display transport open");
            };
            daemon.send(Message::Text(serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelResponseStart {
                response: RelayDisplayTunnelResponseStart { stream_id: request.stream_id.clone(), status: 101, headers: vec![] },
            }).unwrap().into())).await.unwrap();
            request.stream_id
        };
        let (browser, stream_id) = tokio::join!(browser_connection, admit_transport);
        let (mut browser, _) = browser.unwrap();
        let (mut kernel_cipher, mut viewer_cipher) = channels("admitted-viewer-connection");
        let frame = vec![0x91; 200_000];
        let fragments = kernel_cipher.encode(DisplayMessageKind::Binary, &frame).unwrap();
        let mut decoded = None;
        for fragment in fragments {
            let ciphertext = serde_json::to_vec(&fragment).unwrap();
            daemon.send(Message::Text(serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelChunk {
                chunk: RelayDisplayTunnelStreamChunk {
                    stream_id: stream_id.clone(), data: BASE64.encode(&ciphertext), message_kind: Some("text".to_owned()),
                },
            }).unwrap().into())).await.unwrap();
            let received = browser.next().await.unwrap().unwrap().into_text().unwrap();
            assert_eq!(received.as_bytes(), ciphertext, "relay must preserve opaque encrypted bytes");
            let packet = serde_json::from_str(&received).unwrap();
            if let Some(message) = viewer_cipher.decode(&packet).unwrap() {
                assert!(decoded.replace(message).is_none());
            }
        }
        assert_eq!(decoded.unwrap().data, frame);

        let ack = viewer_cipher.encode(DisplayMessageKind::Text, b"CLIENT_FRAME_ACK 7").unwrap().remove(0);
        let wire_ack = serde_json::to_string(&ack).unwrap();
        assert!(!wire_ack.contains("CLIENT_FRAME_ACK"));
        browser.send(Message::Text(wire_ack.into())).await.unwrap();
        let incoming = daemon.next().await.unwrap().unwrap().into_text().unwrap();
        let RelayEnvelope::DaemonDisplayTunnelClientChunk { chunk } = serde_json::from_str(&incoming).unwrap() else {
            panic!("expected opaque viewer reply");
        };
        assert_eq!(chunk.stream_id, stream_id);
        let packet = serde_json::from_slice(&BASE64.decode(chunk.data).unwrap()).unwrap();
        assert_eq!(kernel_cipher.decode(&packet).unwrap().unwrap().data, b"CLIENT_FRAME_ACK 7");
        browser.close(None).await.unwrap();
        daemon.close(None).await.unwrap();
    }).await;
    let _ = shutdown_tx.send(());
    tokio::time::timeout(std::time::Duration::from_secs(5), server_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    run.expect("encrypted display drill exceeded bounded timeout");
}
