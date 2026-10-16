//! MP-08/MP-10/MP-11: real WebSocket transport/admission regression, supplementary.
use super::*;
use crate::binary_event;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn send(socket: &mut Socket, envelope: RelayEnvelope) {
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope).unwrap().into(),
        ))
        .await
        .unwrap();
}
async fn receive(socket: &mut Socket) -> Message {
    timeout(Duration::from_secs(3), async {
        loop {
            let frame = socket.next().await.unwrap().unwrap();
            if !matches!(frame, Message::Ping(_) | Message::Pong(_)) {
                return frame;
            }
        }
    })
    .await
    .expect("MP-10: bounded transport reply")
}
async fn envelope(socket: &mut Socket) -> RelayEnvelope {
    let Message::Text(text) = receive(socket).await else {
        panic!("MP-11: expected control envelope")
    };
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn mp08_peer96_binary_events_negotiate_per_connection_and_downgrade_for_either_legacy_peer() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    for daemon_binary in [false, true] {
        for client_binary in [false, true] {
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
            let request = if daemon_binary {
                binary_event::connection_request(&url).unwrap()
            } else {
                url.as_str().into_client_request().unwrap()
            };
            let (mut daemon, handshake) = connect_async(request).await.unwrap();
            assert_eq!(
                handshake
                    .headers()
                    .contains_key(binary_event::VERSION_HEADER),
                daemon_binary
            );
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
            let mut request = url.as_str().into_client_request().unwrap();
            if client_binary {
                request.headers_mut().insert(
                    "sec-websocket-protocol",
                    binary_event::CAPABILITY.parse().unwrap(),
                );
            }
            let (mut client, handshake) = connect_async(request).await.unwrap();
            assert_eq!(
                handshake.headers().get("sec-websocket-protocol").is_some(),
                client_binary
            );
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
            assert!(matches!(
                envelope(&mut client).await,
                RelayEnvelope::ClientConnected { .. }
            ));
            send(
                &mut client,
                RelayEnvelope::ClientSubscribe {
                    request_id: "subscribe".into(),
                    subscription_id: "sub".into(),
                    target,
                    session_id: "session".into(),
                    attachment_id: "terminal".into(),
                    client_public_key: "opaque-client-key".into(),
                    subscription_scope: None,
                    resume_from_event_id: None,
                },
            )
            .await;
            let RelayEnvelope::DaemonSubscribe {
                relay_request_id, ..
            } = envelope(&mut daemon).await
            else {
                panic!("MP-11: subscription admission")
            };
            send(
                &mut daemon,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: None,
                    error: None,
                },
            )
            .await;
            assert!(matches!(
                envelope(&mut client).await,
                RelayEnvelope::ClientResponse { error: None, .. }
            ));
            let ciphertext = vec![37; 1024];
            let event = RelayEnvelope::DaemonEvent {
                subscription_id: "sub".into(),
                event_id: 7,
                encrypted_event: EncryptedRelayPayload {
                    sender_public_key: "opaque-kernel-key".into(),
                    nonce: "abcdefghijklmnop".into(),
                    ciphertext: base64::engine::general_purpose::STANDARD.encode(&ciphertext),
                },
            };
            if daemon_binary {
                daemon
                    .send(Message::Binary(
                        binary_event::from_envelope(&event).unwrap().unwrap().into(),
                    ))
                    .await
                    .unwrap()
            } else {
                send(&mut daemon, event).await
            }
            let received = receive(&mut client).await;
            if daemon_binary && client_binary {
                let Message::Binary(bytes) = received else {
                    panic!("MP-10: negotiated opaque binary event")
                };
                let (header, raw) = binary_event::decode(&bytes).unwrap();
                assert_eq!(header.kind, "client_event");
                assert_eq!(header.event_id, 7);
                assert_eq!(raw, ciphertext);
            } else {
                let Message::Text(text) = received else {
                    panic!("MP-08: legacy event framing")
                };
                let RelayEnvelope::ClientEvent {
                    event_id,
                    encrypted_event,
                    ..
                } = serde_json::from_str(&text).unwrap()
                else {
                    panic!("MP-11: client route")
                };
                assert_eq!(event_id, 7);
                assert_eq!(
                    base64::engine::general_purpose::STANDARD
                        .decode(encrypted_event.ciphertext)
                        .unwrap(),
                    ciphertext
                );
            }
            client.close(None).await.unwrap();
            daemon.close(None).await.unwrap();
            stop.send(()).unwrap();
            task.await.unwrap();
        }
    }
}
