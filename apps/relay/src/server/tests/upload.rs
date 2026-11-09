//! MP-08 / MP-09 / MP-10 / MP-11: a kernel that offers chunked uploads sends its
//! large messages in chunks-v2 frames; the relay reassembles them, acknowledges
//! each chunk, and lets a small message sent between chunks overtake the bulk.
//! A kernel without the offer (peer 440) keeps writing whole messages.
use super::*;
use crate::frame_transport::{
    read_message, text_chunk, FrameReceiver, TransportFrame, TRANSPORT_QUERY, UPLOAD_CHUNKS,
    UPLOAD_HEADER,
};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const BULK_PAYLOAD_BYTES: usize = 200_000;

async fn relay() -> (String, tokio::sync::oneshot::Sender<()>) {
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server.bind_listener().await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });
    (format!("ws://{addr}/?{TRANSPORT_QUERY}"), shutdown_tx)
}

/// A kernel socket, with or without the upload offer; returns whether the relay answered it.
async fn kernel(url: &str, offer: bool) -> (Socket, bool) {
    let mut request = url.into_client_request().unwrap();
    if offer {
        request
            .headers_mut()
            .insert(UPLOAD_HEADER, UPLOAD_CHUNKS.parse().unwrap());
    }
    let (mut socket, response) = connect_async(request).await.unwrap();
    let answered = response
        .headers()
        .get(UPLOAD_HEADER)
        .is_some_and(|value| value == UPLOAD_CHUNKS);
    socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration("daemon-a", "machine-1", "Linux", 10),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    sleep(Duration::from_millis(50)).await;
    (socket, answered)
}

async fn client(url: &str) -> (Socket, FrameReceiver) {
    let (mut client, _) = connect_async(url).await.unwrap();
    let mut frames = FrameReceiver::default();
    client
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-a".to_string()),
                    daemon_alias: None,
                },
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let connected = next_envelope(&mut client, &mut frames).await;
    assert!(matches!(connected, RelayEnvelope::ClientConnected { .. }));
    (client, frames)
}

async fn next_envelope(socket: &mut Socket, frames: &mut FrameReceiver) -> RelayEnvelope {
    loop {
        match timeout(Duration::from_secs(5), read_message(socket, frames))
            .await
            .expect("the relay answers")
        {
            Some(Ok(Message::Text(text))) => return serde_json::from_str(&text).unwrap(),
            Some(Ok(_)) => {}
            other => panic!("unexpected frame: {other:?}"),
        }
    }
}

fn request(id: &str) -> Message {
    Message::Text(
        serde_json::to_string(&RelayEnvelope::ClientRequest {
            request_id: id.to_string(),
            target: ClientTarget {
                daemon_id: Some("daemon-a".to_string()),
                daemon_alias: None,
            },
            encrypted_request: EncryptedRelayPayload {
                sender_public_key: "client-public".to_string(),
                nonce: "nonce".to_string(),
                ciphertext: "ciphertext".to_string(),
            },
        })
        .unwrap()
        .into(),
    )
}

/// What the relay sends a kernel: its own chunks are reassembled and
/// acknowledged, and receipts for the kernel's chunks are collected.
struct KernelReader {
    frames: FrameReceiver,
    receipts: Vec<(u32, usize)>,
}

impl KernelReader {
    async fn next_envelope(&mut self, kernel: &mut Socket) -> Option<RelayEnvelope> {
        loop {
            let Some(Ok(Message::Text(text))) =
                timeout(Duration::from_secs(5), kernel.next()).await.ok()?
            else {
                continue;
            };
            if let Ok(TransportFrame::TransportAck {
                transfer_id,
                offset,
            }) = serde_json::from_str(&text)
            {
                self.receipts.push((transfer_id, offset as usize));
                continue;
            }
            let (complete, receipt) = self.frames.receive(&text).unwrap();
            if let Some(receipt) = receipt {
                kernel.send(Message::Text(receipt.into())).await.unwrap();
            }
            if let Some(complete) = complete {
                return Some(serde_json::from_str(&complete).unwrap());
            }
        }
    }

    /// The relay request ids of the next two requests the kernel receives.
    async fn relay_request_ids(&mut self, kernel: &mut Socket) -> Vec<String> {
        let mut ids = Vec::new();
        while ids.len() < 2 {
            if let RelayEnvelope::DaemonRequest {
                relay_request_id, ..
            } = self
                .next_envelope(kernel)
                .await
                .expect("the kernel gets the request")
            {
                ids.push(relay_request_id);
            }
        }
        ids
    }
}

fn reply(relay_request_id: &str, bytes: usize) -> String {
    serde_json::to_string(&RelayEnvelope::DaemonResponse {
        relay_request_id: relay_request_id.to_string(),
        encrypted_response: Some(EncryptedRelayPayload {
            sender_public_key: "daemon-public".to_string(),
            nonce: "abcdefghijklmnop".to_string(),
            ciphertext: "c".repeat(bytes),
        }),
        error: None,
    })
    .unwrap()
}

fn response_bytes(envelope: &RelayEnvelope) -> (String, usize) {
    let RelayEnvelope::ClientResponse {
        request_id,
        encrypted_response: Some(payload),
        ..
    } = envelope
    else {
        panic!("a client response")
    };
    (request_id.clone(), payload.ciphertext.len())
}

#[tokio::test]
async fn a_small_reply_overtakes_a_chunked_kernel_upload_and_every_chunk_is_acknowledged() {
    let (url, _shutdown) = relay().await;
    let (mut kernel, answered) = kernel(&url, true).await;
    assert!(answered, "the relay takes chunked uploads");
    let (mut client, mut frames) = client(&url).await;
    client.send(request("bulk")).await.unwrap();
    client.send(request("small")).await.unwrap();
    let mut reader = KernelReader {
        frames: FrameReceiver::default(),
        receipts: Vec::new(),
    };
    let ids = reader.relay_request_ids(&mut kernel).await;

    // Half the bulk reply, then the small reply, then the rest.
    let bulk = reply(&ids[0], BULK_PAYLOAD_BYTES);
    let mut chunks = Vec::new();
    let mut offset = 0;
    while let Some((frame, end)) = text_chunk(1, &bulk, offset) {
        chunks.push(frame);
        offset = end;
    }
    let half = chunks.len() / 2;
    for chunk in &chunks[..half] {
        kernel
            .send(Message::Text(chunk.clone().into()))
            .await
            .unwrap();
    }
    kernel
        .send(Message::Text(reply(&ids[1], 10).into()))
        .await
        .unwrap();
    assert_eq!(
        response_bytes(&next_envelope(&mut client, &mut frames).await),
        ("small".to_string(), 10),
        "the small reply arrives while the bulk upload is still incomplete"
    );
    for chunk in &chunks[half..] {
        kernel
            .send(Message::Text(chunk.clone().into()))
            .await
            .unwrap();
    }
    assert_eq!(
        response_bytes(&next_envelope(&mut client, &mut frames).await),
        ("bulk".to_string(), BULK_PAYLOAD_BYTES),
        "the bulk reply arrives whole"
    );

    while reader.receipts.len() < chunks.len() {
        assert!(
            reader.next_envelope(&mut kernel).await.is_none(),
            "only receipts are left for the kernel"
        );
    }
    let mut expected = Vec::new();
    let mut offset = 0;
    while let Some((_, end)) = text_chunk(1, &bulk, offset) {
        expected.push((1, end));
        offset = end;
    }
    assert_eq!(
        reader.receipts, expected,
        "receipts follow the chunks in order"
    );
}

// Peer 440: no offer, no answer, and a whole large message is read as before.
#[tokio::test]
async fn a_kernel_without_the_offer_keeps_sending_whole_messages() {
    let (url, _shutdown) = relay().await;
    let (mut kernel, answered) = kernel(&url, false).await;
    assert!(!answered, "the relay answers only an offer");
    let (mut client, mut frames) = client(&url).await;
    client.send(request("bulk")).await.unwrap();
    client.send(request("small")).await.unwrap();
    let mut reader = KernelReader {
        frames: FrameReceiver::default(),
        receipts: Vec::new(),
    };
    let ids = reader.relay_request_ids(&mut kernel).await;
    kernel
        .send(Message::Text(reply(&ids[0], BULK_PAYLOAD_BYTES).into()))
        .await
        .unwrap();
    assert_eq!(
        response_bytes(&next_envelope(&mut client, &mut frames).await),
        ("bulk".to_string(), BULK_PAYLOAD_BYTES)
    );
}
