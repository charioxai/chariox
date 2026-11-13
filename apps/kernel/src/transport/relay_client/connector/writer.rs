use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use chariox_relay::protocol::RelayEnvelope;
use futures_util::{Sink, SinkExt};
use tokio_tungstenite::tungstenite::Message;

use super::super::reply_notes::ReplyNotes;
use crate::transport::relay_upload::{Lane as UploadLane, Uploads};

const RELAY_LARGE_FRAME_LOG_BYTES: usize = 256 * 1024;
const RELAY_SLOW_WRITE_LOG_MS: u128 = 500;
const RELAY_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

async fn send_relay_frame<S>(writer: &mut S, frame: Message) -> bool
where
    S: Sink<Message> + Unpin,
{
    // MP-08 / MP-09 / MP-10 / MP-11: a queued renewal cannot heal a stuck
    // writer. Ending it wakes the connector and closes the old socket.
    match tokio::time::timeout(RELAY_WRITE_TIMEOUT, writer.send(frame)).await {
        Ok(result) => result.is_ok(),
        Err(_) => {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "relay socket write timed out; reconnecting",
                serde_json::json!({ "timeout_ms": RELAY_WRITE_TIMEOUT.as_millis() }),
            );
            false
        }
    }
}

/// With `uploads`, a frame above one chunk is handed to its lane to go out in
/// chunks instead of being written whole.
pub(super) async fn send_relay_envelope_frame<S>(
    writer: &mut S,
    envelope: RelayEnvelope,
    lane: &'static str,
    binary_events: bool,
    closing: &AtomicBool,
    notes: &ReplyNotes,
    uploads: Option<(&mut Uploads, UploadLane)>,
) -> bool
where
    S: Sink<Message> + Unpin,
{
    // MP-08/MP-10/MP-11: skip coalesced and queued stale stream events while
    // preserving every bulk reply. Keep no sender inside its own writer task.
    if !super::super::relay_envelope_should_write(&envelope, closing.load(Ordering::Acquire)) {
        return true;
    }
    let kind = relay_envelope_kind(&envelope);
    let frame = if binary_events {
        match chariox_relay::binary_response::from_envelope(&envelope).and_then(|response| {
            match response {
                Some(bytes) => Ok(Some(bytes)),
                None => chariox_relay::binary_event::from_envelope(&envelope),
            }
        }) {
            Ok(Some(bytes)) => Message::Binary(bytes.into()),
            Ok(None) => match serde_json::to_string(&envelope) {
                Ok(text) => Message::Text(text.into()),
                Err(_) => return false,
            },
            Err(_) => match serde_json::to_string(&envelope) {
                Ok(text) => Message::Text(text.into()),
                Err(_) => return false,
            },
        }
    } else {
        match serde_json::to_string(&envelope) {
            Ok(text) => Message::Text(text.into()),
            Err(_) => return false,
        }
    };
    let payload_len = frame.len();
    let reply = notes.take(&envelope);
    let packet = chariox_relay::transport_timing::envelope_signature(&envelope);
    chariox_relay::transport_timing::record("kernel_write_start", packet);
    let started = Instant::now();
    let frame = match uploads {
        Some((uploads, lane)) => uploads.start(lane, frame),
        None => Some(frame),
    };
    let sent = match frame {
        Some(frame) => send_relay_frame(writer, frame).await,
        None => true,
    };
    chariox_relay::transport_timing::record("kernel_write_end", packet);
    if kind == "daemon_event" {
        crate::transport::kernel_browser_display::timing("event_socket_write", started);
    }
    let elapsed_ms = started.elapsed().as_millis();
    notes.record_written(lane, reply.as_ref(), payload_len);
    if payload_len >= RELAY_LARGE_FRAME_LOG_BYTES || elapsed_ms >= RELAY_SLOW_WRITE_LOG_MS {
        crate::logging::warn_with_fields(
            "daemon.relay_client",
            "relay envelope write exceeded diagnostic threshold",
            serde_json::json!({
                "lane": lane,
                "kind": kind,
                "payload_bytes": payload_len,
                "write_ms": elapsed_ms,
                "sent": sent,
                "request_kind": reply.as_ref().map(|reply| reply.origin.request_kind),
                "client": reply.as_ref().map(|reply| reply.origin.client.as_str()),
            }),
        );
    }
    sent
}

fn relay_envelope_kind(envelope: &RelayEnvelope) -> &'static str {
    match envelope {
        RelayEnvelope::DaemonRegister { .. } => "daemon_register",
        RelayEnvelope::DaemonHeartbeat { .. } => "daemon_heartbeat",
        RelayEnvelope::DaemonResponse { .. } => "daemon_response",
        RelayEnvelope::DaemonEvent { .. } => "daemon_event",
        RelayEnvelope::DaemonPeerRequest { .. } => "daemon_peer_request",
        RelayEnvelope::DaemonPeerResponse { .. } => "daemon_peer_response",
        RelayEnvelope::DaemonPeerEvent { .. } => "daemon_peer_event",
        RelayEnvelope::DaemonDisplayTunnelResponseStart { .. } => {
            "daemon_display_tunnel_response_start"
        }
        RelayEnvelope::DaemonDisplayTunnelChunk { .. } => "daemon_display_tunnel_chunk",
        RelayEnvelope::DaemonDisplayTunnelClientChunk { .. } => {
            "daemon_display_tunnel_client_chunk"
        }
        RelayEnvelope::Close { .. } => "close",
        _ => "other",
    }
}

// MP-08 / MP-10 / MP-11: socket writer stays below the connector coordinator.
// Strict priority at every frame boundary: owed receipts and pongs, then the
// priority lane, then the event lane. Toward a relay that takes chunked
// uploads (`upload_receipt_rx` is Some), a lane sends a large message one
// chunk at a time while the window allows and takes no other message until
// it is done, so each lane stays FIFO and the priority lane passes event bulk
// between any two chunks.
#[allow(
    clippy::too_many_arguments,
    reason = "each lane, the closing flag and the reply notes are separate inputs of the one writer task"
)]
pub(super) async fn run_relay_writer<S>(
    mut writer: S,
    mut priority_outgoing_rx: tokio::sync::mpsc::Receiver<RelayEnvelope>,
    mut event_outgoing_rx: tokio::sync::mpsc::Receiver<RelayEnvelope>,
    mut pong_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    mut transport_receipt_rx: tokio::sync::mpsc::Receiver<String>,
    mut upload_receipt_rx: Option<tokio::sync::mpsc::Receiver<(u32, u32)>>,
    binary_events: bool,
    binary_requests: bool,
    closing: std::sync::Arc<AtomicBool>,
    notes: std::sync::Arc<ReplyNotes>,
) where
    S: Sink<Message> + Unpin,
{
    let chunked = upload_receipt_rx.is_some();
    let mut uploads = Uploads::new(binary_requests);
    let mut priority_open = true;
    let mut event_open = true;
    while priority_open
        || event_open
        || uploads.busy(UploadLane::Priority)
        || uploads.busy(UploadLane::Event)
    {
        let priority_chunk = uploads.ready(UploadLane::Priority);
        let event_chunk = uploads.ready(UploadLane::Event);
        let take_priority = priority_open && !uploads.busy(UploadLane::Priority);
        let take_event = event_open && !uploads.busy(UploadLane::Event);
        tokio::select! {
            biased;
            Some(receipt) = transport_receipt_rx.recv() => {
                if !send_relay_frame(&mut writer, Message::Text(receipt.into())).await { break; }
            }
            Some(payload) = pong_rx.recv() => {
                if !send_relay_frame(&mut writer, Message::Pong(payload.into())).await { break; }
            }
            receipt = async { upload_receipt_rx.as_mut()?.recv().await }, if chunked => {
                let Some((id, offset)) = receipt else { break; };
                if !uploads.receipt(id, offset) {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "relay acknowledged an upload chunk out of order; reconnecting",
                        serde_json::json!({ "transfer_id": id, "offset": offset }),
                    );
                    break;
                }
            }
            _ = std::future::ready(()), if priority_chunk => {
                let Some(chunk) = uploads.next_chunk(UploadLane::Priority) else { break; };
                if !send_relay_frame(&mut writer, chunk).await { break; }
            }
            envelope = priority_outgoing_rx.recv(), if take_priority => {
                match envelope {
                    Some(envelope) => if !send_relay_envelope_frame(&mut writer, envelope, "priority", binary_events, &closing, &notes, chunked.then_some((&mut uploads, UploadLane::Priority))).await { break; },
                    None => priority_open = false,
                }
            }
            _ = std::future::ready(()), if event_chunk => {
                let Some(chunk) = uploads.next_chunk(UploadLane::Event) else { break; };
                if !send_relay_frame(&mut writer, chunk).await { break; }
            }
            envelope = event_outgoing_rx.recv(), if take_event => {
                match envelope {
                    Some(envelope) => if !send_relay_envelope_frame(&mut writer, envelope, "event", binary_events, &closing, &notes, chunked.then_some((&mut uploads, UploadLane::Event))).await { break; },
                    None => event_open = false,
                }
            }
            else => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use std::time::Duration;
    use tokio::sync::mpsc;

    struct StalledSink;
    impl Sink<Message> for StalledSink {
        type Error = std::io::Error;
        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_writes_end_the_writer_so_the_connector_can_reconnect() {
        // MP-08 / MP-09 / MP-10 / MP-11: token renewal queued behind a stuck
        // socket must not leave the connector believing it is still online.
        for lane in ["priority", "event", "receipt", "pong"] {
            let (priority, priority_rx) = mpsc::channel(2);
            let (event, event_rx) = mpsc::channel(2);
            let (pong, pong_rx) = mpsc::channel(2);
            let (receipt, receipt_rx) = mpsc::channel(2);
            match lane {
                "priority" => priority
                    .send(RelayEnvelope::DaemonHeartbeat {
                        daemon_id: "kernel".into(),
                        registration: None,
                    })
                    .await
                    .unwrap(),
                "event" => event
                    .send(RelayEnvelope::Close {
                        reason: "test".into(),
                        kernel_reload: false,
                    })
                    .await
                    .unwrap(),
                "receipt" => receipt.send("receipt".into()).await.unwrap(),
                _ => pong.send(vec![]).await.unwrap(),
            }
            let writer = run_relay_writer(
                StalledSink,
                priority_rx,
                event_rx,
                pong_rx,
                receipt_rx,
                None,
                false,
                false,
                Arc::new(AtomicBool::new(false)),
                Arc::default(),
            );
            assert!(
                tokio::time::timeout(Duration::from_secs(31), writer)
                    .await
                    .is_ok(),
                "stalled {lane} write never releases connector"
            );
        }
    }

    fn bulk_reply() -> RelayEnvelope {
        RelayEnvelope::DaemonResponse {
            relay_request_id: "file-chunk".into(),
            encrypted_response: Some(chariox_relay::protocol::EncryptedRelayPayload {
                sender_public_key: "daemon".into(),
                nonce: "nonce".into(),
                ciphertext: "x".repeat(300 * 1024),
            }),
            error: None,
        }
    }

    fn heartbeat() -> RelayEnvelope {
        RelayEnvelope::DaemonHeartbeat {
            daemon_id: "kernel".into(),
            registration: None,
        }
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relaychunk): toward a relay that takes chunked uploads,
    // a 300 KB reply goes out in chunks and a heartbeat queued behind it is written at the
    // next chunk boundary, not after the whole reply (4.6 s at 76 KB/s).
    #[tokio::test]
    async fn a_heartbeat_passes_a_chunked_reply_at_the_next_chunk() {
        let (wire_tx, mut wire) = mpsc::channel::<Message>(1);
        let sink = Box::pin(futures_util::sink::unfold(
            wire_tx,
            |tx, message: Message| async move {
                tx.send(message).await.unwrap();
                Ok::<_, std::convert::Infallible>(tx)
            },
        ));
        let (priority, priority_rx) = mpsc::channel(4);
        let (event, event_rx) = mpsc::channel(4);
        let (_pong, pong_rx) = mpsc::channel(1);
        let (_receipt, receipt_rx) = mpsc::channel(1);
        let (upload_receipt, upload_receipt_rx) = mpsc::channel(64);
        event.send(bulk_reply()).await.unwrap();
        tokio::spawn(run_relay_writer(
            sink,
            priority_rx,
            event_rx,
            pong_rx,
            receipt_rx,
            Some(upload_receipt_rx),
            false,
            false,
            Arc::new(AtomicBool::new(false)),
            Arc::default(),
        ));
        let mut relay = chariox_relay::frame_transport::FrameReceiver::default();
        let mut acknowledge = async |message: Message| {
            let Message::Text(text) = message else {
                panic!("text frames")
            };
            assert!(text.len() <= chariox_relay::frame_transport::CHUNK_BYTES);
            let (complete, receipt) = relay.receive(&text).unwrap();
            if let Some(receipt) = receipt {
                let chariox_relay::frame_transport::TransportFrame::TransportAck {
                    transfer_id,
                    offset,
                } = serde_json::from_str(&receipt).unwrap()
                else {
                    panic!("receipt expected")
                };
                upload_receipt.send((transfer_id, offset)).await.unwrap();
            }
            complete
        };
        // A cold window takes short chunks until it is full, then the writer waits.
        let cold = chariox_relay::drain_window::DrainWindow::new(tokio::time::Instant::now());
        let mut in_flight = Vec::new();
        let mut in_flight_bytes = 0;
        while in_flight.is_empty() || in_flight_bytes + cold.chunk_bytes() <= cold.bytes() {
            let chunk = wire.recv().await.unwrap();
            in_flight_bytes += chunk.len();
            in_flight.push(chunk);
        }
        priority.send(heartbeat()).await.unwrap();
        for chunk in in_flight {
            assert!(acknowledge(chunk).await.is_none());
        }
        let next = wire.recv().await.unwrap();
        assert!(
            matches!(&next, Message::Text(text) if text.contains("daemon_heartbeat")),
            "the heartbeat goes before the third chunk"
        );
        assert!(acknowledge(next).await.is_some());
        let reply = loop {
            if let Some(complete) = acknowledge(wire.recv().await.unwrap()).await {
                break complete;
            }
        };
        assert_eq!(reply, serde_json::to_string(&bulk_reply()).unwrap());
    }

    fn peer_request(id: &str, ciphertext_bytes: usize) -> RelayEnvelope {
        RelayEnvelope::DaemonPeerRequest {
            request_id: id.into(),
            target: chariox_relay::protocol::ClientTarget {
                daemon_id: Some("serving-kernel".into()),
                daemon_alias: None,
            },
            encrypted_request: chariox_relay::protocol::EncryptedRelayPayload {
                sender_public_key: "daemon".into(),
                nonce: "nonce".into(),
                ciphertext: "x".repeat(ciphertext_bytes),
            },
        }
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relayproof r3): `files put` sends each 64 KiB file
    // chunk as a peer request of about 117 KB. A small peer request (a session list, a
    // kernel renewal) queued behind the window of upload chunks waited until every one
    // of them was on the wire (0.4 s at 8 Mbit/s, 3.8 s at 1 Mbit/s); it now goes out
    // before the next chunk.
    #[tokio::test]
    async fn a_small_peer_request_passes_queued_upload_chunks() {
        let (wire_tx, mut wire) = mpsc::unbounded_channel::<Message>();
        let sink = Box::pin(futures_util::sink::unfold(
            wire_tx,
            |tx, message: Message| async move {
                tx.send(message).unwrap();
                Ok::<_, std::convert::Infallible>(tx)
            },
        ));
        let (outgoing, priority_rx, event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(16);
        for n in 0..4 {
            outgoing
                .try_send(peer_request(&format!("upload-{n}"), 117 * 1024))
                .unwrap();
        }
        outgoing.try_send(peer_request("list", 600)).unwrap();
        let (_pong, pong_rx) = mpsc::channel(1);
        let (_receipt, receipt_rx) = mpsc::channel(1);
        let (upload_receipt, upload_receipt_rx) = mpsc::channel(64);
        tokio::spawn(run_relay_writer(
            sink,
            priority_rx,
            event_rx,
            pong_rx,
            receipt_rx,
            Some(upload_receipt_rx),
            false,
            false,
            Arc::new(AtomicBool::new(false)),
            Arc::default(),
        ));
        let mut relay = chariox_relay::frame_transport::FrameReceiver::default();
        let mut bulk_bytes_ahead = 0;
        loop {
            let Message::Text(text) = wire.recv().await.unwrap() else {
                panic!("text frames")
            };
            if text.contains("\"request_id\":\"list\"") {
                break;
            }
            bulk_bytes_ahead += text.len();
            let (_, receipt) = relay.receive(&text).unwrap();
            let chariox_relay::frame_transport::TransportFrame::TransportAck {
                transfer_id,
                offset,
            } = serde_json::from_str(&receipt.unwrap()).unwrap()
            else {
                panic!("receipt expected")
            };
            upload_receipt.send((transfer_id, offset)).await.unwrap();
        }
        assert!(
            bulk_bytes_ahead <= chariox_relay::frame_transport::CHUNK_BYTES,
            "{bulk_bytes_ahead} bytes of upload chunks went out ahead of the small request"
        );
    }

    // Toward a relay that did not answer the upload offer (peer 440), the reply is one frame.
    #[tokio::test]
    async fn without_chunked_uploads_a_reply_is_one_frame() {
        let (wire_tx, mut wire) = mpsc::unbounded_channel::<Message>();
        let sink = Box::pin(futures_util::sink::unfold(
            wire_tx,
            |tx, message: Message| async move {
                tx.send(message).unwrap();
                Ok::<_, std::convert::Infallible>(tx)
            },
        ));
        let (_priority, priority_rx) = mpsc::channel(4);
        let (event, event_rx) = mpsc::channel(4);
        let (_pong, pong_rx) = mpsc::channel(1);
        let (_receipt, receipt_rx) = mpsc::channel(1);
        event.send(bulk_reply()).await.unwrap();
        tokio::spawn(run_relay_writer(
            sink,
            priority_rx,
            event_rx,
            pong_rx,
            receipt_rx,
            None,
            false,
            false,
            Arc::new(AtomicBool::new(false)),
            Arc::default(),
        ));
        let Some(Message::Text(text)) = wire.recv().await else {
            panic!("a text frame")
        };
        assert_eq!(text.as_str(), serde_json::to_string(&bulk_reply()).unwrap());
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relayflood): the 11-09 flood had to be traced to its
    // requester from frame sizes; the diagnostic now names the request kind and the client.
    #[tokio::test]
    async fn large_reply_log_names_its_request() {
        let logs = crate::logging::capture::start();
        let notes = ReplyNotes::default();
        let reply = RelayEnvelope::DaemonResponse {
            relay_request_id: "chunk-1".into(),
            encrypted_response: Some(chariox_relay::protocol::EncryptedRelayPayload {
                sender_public_key: "daemon".into(),
                nonce: "nonce".into(),
                ciphertext: "x".repeat(300 * 1024),
            }),
            error: None,
        };
        let origin = super::super::super::reply_notes::ReplyOrigin::new(
            "machine_file.read",
            "key-of-the-file-reader",
        );
        let client = origin.client.clone();
        notes.note(&reply, origin, true).await;
        let mut sink = futures_util::sink::drain::<Message>();
        assert!(
            send_relay_envelope_frame(
                &mut sink,
                reply,
                "event",
                false,
                &AtomicBool::new(false),
                &notes,
                None
            )
            .await
        );
        let record = logs
            .records()
            .into_iter()
            .find(|record| record.contains("relay envelope write exceeded diagnostic threshold"))
            .expect("a 300 KB reply is logged");
        assert!(
            record.contains("\"request_kind\":\"machine_file.read\""),
            "{record}"
        );
        assert!(
            record.contains(&format!("\"client\":\"{client}\"")),
            "{record}"
        );
        assert_eq!(client.len(), 8);
    }
}
