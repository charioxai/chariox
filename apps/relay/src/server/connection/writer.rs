//! MP-08 / MP-10 / MP-11: bounded wire writes and independent control delivery.
use crate::frame_transport::{TransportFrame, CHUNK_BYTES, MAX_MESSAGE_BYTES, WINDOW_BYTES};
use futures_util::{Sink, SinkExt};
use std::collections::VecDeque;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

struct Transfer {
    id: u32,
    text: String,
    offset: usize,
    outstanding: VecDeque<(u32, usize, usize)>,
    wire_bytes: usize,
    small_sent: usize,
    small_acked: usize,
    request: bool,
}

impl Transfer {
    fn receipt(&mut self, id: u32, offset: u32) -> bool {
        if id != self.id || self.outstanding.front().map(|entry| entry.0) != Some(offset) {
            return false;
        }
        let (_, wire_bytes, small_acked) = self.outstanding.pop_front().unwrap();
        self.wire_bytes -= wire_bytes;
        // Ordered receipt proves delivery of small messages preceding this chunk.
        // Older chunks cannot release credit for small messages sent after them.
        self.small_acked = small_acked;
        true
    }

    fn next_chunk(&self) -> Option<(Message, usize)> {
        if self.offset == self.text.len() {
            return None;
        }
        let mut end = (self.offset + CHUNK_BYTES).min(self.text.len());
        loop {
            while !self.text.is_char_boundary(end) {
                end -= 1;
            }
            let text = serde_json::to_string(&TransportFrame::TransportChunk {
                transfer_id: self.id,
                offset: self.offset as u32,
                total_bytes: self.text.len() as u32,
                payload: self.text[self.offset..end].to_string(),
            })
            .ok()?;
            if text.len() <= CHUNK_BYTES {
                return (self.wire_bytes + text.len() <= WINDOW_BYTES)
                    .then_some((Message::Text(text.into()), end));
            }
            end = self.offset + (end - self.offset) / 2;
        }
    }
}

// Only correlation-addressed requests/responses may overtake a bulk envelope.
// Subscription events and lifecycle messages retain their original order.
fn independent_small(message: &Message) -> bool {
    if message.len() > CHUNK_BYTES {
        return false;
    }
    match message {
        Message::Ping(_) | Message::Pong(_) | Message::Close(_) => true,
        Message::Text(text) => serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|value| {
                value
                    .get("kind")
                    .and_then(|kind| kind.as_str())
                    .map(str::to_string)
            })
            .is_some_and(|kind| {
                matches!(
                    kind.as_str(),
                    "client_connected"
                        | "client_response"
                        | "client_metadata_response"
                        | "daemon_request"
                        | "daemon_peer_response"
                        | "daemon_incoming_peer_request"
                )
            }),
        _ => false,
    }
}

fn control(message: &Message) -> bool {
    matches!(
        message,
        Message::Ping(_) | Message::Pong(_) | Message::Close(_)
    ) || matches!(message, Message::Text(text) if text.len() <= CHUNK_BYTES && serde_json::from_str::<serde_json::Value>(text).ok()
        .is_some_and(|value| value.get("kind").and_then(|kind| kind.as_str()) == Some("close")))
}

fn is_request(message: &Message) -> bool {
    match message {
        Message::Text(text) => serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|value| {
                value
                    .get("kind")
                    .and_then(|kind| kind.as_str())
                    .map(str::to_string)
            })
            .is_some_and(|kind| kind.ends_with("request")),
        _ => false,
    }
}

async fn write<S: Sink<Message> + Unpin>(writer: &mut S, message: Message) -> bool {
    if writer.send(message).await.is_err() {
        // Tungstenite may have queued its automatic close/Pong while reading.
        let _ = writer.flush().await;
        false
    } else {
        true
    }
}

pub(super) async fn run_writer<S>(
    mut writer: S,
    mut outgoing: mpsc::Receiver<Message>,
    chunks: bool,
    mut receipts: mpsc::Receiver<(u32, u32)>,
    mut controls: mpsc::Receiver<Message>,
) where
    S: Sink<Message> + Unpin,
{
    let capacity = outgoing.max_capacity();
    let mut pending = VecDeque::new();
    let mut active: Option<Transfer> = None;
    let mut next_id = 0u32;
    let mut open = true;
    let mut receipts_open = true;
    let mut controls_open = true;
    loop {
        // Heartbeats cannot wait for space in the ordinary data queue.
        while let Ok(message) = controls.try_recv() {
            let terminal = matches!(message, Message::Close(_));
            if !write(&mut writer, message).await || terminal {
                return;
            }
        }
        while let Ok((id, offset)) = receipts.try_recv() {
            if !active
                .as_mut()
                .is_some_and(|transfer| transfer.receipt(id, offset))
            {
                return;
            }
        }
        if active
            .as_ref()
            .is_some_and(|t| t.offset == t.text.len() && t.outstanding.is_empty())
        {
            active = None;
        }
        while pending.len() < capacity {
            match outgoing.try_recv() {
                Ok(message) => pending.push_back(message),
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    open = false;
                    break;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
            }
        }
        let urgent = pending.iter().position(|message| {
            control(message)
                || active.as_ref().is_some_and(|transfer| {
                    transfer.small_sent - transfer.small_acked + message.len() <= CHUNK_BYTES
                        && independent_small(message)
                        && (!transfer.request || !is_request(message))
                })
        });
        if let Some(index) = urgent {
            let message = pending.remove(index).unwrap();
            let terminal = matches!(message, Message::Close(_));
            if let Some(transfer) = active.as_mut() {
                if matches!(message, Message::Text(_)) {
                    transfer.small_sent += message.len();
                }
            }
            if !write(&mut writer, message).await || terminal {
                return;
            }
            tokio::task::yield_now().await;
            continue;
        }
        if let Some(transfer) = active.as_mut() {
            if let Some((message, end)) = transfer.next_chunk() {
                let wire_bytes = message.len();
                // Record before writing so a fast receipt is never ahead of its chunk.
                transfer.offset = end;
                transfer
                    .outstanding
                    .push_back((end as u32, wire_bytes, transfer.small_sent));
                transfer.wire_bytes += wire_bytes;
                if !write(&mut writer, message).await {
                    return;
                }
                tokio::task::yield_now().await;
                continue;
            }
        } else if let Some(message) = pending.pop_front() {
            if let Message::Text(text) = &message {
                if text.len() > CHUNK_BYTES {
                    if !chunks || text.len() > MAX_MESSAGE_BYTES {
                        let _ = writer.send(Message::Text("{\"kind\":\"close\",\"reason\":\"relay bounded transport requires an updated client\"}".into())).await;
                        // MP-08 / MP-10 / MP-11: legacy clients project the final
                        // WebSocket close after the application close; retain its action.
                        let _ = writer
                            .send(Message::Close(Some(
                                tokio_tungstenite::tungstenite::protocol::CloseFrame {
                                    code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                                    reason: "relay bounded transport requires an updated client".into(),
                                },
                            )))
                            .await;
                        return;
                    }
                    let Some(id) = next_id.checked_add(1) else {
                        return;
                    };
                    next_id = id;
                    let request = serde_json::from_str::<serde_json::Value>(text)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("kind")
                                .and_then(|kind| kind.as_str())
                                .map(str::to_string)
                        })
                        .is_some_and(|kind| kind.ends_with("request"));
                    active = Some(Transfer {
                        id,
                        text: text.to_string(),
                        offset: 0,
                        outstanding: VecDeque::new(),
                        wire_bytes: 0,
                        small_sent: 0,
                        small_acked: 0,
                        request,
                    });
                    continue;
                }
            }
            if !write(&mut writer, message).await {
                return;
            }
            continue;
        } else if !open {
            return;
        }
        if !receipts_open && !open {
            return;
        }
        tokio::select! {
            message = controls.recv(), if controls_open => {
                if let Some(message) = message {
                    let terminal = matches!(message, Message::Close(_));
                    if !write(&mut writer, message).await || terminal { return; }
                } else { controls_open = false; }
            }
            receipt = receipts.recv(), if receipts_open => {
                let Some((id, offset)) = receipt else { receipts_open = false; continue; };
                if !active.as_mut().is_some_and(|transfer| transfer.receipt(id, offset)) { return; }
            }
            message = outgoing.recv(), if open && pending.len() < capacity => {
                match message { Some(message) => pending.push_back(message), None => open = false }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    // MP-08 / MP-10 / MP-11: clients project the final native close to the user.
    #[tokio::test]
    async fn legacy_large_response_retains_upgrade_reason_in_native_close() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            tokio_tungstenite::connect_async(format!("ws://{address}"))
                .await
                .unwrap()
                .0
        });
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut client = client.await.unwrap();
        let (tx, rx) = mpsc::channel(8);
        let (_ack_tx, ack_rx) = mpsc::channel(8);
        let (_control_tx, control_rx) = mpsc::channel(8);
        tx.send(Message::Text("x".repeat(CHUNK_BYTES + 1).into()))
            .await
            .unwrap();
        let task = tokio::spawn(run_writer(socket, rx, false, ack_rx, control_rx));
        let message = client.next().await.unwrap().unwrap();
        assert!(
            matches!(message, Message::Text(text) if text.contains("requires an updated client"))
        );
        let Message::Close(Some(close)) = client.next().await.unwrap().unwrap() else {
            panic!("native close must preserve the actionable upgrade reason");
        };
        assert_eq!(
            close.code,
            tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy
        );
        assert_eq!(
            close.reason,
            "relay bounded transport requires an updated client"
        );
        task.await.unwrap();
        let _ = client.close(None).await;
    }

    // MP-08 / MP-10 / MP-11: ACK of a later chunk must release drained small traffic
    // even while another bulk chunk remains outstanding.
    #[test]
    fn small_credit_follows_proven_wire_progress_not_transfer_completion() {
        let mut transfer = Transfer {
            id: 1,
            text: "x".repeat(17000),
            offset: 12,
            outstanding: VecDeque::from([
                (4, 1000, 0),
                (8, 1000, CHUNK_BYTES),
                (12, 1000, CHUNK_BYTES),
            ]),
            wire_bytes: 3000,
            small_sent: CHUNK_BYTES,
            small_acked: 0,
            request: false,
        };
        assert!(transfer.receipt(1, 4));
        assert_eq!(
            transfer.small_sent - transfer.small_acked,
            CHUNK_BYTES,
            "older data cannot acknowledge later small frames"
        );
        assert!(transfer.receipt(1, 8));
        assert_eq!(
            transfer.small_sent - transfer.small_acked,
            0,
            "small traffic preceding this chunk has drained; remaining bulk cannot hold its credit"
        );
        assert_eq!(transfer.outstanding.len(), 1);
    }

    // MP-08 / MP-10 / MP-11: both data and receipts traverse real ordered sockets.
    #[tokio::test]
    async fn small_requests_continue_during_partial_bulk_drain() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client_task = tokio::spawn(async move {
            tokio_tungstenite::connect_async(format!("ws://{address}"))
                .await
                .unwrap()
                .0
        });
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut client = client_task.await.unwrap();
        let (sink, mut reader) = socket.split();
        let (tx, rx) = mpsc::channel(8);
        let (ack_tx, ack_rx) = mpsc::channel(8);
        let (_control_tx, control_rx) = mpsc::channel(8);
        let acknowledgements = tokio::spawn(async move {
            while let Some(Ok(Message::Text(text))) = reader.next().await {
                let TransportFrame::TransportAck {
                    transfer_id,
                    offset,
                } = serde_json::from_str(&text).unwrap()
                else {
                    panic!("receipt expected")
                };
                if ack_tx.send((transfer_id, offset)).await.is_err() {
                    break;
                }
            }
        });
        let original = serde_json::json!({"kind":"client_response","request_id":"large","payload":"x".repeat(2_932_256)}).to_string();
        let expected = original.clone();
        let sender = tokio::spawn(async move {
            tx.send(Message::Text(original.into())).await.unwrap();
            for index in 0..64 {
                tx.send(Message::Text(serde_json::json!({"kind":"daemon_request","relay_request_id":format!("small-{index}"),"payload":"x".repeat(4096)}).to_string().into())).await.unwrap();
            }
        });
        let writer = tokio::spawn(run_writer(sink, rx, true, ack_rx, control_rx));
        let mut frames = crate::frame_transport::FrameReceiver::default();
        let mut complete = false;
        let mut small = 0;
        let mut small_before_complete = 0;
        let mut pending_receipt = None;
        while !complete || small != 64 {
            let message = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let Message::Text(text) = message else {
                continue;
            };
            assert!(text.len() <= CHUNK_BYTES);
            let (text, receipt) = frames.receive(&text).unwrap();
            if let Some(receipt) = receipt {
                // A natural one-chunk receipt delay keeps the bulk window nonempty.
                if let Some(previous) = pending_receipt.replace(receipt) {
                    client.send(Message::Text(previous.into())).await.unwrap();
                }
            }
            if let Some(text) = text {
                if text == expected {
                    complete = true;
                    if let Some(receipt) = pending_receipt.take() {
                        client.send(Message::Text(receipt.into())).await.unwrap();
                    }
                } else {
                    assert!(text.contains("small-"));
                    small += 1;
                    if !complete {
                        small_before_complete += 1;
                    }
                }
            }
        }
        assert!(small_before_complete >= 32, "small requests must continue beyond the initial16KiB credit while bulk is draining: {small_before_complete}");
        sender.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), writer)
            .await
            .unwrap()
            .unwrap();
        acknowledgements.abort();
        let _ = acknowledgements.await;
        let _ = client.close(None).await;
    }

    // MP-08 / MP-10 / MP-11: real ordered WebSocket bytes, not a message queue mock.
    #[tokio::test]
    async fn ordered_wire_large_response_is_bounded_and_control_overtakes_drain() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            tokio_tungstenite::connect_async(format!("ws://{address}"))
                .await
                .unwrap()
                .0
        });
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut client = client.await.unwrap();
        let (tx, rx) = mpsc::channel(8);
        let (_ack_tx, ack_rx) = mpsc::channel(8);
        let (control_tx, control_rx) = mpsc::channel(8);
        tx.send(Message::Text(serde_json::json!({"kind":"client_response", "request_id":"large", "payload":"x".repeat(2_932_256)}).to_string().into())).await.unwrap();
        let task = tokio::spawn(run_writer(socket, rx, true, ack_rx, control_rx));
        let first = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            first.len() < 20 * 1024,
            "a large response must not monopolize one ordered wire frame: {} bytes",
            first.len()
        );
        // Withhold receipts: a live socket can still process Ping and small responses.
        control_tx
            .send(Message::Ping(vec![7].into()))
            .await
            .unwrap();
        tx.send(Message::Text(
            "{\"kind\":\"client_response\",\"request_id\":\"small\"}".into(),
        ))
        .await
        .unwrap();
        tx.send(Message::Text(
            "{\"kind\":\"daemon_request\",\"relay_request_id\":\"small-request\"}".into(),
        ))
        .await
        .unwrap();
        // Ordered events wait for the snapshot, but cannot starve the control lane.
        for _ in 0..8 {
            let _ = tx.try_send(Message::Text(
                "{\"kind\":\"client_event\",\"payload\":\"ordered\"}".into(),
            ));
        }
        let mut ping = false;
        let mut small = false;
        let mut small_request = false;
        for _ in 0..12 {
            let message = tokio::time::timeout(std::time::Duration::from_secs(1), client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            match message {
                Message::Ping(_) => ping = true,
                Message::Text(text) if text.contains("\"small\"") => small = true,
                Message::Text(text) if text.contains("small-request") => small_request = true,
                Message::Text(text) => {
                    assert!(text.len() < 20 * 1024);
                    assert!(
                        !text.contains("\"ordered\""),
                        "events cannot precede the unfinished snapshot"
                    );
                }
                _ => {}
            }
            if ping && small && small_request {
                break;
            }
        }
        assert!(
            ping && small && small_request,
            "control and independent responses must pass a blocked bulk transfer"
        );
        task.abort();
        let _ = task.await;
        let _ = client.close(None).await;
    }
}
