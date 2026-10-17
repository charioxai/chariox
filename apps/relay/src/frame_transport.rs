//! MP-08 / MP-10 / MP-11: bounded relay transport for every runtime client.
//! Opaque, socket-local framing. Receipts acknowledge transport delivery only;
//! they never authorize a request or acknowledge a kernel operation.
use serde::{Deserialize, Serialize};

pub const TRANSPORT_QUERY: &str = "chariox_transport=chunks-v1";
pub const CHUNK_BYTES: usize = 16 * 1024;
pub const WINDOW_BYTES: usize = 64 * 1024;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransportFrame {
    TransportChunk {
        transfer_id: u32,
        offset: u32,
        total_bytes: u32,
        payload: String,
    },
    TransportAck {
        transfer_id: u32,
        offset: u32,
    },
}

pub fn transport_url(value: &str) -> String {
    let (base, query) = value.split_once('?').unwrap_or((value, ""));
    let has_path = base
        .split_once("://")
        .is_some_and(|(_, authority)| authority.contains('/'));
    format!(
        "{base}{}?{}{TRANSPORT_QUERY}",
        if has_path { "" } else { "/" },
        if query.is_empty() {
            String::new()
        } else {
            format!("{query}&")
        }
    )
}

/// No payload appears in errors or Debug output, including incomplete transfers.
#[derive(Default)]
pub struct FrameReceiver {
    active: Option<(u32, usize, String)>,
    last_id: u32,
}

impl FrameReceiver {
    /// Returns the complete original envelope, and a receipt to send immediately.
    /// An ordinary envelope may interleave without disturbing the active transfer.
    pub fn receive(
        &mut self,
        text: &str,
    ) -> Result<(Option<String>, Option<String>), &'static str> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|_| "invalid relay JSON")?;
        match value.get("kind").and_then(|kind| kind.as_str()) {
            Some("transport_chunk") => {}
            Some("transport_ack") => return Err("unexpected transport receipt"),
            _ => return Ok((Some(text.to_string()), None)),
        }
        let TransportFrame::TransportChunk {
            transfer_id,
            offset,
            total_bytes,
            payload,
        } = serde_json::from_value(value).map_err(|_| "invalid transport chunk")?
        else {
            unreachable!()
        };
        let total = total_bytes as usize;
        if transfer_id == 0
            || total <= CHUNK_BYTES
            || total > MAX_MESSAGE_BYTES
            || payload.is_empty()
            || payload.len() > CHUNK_BYTES
        {
            return Err("invalid transport chunk bounds");
        }
        if self.active.is_none() {
            if offset != 0 || transfer_id <= self.last_id {
                return Err("invalid transport transfer start");
            }
            // Allocate only bytes actually received, not an untrusted total.
            self.active = Some((transfer_id, total, String::new()));
        }
        let (id, expected_total, assembled) = self.active.as_mut().unwrap();
        if *id != transfer_id
            || *expected_total != total
            || assembled.len() != offset as usize
            || payload.len() > total - assembled.len()
        {
            return Err("non-contiguous transport chunk");
        }
        assembled.push_str(&payload);
        let ack = serde_json::to_string(&TransportFrame::TransportAck {
            transfer_id,
            offset: assembled.len() as u32,
        })
        .map_err(|_| "cannot encode transport receipt")?;
        if assembled.len() == total {
            self.last_id = transfer_id;
            let (_, _, completed) = self.active.take().unwrap();
            // Reassembly cannot create a nested transport frame.
            let value: serde_json::Value =
                serde_json::from_str(&completed).map_err(|_| "invalid assembled relay JSON")?;
            if value
                .get("kind")
                .and_then(|v| v.as_str())
                .is_some_and(|kind| kind.starts_with("transport_"))
            {
                return Err("nested transport frame");
            }
            Ok((Some(completed), Some(ack)))
        } else {
            Ok((None, Some(ack)))
        }
    }
}

// MP-08 / MP-10 / MP-11: keep kernel-to-relay frames below the same wire bound.
// RFC 6455 continuation frames preserve the exact envelope; they add no JSON
// protocol or client-visible state and are reassembled by the WebSocket peer.
pub async fn send_text<S>(writer: &mut S, text: String) -> bool
where
    S: futures_util::Sink<tokio_tungstenite::tungstenite::Message> + Unpin,
{
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::{
        protocol::frame::{
            coding::{Data as OpData, OpCode},
            Frame,
        },
        Message,
    };
    if text.len() > MAX_MESSAGE_BYTES {
        return false;
    }
    if text.len() <= CHUNK_BYTES {
        return writer.send(Message::Text(text.into())).await.is_ok();
    }
    for (index, chunk) in text.as_bytes().chunks(CHUNK_BYTES).enumerate() {
        let opcode = OpCode::Data(if index == 0 {
            OpData::Text
        } else {
            OpData::Continue
        });
        let final_frame = (index + 1) * CHUNK_BYTES >= text.len();
        if writer
            .send(Message::Frame(Frame::message(
                chunk.to_vec(),
                opcode,
                final_frame,
            )))
            .await
            .is_err()
        {
            return false;
        }
        tokio::task::yield_now().await;
    }
    true
}

/// Temporary peer/metadata connections share exactly the same reassembly seam.
pub async fn read_message<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    receiver: &mut FrameReceiver,
) -> Option<Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Error, Message};
    loop {
        match socket.next().await? {
            Ok(Message::Text(text)) => match receiver.receive(&text) {
                Ok((text, receipt)) => {
                    if let Some(receipt) = receipt {
                        if let Err(error) = socket.send(Message::Text(receipt.into())).await {
                            return Some(Err(error));
                        }
                    }
                    if let Some(text) = text {
                        return Some(Ok(Message::Text(text.into())));
                    }
                }
                Err(reason) => return Some(Err(Error::Io(std::io::Error::other(reason)))),
            },
            Ok(Message::Ping(payload)) => {
                if let Err(error) = socket.send(Message::Pong(payload)).await {
                    return Some(Err(error));
                }
            }
            Ok(Message::Pong(_)) => {}
            message => return Some(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MP-08 / MP-10 / MP-11: transport regression only; real hosted catalog
    // failure/pass is recorded separately by the native-provider drill.
    #[tokio::test]
    async fn mp08_large_kernel_envelopes_use_bounded_websocket_frames() {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let expected = format!(
            r#"{{"kind":"daemon_peer_response","ciphertext":"{}"}}"#,
            "x".repeat(19 * 1024 * 1024)
        );
        let digest = expected.clone();
        let received = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let config = WebSocketConfig::default()
                .max_frame_size(Some(CHUNK_BYTES))
                .max_message_size(Some(MAX_MESSAGE_BYTES));
            let mut socket = tokio_tungstenite::accept_async_with_config(socket, Some(config))
                .await
                .unwrap();
            let Message::Text(text) = socket
                .next()
                .await
                .unwrap()
                .expect("every wire frame is bounded")
            else {
                panic!("expected unchanged text envelope")
            };
            assert_eq!(text.as_str(), digest);
        });
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        assert!(send_text(&mut socket, expected).await);
        received.await.unwrap();
    }

    #[tokio::test]
    async fn mp08_fragmented_unicode_envelopes_preserve_utf8_and_small_messages() {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let expected = format!(
            r#"{{"kind":"daemon_peer_response","public":"{}"}}"#,
            "𝄞é".repeat(CHUNK_BYTES)
        );
        let digest = expected.clone();
        let received = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let config = WebSocketConfig::default()
                .max_frame_size(Some(CHUNK_BYTES))
                .max_message_size(Some(MAX_MESSAGE_BYTES));
            let mut socket = tokio_tungstenite::accept_async_with_config(socket, Some(config))
                .await
                .unwrap();
            let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                panic!("expected text")
            };
            assert_eq!(text.as_str(), digest);
            let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                panic!("expected small message")
            };
            assert_eq!(text.as_str(), r#"{"kind":"daemon_heartbeat"}"#);
        });
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        assert!(send_text(&mut socket, expected).await);
        assert!(send_text(&mut socket, r#"{"kind":"daemon_heartbeat"}"#.into()).await);
        received.await.unwrap();
    }

    #[test]
    fn transport_urls_preserve_paths_and_normalize_empty_paths() {
        assert_eq!(
            transport_url("ws://127.0.0.1:1"),
            "ws://127.0.0.1:1/?chariox_transport=chunks-v1"
        );
        assert_eq!(
            transport_url("wss://relay.example/path?scope=public"),
            "wss://relay.example/path?scope=public&chariox_transport=chunks-v1"
        );
    }
    #[test]
    fn transport_snapshot_is_stable() {
        use sha2::{Digest, Sha256};
        let chunk = TransportFrame::TransportChunk {
            transfer_id: 1,
            offset: 0,
            total_bytes: 17000,
            payload: "public".to_string(),
        };
        let text = serde_json::to_string(&chunk).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(text.as_bytes())),
            "63f2c15eb230dffa4358b314b5c798c07ac7d6c726781fd09314f6e2810b9e05"
        );
    }
    #[test]
    fn malformed_chunks_do_not_allocate_declared_totals_or_acknowledge_overlap() {
        let mut receiver = FrameReceiver::default();
        let chunk = |id, offset, total, text: &str| {
            serde_json::to_string(&TransportFrame::TransportChunk {
                transfer_id: id,
                offset,
                total_bytes: total,
                payload: text.to_string(),
            })
            .unwrap()
        };
        assert!(receiver
            .receive(&chunk(1, 0, MAX_MESSAGE_BYTES as u32 + 1, "private-canary"))
            .is_err());
        assert!(receiver.active.is_none());
        let (_, receipt) = receiver.receive(&chunk(1, 0, 17000, "public")).unwrap();
        assert!(receipt.is_some());
        assert!(receiver.receive(&chunk(1, 0, 17000, "duplicate")).is_err());
        assert!(receiver.receive(&chunk(2, 6, 17000, "other")).is_err());
        assert!(receiver
            .receive(&chunk(1, 6, 18000, "changed total"))
            .is_err());
    }
}
