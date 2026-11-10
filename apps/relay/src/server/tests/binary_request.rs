//! A request's ciphertext crosses the relay as bytes toward a daemon that offered binary
//! requests, from a client that offered them or not, and as JSON toward everyone else.
use super::*;
use crate::{binary_event, binary_request};
use base64::Engine;
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
    .expect("a bounded reply")
}
async fn envelope(socket: &mut Socket) -> RelayEnvelope {
    let Message::Text(text) = receive(socket).await else {
        panic!("expected a control envelope")
    };
    serde_json::from_str(&text).unwrap()
}

fn request_payload(ciphertext: &[u8]) -> EncryptedRelayPayload {
    EncryptedRelayPayload {
        sender_public_key: "opaque-client-key".into(),
        nonce: "abcdefghijklmnop".into(),
        ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
    }
}

#[tokio::test]
async fn a_request_is_binary_exactly_where_the_connection_offered_it() {
    for daemon_offers in [false, true] {
        for client_offers in [false, true] {
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
            let handshake_request = |offers: bool| {
                let request = binary_event::connection_request(&url).unwrap();
                if offers {
                    binary_request::offer(request)
                } else {
                    request
                }
            };
            // A kernel takes large requests in chunks, as every kernel connects.
            let (mut daemon, handshake) = connect_async(
                binary_event::connection_request(&crate::frame_transport::transport_url(&url))
                    .map(|request| {
                        if daemon_offers {
                            binary_request::offer(request)
                        } else {
                            request
                        }
                    })
                    .unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(
                handshake.headers().contains_key(binary_request::HEADER),
                daemon_offers,
                "the relay answers the offer it was given"
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
            let (mut client, handshake) = connect_async(handshake_request(client_offers))
                .await
                .unwrap();
            assert_eq!(
                handshake.headers().contains_key(binary_request::HEADER),
                client_offers
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
            let ciphertext = vec![0xa5; 40 * 1024];
            let request = RelayEnvelope::ClientRequest {
                request_id: "put-1".into(),
                target,
                encrypted_request: request_payload(&ciphertext),
            };
            if client_offers {
                let frame = binary_request::from_envelope(&request).unwrap().unwrap();
                assert!(
                    frame.len() < ciphertext.len() + 1024,
                    "the client's request is not inflated"
                );
                client.send(Message::Binary(frame.into())).await.unwrap();
            } else {
                send(&mut client, request).await;
            }
            let mut chunks = crate::frame_transport::FrameReceiver::default();
            let received = timeout(
                Duration::from_secs(3),
                crate::frame_transport::read_message(&mut daemon, &mut chunks),
            )
            .await
            .expect("a bounded request")
            .unwrap()
            .unwrap();
            let delivered = match (daemon_offers, received) {
                (true, Message::Binary(bytes)) => {
                    assert!(
                        bytes.len() < ciphertext.len() + 2048,
                        "the daemon's request is not inflated: {} bytes",
                        bytes.len()
                    );
                    binary_request::into_envelope(&bytes).unwrap()
                }
                (false, Message::Text(text)) => serde_json::from_str(&text).unwrap(),
                (offered, other) => panic!("daemon offered {offered} and got {other:?}"),
            };
            let RelayEnvelope::DaemonRequest {
                relay_request_id,
                encrypted_request,
                ..
            } = delivered
            else {
                panic!("the daemon is sent a daemon request")
            };
            assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .decode(encrypted_request.ciphertext)
                    .unwrap(),
                ciphertext
            );
            send(
                &mut daemon,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: None,
                    error: None,
                },
            )
            .await;
            let RelayEnvelope::ClientResponse { request_id, .. } = envelope(&mut client).await
            else {
                panic!("the client is answered")
            };
            assert_eq!(request_id, "put-1");
            client.close(None).await.unwrap();
            daemon.close(None).await.unwrap();
            let _ = stop.send(());
            task.await.unwrap();
        }
    }
}

#[tokio::test]
async fn a_binary_request_without_the_offer_ends_the_connection() {
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".into(),
        port: 0,
        shared_token: Some("secret".into()),
    });
    let listener = server.bind_listener().await.unwrap();
    let addr = listener.local_addr().unwrap();
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
    let (mut client, _) = connect_async(binary_event::connection_request(&url).unwrap())
        .await
        .unwrap();
    let frame = binary_request::from_envelope(&RelayEnvelope::ClientRequest {
        request_id: "put-1".into(),
        target: ClientTarget {
            daemon_id: Some("kernel".into()),
            daemon_alias: None,
        },
        encrypted_request: request_payload(&[1; 64]),
    })
    .unwrap()
    .unwrap();
    client.send(Message::Binary(frame.into())).await.unwrap();
    let ended = timeout(Duration::from_secs(3), async {
        loop {
            match client.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(
        ended.is_ok(),
        "the relay refuses an unnegotiated binary request"
    );
    let _ = stop.send(());
    task.await.unwrap();
}
