//! Row 207: a peer request or reply crosses the relay as bytes exactly where both kernels offered
//! binary peer messages, and as JSON toward a kernel that did not (it still works).
use super::*;
use crate::frame_transport::FrameReceiver;
use crate::{binary_event, binary_request, binary_response};
use base64::Engine;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

enum Got {
    Text(String),
    Binary(Vec<u8>),
}

/// One complete message and the bytes it took on the socket, reassembling chunks and
/// acknowledging them as a kernel does.
async fn next_message(socket: &mut Socket, frames: &mut FrameReceiver, wire: &mut usize) -> Got {
    timeout(Duration::from_secs(10), async {
        loop {
            let frame = socket.next().await.unwrap().unwrap();
            let (complete, receipt) = match frame {
                Message::Text(text) => {
                    *wire += text.len();
                    let (text, receipt) = frames.receive(&text).unwrap();
                    (text.map(Got::Text), receipt)
                }
                Message::Binary(bytes) => {
                    *wire += bytes.len();
                    let (bytes, receipt) = frames.receive_binary(&bytes).unwrap();
                    (bytes.map(Got::Binary), receipt)
                }
                _ => continue,
            };
            if let Some(receipt) = receipt {
                socket.send(Message::Text(receipt.into())).await.unwrap();
            }
            if let Some(complete) = complete {
                return complete;
            }
        }
    })
    .await
    .expect("a bounded message")
}

async fn kernel(url: &str, id: &str, offers_peer: bool) -> Socket {
    let mut request = binary_request::offer(
        binary_event::connection_request(&crate::frame_transport::transport_url(url)).unwrap(),
    );
    if !offers_peer {
        // A kernel from before the peer offer: binary requests and chunked uploads only.
        request.headers_mut().remove(binary_request::PEER_HEADER);
    }
    let (mut socket, handshake) = connect_async(request).await.unwrap();
    assert_eq!(
        handshake
            .headers()
            .contains_key(binary_request::PEER_HEADER),
        offers_peer,
        "the relay answers the peer offer it was given"
    );
    let register = signed_register(
        url,
        &handshake,
        test_registration(id, &format!("machine-{id}"), "Linux", 10),
    );
    socket
        .send(Message::Text(
            serde_json::to_string(&register).unwrap().into(),
        ))
        .await
        .unwrap();
    socket
}

fn payload(ciphertext: &[u8]) -> EncryptedRelayPayload {
    EncryptedRelayPayload {
        sender_public_key: "opaque-key".into(),
        nonce: "abcdefghijklmnop".into(),
        ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
    }
}

#[tokio::test]
async fn peer_messages_are_binary_exactly_between_kernels_that_offered_them() {
    for (home_offers, target_offers) in [(false, false), (false, true), (true, false), (true, true)]
    {
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
        let mut home = kernel(&url, "daemon-a", home_offers).await;
        let mut target = kernel(&url, "daemon-b", target_offers).await;
        sleep(Duration::from_millis(100)).await;

        // The request: small, binary only when the sender offered.
        let request_bytes = vec![0xab; 2048];
        let request = RelayEnvelope::DaemonPeerRequest {
            request_id: "peer-1".into(),
            target: ClientTarget {
                daemon_id: Some("daemon-b".into()),
                daemon_alias: None,
            },
            encrypted_request: payload(&request_bytes),
        };
        let frame = if home_offers {
            Message::Binary(
                binary_request::from_envelope(&request)
                    .unwrap()
                    .unwrap()
                    .into(),
            )
        } else {
            Message::Text(serde_json::to_string(&request).unwrap().into())
        };
        home.send(frame).await.unwrap();

        let (mut target_frames, mut home_frames) =
            (FrameReceiver::default(), FrameReceiver::default());
        let mut wire = 0;
        let incoming = match next_message(&mut target, &mut target_frames, &mut wire).await {
            Got::Binary(bytes) => {
                assert!(
                    target_offers,
                    "binary toward a kernel that did not offer it"
                );
                binary_request::into_envelope(&bytes).unwrap()
            }
            Got::Text(text) => {
                assert!(!target_offers, "JSON toward a kernel that offered binary");
                serde_json::from_str(&text).unwrap()
            }
        };
        let RelayEnvelope::DaemonIncomingPeerRequest {
            relay_request_id,
            from_daemon_id,
            encrypted_request,
            ..
        } = incoming
        else {
            panic!("an incoming peer request");
        };
        assert_eq!(from_daemon_id, "daemon-a");
        assert_eq!(encrypted_request, payload(&request_bytes));

        // The reply: three chunks of ciphertext, binary only when both ends offered.
        let reply_bytes: Vec<u8> = (0..3 * crate::frame_transport::CHUNK_BYTES)
            .map(|index| (index * 31) as u8)
            .collect();
        let reply = RelayEnvelope::DaemonIncomingPeerResponse {
            relay_request_id,
            encrypted_response: Some(payload(&reply_bytes)),
            error: None,
        };
        let frame = if target_offers {
            Message::Binary(
                binary_response::from_envelope(&reply)
                    .unwrap()
                    .unwrap()
                    .into(),
            )
        } else {
            Message::Text(serde_json::to_string(&reply).unwrap().into())
        };
        target.send(frame).await.unwrap();

        let mut reply_wire = 0;
        let routed = match next_message(&mut home, &mut home_frames, &mut reply_wire).await {
            Got::Binary(bytes) => {
                assert!(home_offers, "binary toward a kernel that did not offer it");
                binary_response::into_envelope(&bytes).unwrap()
            }
            Got::Text(text) => {
                assert!(!home_offers, "JSON toward a kernel that offered binary");
                serde_json::from_str(&text).unwrap()
            }
        };
        let RelayEnvelope::DaemonPeerResponse {
            request_id,
            from_daemon_id,
            encrypted_response: Some(routed),
            error: None,
        } = routed
        else {
            panic!("a routed peer reply");
        };
        assert_eq!(request_id, "peer-1");
        assert_eq!(from_daemon_id, "daemon-b");
        assert_eq!(routed, payload(&reply_bytes));
        if home_offers {
            assert!(
                reply_wire < reply_bytes.len() + reply_bytes.len() / 20,
                "{reply_wire} bytes on the wire for {} bytes of ciphertext",
                reply_bytes.len()
            );
        } else {
            assert!(reply_wire > reply_bytes.len() + reply_bytes.len() / 4);
        }
        let _ = stop.send(());
        task.await.unwrap();
    }
}
