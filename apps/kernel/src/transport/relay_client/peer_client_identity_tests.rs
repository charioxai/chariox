use super::*;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, WebSocketStream};

// The relay's claimed sender and the encrypted sender must both match discovery.
// This peer-client test injects hostile wire responses, not mocked kernel code.
#[tokio::test]
async fn temporary_peer_request_rejects_response_from_another_key() {
    rejects_mismatched_identity(true).await;
}

#[tokio::test]
async fn temporary_peer_request_rejects_response_from_another_kernel() {
    rejects_mismatched_identity(false).await;
}

async fn receive(socket: &mut WebSocketStream<TcpStream>) -> RelayEnvelope {
    let message = timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

async fn send(socket: &mut WebSocketStream<TcpStream>, envelope: RelayEnvelope) {
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope).unwrap().into(),
        ))
        .await
        .unwrap();
}

async fn rejects_mismatched_identity(wrong_key: bool) {
    let mut home = crate::config::DaemonConfig::for_tests();
    let worker = crate::config::DaemonConfig::for_tests();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    home.relay_url = Some(format!("ws://{}", listener.local_addr().unwrap()));
    home.relay_token = Some("identity-fixture".into());
    let home_key = home.relay_public_key.clone();
    let server = tokio::spawn(async move {
        let mut metadata = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        let RelayEnvelope::ClientMetadataRequest { request_id, .. } = receive(&mut metadata).await
        else {
            panic!("expected discovery request");
        };
        let presence = serde_json::from_value(serde_json::json!({
            "kernel_id":"worker", "machine_id":"slice:fixture", "public_key":worker.relay_public_key
        }))
        .unwrap();
        send(
            &mut metadata,
            RelayEnvelope::ClientMetadataResponse {
                request_id,
                machines: None,
                kernels: None,
                kernel: Some(presence),
                error: None,
            },
        )
        .await;
        assert!(matches!(metadata.next().await, Some(Ok(Message::Close(_)))));
        let _ = metadata.close(None).await;
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        assert!(matches!(
            receive(&mut socket).await,
            RelayEnvelope::DaemonRegister { .. }
        ));
        let RelayEnvelope::DaemonPeerRequest {
            request_id,
            encrypted_request,
            ..
        } = receive(&mut socket).await
        else {
            panic!("expected encrypted peer request");
        };
        relay_crypto::decrypt_payload_for_private_key(
            &worker.relay_private_key,
            &encrypted_request,
        )
        .unwrap();
        let response_key = if wrong_key {
            relay_crypto::generate_private_key_base64()
        } else {
            worker.relay_private_key
        };
        let response = RelayPeerResponse::Pong {
            value: "reply".into(),
            daemon_id: "worker".into(),
        };
        send(
            &mut socket,
            RelayEnvelope::DaemonPeerResponse {
                request_id,
                from_daemon_id: if wrong_key {
                    "worker".into()
                } else {
                    "other-worker".into()
                },
                encrypted_response: Some(
                    relay_crypto::encrypt_payload_for_peer(
                        &response_key,
                        &home_key,
                        &serde_json::to_vec(&response).unwrap(),
                    )
                    .unwrap(),
                ),
                error: None,
            },
        )
        .await;
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        let _ = socket.close(None).await;
    });
    let response = send_peer_request_via_temporary_connection_with_timeout(
        &home,
        ClientTarget {
            daemon_id: Some("worker".into()),
            daemon_alias: None,
        },
        RelayPeerRequest::Ping {
            value: "request".into(),
        },
        Duration::from_secs(3),
    )
    .await;
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    let error = response.expect_err("mismatched response identity must not be accepted");
    assert!(
        error
            .to_string()
            .contains("peer response identity mismatch"),
        "{error}"
    );
}
