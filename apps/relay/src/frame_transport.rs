//! MP-08 / MP-10 / MP-11: bounded relay transport for every runtime client.
//! Opaque, socket-local framing. Receipts acknowledge transport delivery only;
//! they never authorize a request or acknowledge a kernel operation.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DISPLAY_EVENT_SCOPE: &str = "kernel_browser_display";
/// MP-08/MP-10/MP-11: allocated535/143, delivery registration invoking525 resume.
pub const DISPLAY_RESUME_SCOPE: &str = "kernel_browser_display_resume";
/// MP-08/MP-10/MP-11: 553/161, initial resume plus binary-v96 event transport.
pub const DISPLAY_RESUME_CONNECT_CAPABILITY: &str = "chariox-room-display-resume-v553";
pub fn is_display_scope(scope: Option<&str>) -> bool {
    matches!(scope, Some(DISPLAY_EVENT_SCOPE | DISPLAY_RESUME_SCOPE))
}
pub const TRANSPORT_QUERY: &str = "chariox_transport=chunks-v2";
pub const CHUNK_BYTES: usize = 16 * 1024;
/// Bulk bytes in flight per socket: a 480 KB mirror packet takes two round
/// trips instead of eight. 256 KiB per RTT is 23 Mbit/s at 90 ms.
pub const WINDOW_BYTES: usize = 256 * 1024;
/// Extra credit only single-chunk messages may use, so a small interactive
/// frame never waits a round trip behind a window full of bulk data.
pub const SMALL_RESERVE_BYTES: usize = 2 * CHUNK_BYTES;
/// Smallest chunk frame on the wire. The byte window plus reserve bound
/// unacknowledged frames at that many bytes / MIN_FRAME_BYTES, so a receipt
/// queue of that depth can never overflow for a conforming peer.
pub const MIN_FRAME_BYTES: usize = 64;
pub const MAX_UNACKED_FRAMES: usize = (WINDOW_BYTES + SMALL_RESERVE_BYTES) / MIN_FRAME_BYTES;
/// Unacknowledged frames a sender may keep toward any peer. Peers built with
/// the 64 KiB window size their receipt queue to this many entries and drop
/// the connection when it overflows, so the larger byte window must not raise it.
pub const PEER_RECEIPT_FRAMES: usize = (64 * 1024 + SMALL_RESERVE_BYTES) / MIN_FRAME_BYTES;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_TRANSFERS: usize = 4;
pub const MAX_ASSEMBLY_BYTES: usize = 128 * 1024 * 1024;

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
    let query = query
        .split('&')
        .filter(|part| !part.is_empty() && !part.starts_with("chariox_transport="))
        .collect::<Vec<_>>()
        .join("&");
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
    active: BTreeMap<u32, (usize, String)>,
    last_id: u32,
    reserved: usize,
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
        if transfer_id == 0 || total > MAX_MESSAGE_BYTES || payload.len() > CHUNK_BYTES {
            return Err("invalid transport chunk bounds");
        }
        // A zero-byte transfer is the receipt checkpoint immediately after one
        // bounded binary envelope. It never dispatches an application envelope.
        if total == 0 {
            if offset != 0 || !payload.is_empty() || transfer_id <= self.last_id {
                return Err("invalid transport checkpoint");
            }
            self.last_id = transfer_id;
            let ack = serde_json::to_string(&TransportFrame::TransportAck {
                transfer_id,
                offset: 0,
            })
            .map_err(|_| "cannot encode transport receipt")?;
            return Ok((None, Some(ack)));
        }
        if payload.is_empty() {
            return Err("invalid transport chunk bounds");
        }
        if !self.active.contains_key(&transfer_id) {
            if offset != 0
                || transfer_id <= self.last_id
                || self.active.len() >= MAX_TRANSFERS
                || total > MAX_ASSEMBLY_BYTES - self.reserved
            {
                return Err("invalid transport transfer start");
            }
            self.last_id = transfer_id;
            self.reserved += total;
            // Allocate only bytes received; declared totals reserve a fixed budget.
            self.active.insert(transfer_id, (total, String::new()));
        }
        let (expected_total, assembled) = self.active.get_mut(&transfer_id).unwrap();
        if *expected_total != total
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
            let (_, completed) = self.active.remove(&transfer_id).unwrap();
            self.reserved -= total;
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
    #[test]
    fn transport_urls_preserve_paths_and_normalize_empty_paths() {
        assert_eq!(
            transport_url("ws://127.0.0.1:1"),
            "ws://127.0.0.1:1/?chariox_transport=chunks-v2"
        );
        assert_eq!(
            transport_url("wss://relay.example/path?scope=public"),
            "wss://relay.example/path?scope=public&chariox_transport=chunks-v2"
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
    fn receipt_queue_holds_every_frame_the_window_admits() {
        let smallest = serde_json::to_string(&TransportFrame::TransportChunk {
            transfer_id: 1,
            offset: 0,
            total_bytes: 0,
            payload: String::new(),
        })
        .unwrap();
        assert!(smallest.len() >= MIN_FRAME_BYTES);
        assert!(MAX_UNACKED_FRAMES * smallest.len() >= WINDOW_BYTES + SMALL_RESERVE_BYTES);
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
        assert!(receiver.active.is_empty());
        let (_, receipt) = receiver.receive(&chunk(1, 0, 17000, "public")).unwrap();
        assert!(receipt.is_some());
        assert!(receiver.receive(&chunk(1, 0, 17000, "duplicate")).is_err());
        assert!(receiver.receive(&chunk(2, 6, 17000, "other")).is_err());
        assert!(receiver
            .receive(&chunk(1, 6, 18000, "changed total"))
            .is_err());
    }
}

#[cfg(test)]
mod multiplex_tests {
    use super::*;
    #[test]
    fn mp08_mp10_mp11_medium_reply_finishes_before_catalog() {
        let mut receiver = FrameReceiver::default();
        let catalog=serde_json::json!({"kind":"client_response","request_id":"catalog","payload":"x".repeat(40000)}).to_string();
        let reply=serde_json::json!({"kind":"client_response","request_id":"mirror","payload":"y".repeat(18000)}).to_string();
        let chunk = |id, offset, total, payload: &str| {
            serde_json::to_string(&TransportFrame::TransportChunk {
                transfer_id: id,
                offset,
                total_bytes: total,
                payload: payload.into(),
            })
            .unwrap()
        };
        receiver
            .receive(&chunk(1, 0, catalog.len() as u32, &catalog[..8000]))
            .unwrap();
        receiver
            .receive(&chunk(2, 0, reply.len() as u32, &reply[..8000]))
            .unwrap();
        assert_eq!(
            receiver
                .receive(&chunk(2, 8000, reply.len() as u32, &reply[8000..]))
                .unwrap()
                .0,
            Some(reply)
        );
        let mut complete = None;
        for offset in (8000..catalog.len()).step_by(8000) {
            complete = receiver
                .receive(&chunk(
                    1,
                    offset as u32,
                    catalog.len() as u32,
                    &catalog[offset..(offset + 8000).min(catalog.len())],
                ))
                .unwrap()
                .0;
        }
        assert_eq!(complete, Some(catalog));
    }
}
