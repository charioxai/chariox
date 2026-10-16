//! MP-08 / MP-10 / MP-11: bounded opaque multiplexing (532 / peer 140).
use crate::frame_transport::{
    TransportFrame, CHUNK_BYTES, MAX_ASSEMBLY_BYTES, MAX_MESSAGE_BYTES, MAX_TRANSFERS, WINDOW_BYTES,
};
use futures_util::{Sink, SinkExt};
use std::collections::VecDeque;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    Reply,
    Request,
    Event,
    Ordered,
}
fn lane(message: &Message, display_route: &impl Fn(&str) -> bool) -> Lane {
    if let Message::Binary(bytes) = message {
        return if crate::binary_event::decode(bytes)
            .ok()
            .is_some_and(|(header, _)| {
                header.kind == "client_event" && display_route(&header.subscription_id)
            }) {
            Lane::Event
        } else {
            Lane::Ordered
        };
    }
    let Message::Text(text) = message else {
        return Lane::Ordered;
    };
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Lane::Ordered,
    };
    match value.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
        "client_response"
        | "client_metadata_response"
        | "daemon_peer_response"
        | "client_connected" => Lane::Reply,
        "client_event"
            if value
                .get("subscription_id")
                .and_then(|v| v.as_str())
                .is_some_and(display_route) =>
        {
            Lane::Event
        }
        kind if kind.ends_with("request") => Lane::Request,
        _ => Lane::Ordered,
    }
}
fn control(message: &Message) -> bool {
    matches!(
        message,
        Message::Ping(_) | Message::Pong(_) | Message::Close(_)
    ) || matches!(message, Message::Text(text) if text.len() <= CHUNK_BYTES && serde_json::from_str::<serde_json::Value>(text).ok()
            .is_some_and(|v| v.get("kind").and_then(|v|v.as_str()) == Some("close")))
}
struct Transfer {
    id: u32,
    text: String,
    binary: Option<Message>,
    offset: usize,
    lane: Lane,
    waiting: usize,
}
impl Transfer {
    fn next_chunk(&self) -> Option<(Message, usize)> {
        if self.binary.is_some() {
            let checkpoint = serde_json::to_string(&TransportFrame::TransportChunk {
                transfer_id: self.id,
                offset: 0,
                total_bytes: 0,
                payload: String::new(),
            })
            .ok()?;
            return Some((Message::Text(checkpoint.into()), 0));
        }
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
                return Some((Message::Text(text.into()), end));
            }
            end = self.offset + (end - self.offset) / 2;
        }
    }
    fn finished(&self) -> bool {
        // MP-08/MP-10/MP-11: assembly/lane occupancy ends when all bytes are
        // written in FIFO order, not a WAN receipt later. WireCredit retains
        // every unacknowledged byte independently, including binary checkpoints.
        self.binary.is_none() && self.offset == self.text.len()
    }
}
// Receipts follow wire order, including across transfer IDs. A later transfer's
// receipt cannot forge credit for earlier chunks or binary envelopes.
#[derive(Default)]
struct WireCredit {
    pending: VecDeque<(u32, u32, usize)>,
    bytes: usize,
}
impl WireCredit {
    fn receipt(&mut self, id: u32, offset: u32, active: &mut VecDeque<Transfer>) -> bool {
        let Some(&(expected_id, expected_offset, bytes)) = self.pending.front() else {
            return false;
        };
        if (id, offset) != (expected_id, expected_offset) {
            return false;
        }
        // A fully written transfer already released its assembly/lane slot.
        // Only an exact receipt for the oldest written chunk can release credit.
        if let Some(transfer) = active.iter_mut().find(|t| t.id == id) {
            if transfer.waiting == 0 {
                return false;
            }
            transfer.waiting -= 1;
        }
        self.pending.pop_front();
        self.bytes -= bytes;
        true
    }
}
async fn write<S: Sink<Message> + Unpin>(writer: &mut S, message: Message) -> bool {
    if writer.send(message).await.is_err() {
        let _ = writer.flush().await;
        false
    } else {
        true
    }
}
async fn upgrade<S: Sink<Message> + Unpin>(writer: &mut S) {
    let reason = "relay bounded transport requires an updated client";
    let _ = write(
        writer,
        Message::Text(format!("{{\"kind\":\"close\",\"reason\":\"{reason}\"}}").into()),
    )
    .await;
    let _ = write(
        writer,
        Message::Close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
            code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
            reason: reason.into(),
        })),
    )
    .await;
}
fn receiver_message(message: Message, binary_events: bool) -> Option<Message> {
    // Large binary events use the existing encrypted JSON envelope so every wire
    // frame is bounded without another binary codec. Small binary stays binary.
    if let Message::Binary(bytes) = &message {
        if !binary_events || bytes.len() > CHUNK_BYTES {
            let envelope = crate::binary_event::legacy_client_envelope(bytes).ok()?;
            return serde_json::to_string(&envelope)
                .ok()
                .map(|s| Message::Text(s.into()));
        }
    }
    Some(message)
}
struct Queued {
    message: Message,
    lane: Lane,
}
impl Queued {
    fn new(message: Message, display_route: &impl Fn(&str) -> bool) -> Self {
        let lane = lane(&message, display_route);
        Self { message, lane }
    }
}
fn eligible(pending: &VecDeque<Queued>, index: usize, active: &VecDeque<Transfer>) -> bool {
    let current = pending[index].lane;
    if current == Lane::Reply {
        // Reserve lanes for events and requests. This also bounds receiver state.
        return active.iter().filter(|t| t.lane == Lane::Reply).count() < 2;
    }
    if current == Lane::Ordered {
        return index == 0 && active.is_empty();
    }
    // Events and requests retain their own FIFO; unknown lifecycle envelopes
    // are barriers. Correlated replies may pass them to avoid catalog deadlock.
    !active
        .iter()
        .any(|t| t.lane == current || t.lane == Lane::Ordered)
        && !pending
            .iter()
            .take(index)
            .any(|m| matches!(m.lane, Lane::Ordered) || m.lane == current)
}
pub(super) async fn run_writer<S, F>(
    mut writer: S,
    mut outgoing: mpsc::Receiver<Message>,
    chunks: bool,
    binary_events: bool,
    mut receipts: mpsc::Receiver<(u32, u32)>,
    mut controls: mpsc::Receiver<Message>,
    display_route: F,
) where
    S: Sink<Message> + Unpin,
    F: Fn(&str) -> bool,
{
    let capacity = outgoing.max_capacity();
    let mut pending = VecDeque::new();
    let mut active = VecDeque::<Transfer>::new();
    let mut credit = WireCredit::default();
    let mut next_id = 0u32;
    let mut open = true;
    let mut receipts_open = true;
    let mut controls_open = true;
    loop {
        while let Ok(message) = controls.try_recv() {
            let terminal = matches!(message, Message::Close(_));
            if !write(&mut writer, message).await || terminal {
                return;
            }
        }
        while let Ok((id, offset)) = receipts.try_recv() {
            if !credit.receipt(id, offset, &mut active) {
                return;
            }
        }
        active.retain(|t| !t.finished());
        while pending.len() < capacity {
            match outgoing.try_recv() {
                Ok(message) => match receiver_message(message, binary_events) {
                    Some(m) => pending.push_back(Queued::new(m, &display_route)),
                    None => return,
                },
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    open = false;
                    break;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
            }
        }
        if let Some(index) = pending.iter().position(|queued| control(&queued.message)) {
            let message = pending.remove(index).unwrap().message;
            let terminal = matches!(message, Message::Close(_));
            if !write(&mut writer, message).await || terminal {
                return;
            }
            continue;
        }
        if !chunks {
            if let Some(queued) = pending.pop_front() {
                let message = queued.message;
                if message.len() > CHUNK_BYTES {
                    upgrade(&mut writer).await;
                    return;
                }
                if !write(&mut writer, message).await {
                    return;
                }
                continue;
            }
        } else if active.len() < MAX_TRANSFERS {
            let reserved: usize = active.iter().map(|t| t.text.len()).sum();
            // Start in enqueue order; replies and the event/request lanes can
            // pass blocked transfers but never reorder members of their lane.
            if let Some(index) = (0..pending.len()).find(|&i| {
                eligible(&pending, i, &active)
                    && pending[i].message.len() <= MAX_ASSEMBLY_BYTES - reserved
            }) {
                let queued = pending.remove(index).unwrap();
                let class = queued.lane;
                let message = queued.message;
                if message.len() > MAX_MESSAGE_BYTES {
                    upgrade(&mut writer).await;
                    return;
                }
                let id = 0; // Assigned on the first wire write, not queue admission.
                let (text, binary) = match message {
                    Message::Text(s) => (s.to_string(), None),
                    Message::Binary(_) => (String::new(), Some(message)),
                    _ => {
                        if !write(&mut writer, message).await {
                            return;
                        }
                        continue;
                    }
                };
                active.push_back(Transfer {
                    id,
                    text,
                    binary,
                    offset: 0,
                    lane: class,
                    waiting: 0,
                });
            }
        }
        let mut sent = false;
        // Round robin prevents continuous display traffic starving snapshots.
        for _ in 0..active.len() {
            let mut transfer = active.pop_front().unwrap();
            let starting = transfer.id == 0;
            if starting {
                let Some(id) = next_id.checked_add(1) else {
                    return;
                };
                transfer.id = id;
            }
            if let Some((message, end)) = transfer.next_chunk() {
                let bytes = message.len() + transfer.binary.as_ref().map_or(0, Message::len);
                if credit.bytes + bytes <= WINDOW_BYTES {
                    if starting {
                        next_id = transfer.id;
                    }
                    credit.bytes += bytes;
                    credit.pending.push_back((transfer.id, end as u32, bytes));
                    transfer.offset = end;
                    transfer.waiting += 1;
                    let binary = transfer.binary.take();
                    active.push_back(transfer);
                    if let Some(binary) = binary {
                        if !write(&mut writer, binary).await {
                            return;
                        }
                    }
                    if !write(&mut writer, message).await {
                        return;
                    }
                    sent = true;
                    break;
                }
            }
            if starting {
                transfer.id = 0;
            }
            active.push_back(transfer);
        }
        if sent {
            tokio::task::yield_now().await;
            continue;
        }
        if !open && pending.is_empty() && active.is_empty() && credit.pending.is_empty() {
            return;
        }
        // A closed receipt lane can no longer drain credit. Do not spin forever.
        if chunks && !receipts_open && !active.is_empty() {
            return;
        }
        tokio::select! {
            message=controls.recv(),if controls_open=>{
                if let Some(message)=message {let terminal=matches!(message,Message::Close(_));if !write(&mut writer,message).await||terminal{return;}}else{controls_open=false;}
            }
            receipt=receipts.recv(),if receipts_open=>{
                let Some((id,offset))=receipt else{receipts_open=false;continue;};
                if !credit.receipt(id,offset,&mut active){return;}
            }
            message=outgoing.recv(),if open && pending.len()<capacity=>{
                match message {Some(message)=>match receiver_message(message,binary_events){Some(m)=>pending.push_back(Queued::new(m, &display_route)),None=>return},None=>open=false}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use std::time::Duration;
    async fn sockets() -> (
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            tokio_tungstenite::connect_async(format!("ws://{address}"))
                .await
                .unwrap()
                .0
        });
        let (stream, _) = listener.accept().await.unwrap();
        let server = tokio_tungstenite::accept_async(stream).await.unwrap();
        (server, client.await.unwrap())
    }
    // MP-08/MP-10/MP-11: a real socket fills its wire window before receipts.
    // A medium snapshot, ordered display events and requests all progress without
    // allowing the medium reply's old 2MiB inline bypass.
    #[tokio::test]
    async fn mp08_mp10_mp11_multiplex_medium_reply_and_band_with_bounded_wire() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(16);
        let (ack_tx, ack_rx) = mpsc::channel(16);
        let (_ct, cr) = mpsc::channel(8);
        let catalog=serde_json::json!({"kind":"client_response","request_id":"catalog","payload":"x".repeat(3_000_000)}).to_string();
        tx.send(Message::Text(catalog.into())).await.unwrap();
        let task = tokio::spawn(run_writer(socket, rx, true, true, ack_rx, cr, |_| true));
        let first = client.next().await.unwrap().unwrap();
        let medium=serde_json::json!({"kind":"client_response","request_id":"mirror","payload":"y".repeat(200_000)}).to_string();
        let event = |id, payload| {
            Message::Text(
                serde_json::json!({"kind":"client_event","subscription_id":"display","event_id":id,"payload":payload})
                    .to_string()
                    .into(),
            )
        };
        for message in [Message::Text(medium.into()),event(1,"z".repeat(20000)),event(2,"band".into()),
            Message::Text(serde_json::json!({"kind":"daemon_request","relay_request_id":"A","payload":"q".repeat(90000)}).to_string().into()),
            Message::Text(r#"{"kind":"daemon_request","relay_request_id":"B"}"#.into())] {tx.send(message).await.unwrap();}
        drop(tx);
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let mut message = first;
        let mut done = Vec::new();
        let mut event_ids = Vec::new();
        let mut request_ids = Vec::new();
        let mut wire = 0;
        loop {
            let Message::Text(text) = message else {
                panic!("text frame expected")
            };
            assert!(
                text.len() <= CHUNK_BYTES,
                "all medium reply frames must be bounded"
            );
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed["kind"], "transport_chunk", "no inline bypass");
            wire += text.len();
            let (complete, receipt) = receiver.receive(&text).unwrap();
            let receipt = receipt.unwrap();
            let TransportFrame::TransportAck {
                transfer_id,
                offset,
            } = serde_json::from_str(&receipt).unwrap()
            else {
                panic!()
            };
            ack_tx.send((transfer_id, offset)).await.unwrap();
            if let Some(text) = complete {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if let Some(id) = value["request_id"].as_str() {
                    done.push(id.to_string());
                }
                if value["kind"] == "client_event" {
                    event_ids.push(value["event_id"].as_u64().unwrap());
                    assert!(!done.contains(&"catalog".into()));
                }
                if let Some(id) = value["relay_request_id"].as_str() {
                    request_ids.push(id.to_string());
                }
            }
            if done.len() == 2 && event_ids.len() == 2 && request_ids.len() == 2 {
                break;
            }
            message = tokio::time::timeout(Duration::from_secs(3), client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
        assert_eq!(done, ["mirror", "catalog"]);
        assert_eq!(event_ids, [1, 2]);
        assert_eq!(request_ids, ["A", "B"]);
        assert!(wire > 3_000_000);
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        let _ = client.close(None).await;
    }
    #[test]
    fn mp08_mp10_mp11_combined_credit_rejects_cross_transfer_and_duplicate_receipts() {
        let mut active = VecDeque::from([
            Transfer {
                id: 1,
                text: "x".into(),
                binary: None,
                offset: 1,
                lane: Lane::Reply,
                waiting: 1,
            },
            Transfer {
                id: 2,
                text: "y".into(),
                binary: None,
                offset: 1,
                lane: Lane::Event,
                waiting: 1,
            },
        ]);
        let mut credit = WireCredit {
            pending: VecDeque::from([(1, 1, 100), (2, 1, 200)]),
            bytes: 300,
        };
        assert!(!credit.receipt(2, 1, &mut active));
        assert_eq!(credit.bytes, 300);
        assert!(credit.receipt(1, 1, &mut active));
        assert!(!credit.receipt(1, 1, &mut active));
        assert!(credit.receipt(2, 1, &mut active));
        assert_eq!(credit.bytes, 0);
    }
    // MP-08/MP-10/MP-11: a fully written small event must not consume an
    // assembly lane until its receipt makes a WAN round trip. Only byte credit
    // remains outstanding; all events keep wire order and the 64 KiB bound.
    #[tokio::test]
    async fn mp08_mp10_mp11_small_events_fill_wire_window_without_receipt_serialization() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(32);
        let (_ack_tx, ack_rx) = mpsc::channel(32);
        let (_ct, cr) = mpsc::channel(8);
        for event_id in 1..=24 {
            let header = crate::binary_event::EventHeader {
                kind: "client_event".into(),
                subscription_id: "display".into(),
                event_id,
                sender_public_key: "opaque".into(),
                nonce: "abcdefghijklmnop".into(),
            };
            let bytes = crate::binary_event::encode(&header, &[7; 4096]).unwrap();
            tx.send(Message::Binary(bytes.into())).await.unwrap();
        }
        let task = tokio::spawn(run_writer(socket, rx, true, true, ack_rx, cr, |_| true));
        let mut wire = 0;
        let mut ids = Vec::new();
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        while let Ok(Some(Ok(message))) =
            tokio::time::timeout(Duration::from_millis(150), client.next()).await
        {
            wire += message.len();
            assert!(wire <= WINDOW_BYTES, "combined byte credit exceeded");
            match message {
                Message::Binary(bytes) => {
                    let (h, _) = crate::binary_event::decode(&bytes).unwrap();
                    ids.push(h.event_id);
                }
                Message::Text(text) => {
                    receiver.receive(&text).unwrap();
                }
                _ => panic!("unexpected control"),
            }
        }
        task.abort();
        let _ = task.await;
        assert!(
            ids.len() > MAX_TRANSFERS,
            "small messages stalled on receipt/assembly slots: {ids:?}"
        );
        assert!(
            ids.len() < 24,
            "byte window must stop an unacknowledged stream"
        );
        assert_eq!(ids, (1..=ids.len() as u64).collect::<Vec<_>>());
    }
    #[test]
    fn mp08_mp10_mp11_retired_writes_keep_exact_ordered_byte_credit() {
        let mut active = VecDeque::new();
        let mut credit = WireCredit {
            pending: VecDeque::from([(1, 10, 100), (2, 0, 200)]),
            bytes: 300,
        };
        assert!(!credit.receipt(2, 0, &mut active));
        assert_eq!(credit.bytes, 300);
        assert!(!credit.receipt(1, 11, &mut active));
        assert_eq!(credit.bytes, 300);
        assert!(credit.receipt(1, 10, &mut active));
        assert_eq!(credit.bytes, 200);
        assert!(!credit.receipt(1, 10, &mut active));
        assert_eq!(credit.bytes, 200);
        assert!(credit.receipt(2, 0, &mut active));
        assert_eq!(credit.bytes, 0);
        assert!(!credit.receipt(3, 0, &mut active));
    }
    #[tokio::test]
    async fn mp08_mp10_mp11_small_binary_event_checkpoint_and_window_stop() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(8);
        let (at, ar) = mpsc::channel(16);
        let (_ct, cr) = mpsc::channel(8);
        let header = crate::binary_event::EventHeader {
            kind: "client_event".into(),
            subscription_id: "display".into(),
            event_id: 1,
            sender_public_key: "opaque".into(),
            nonce: "abcdefghijklmnop".into(),
        };
        let binary = crate::binary_event::encode(&header, &[7; 4096]).unwrap();
        tx.send(Message::Text(
            serde_json::json!({"kind":"client_response","payload":"x".repeat(200000)})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        tx.send(Message::Binary(binary.clone().into()))
            .await
            .unwrap();
        drop(tx);
        let task = tokio::spawn(run_writer(socket, rx, true, true, ar, cr, |_| true));
        let mut total = 0;
        let mut receipts = Vec::new();
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let mut found = false;
        // Withhold ALL receipts until the window stops: binary bytes and its
        // checkpoint must count in the same limit as both response streams.
        while let Ok(Some(Ok(message))) =
            tokio::time::timeout(Duration::from_millis(80), client.next()).await
        {
            total += message.len();
            assert!(total <= WINDOW_BYTES);
            match message {
                Message::Binary(b) => {
                    assert_eq!(&b[..], &binary);
                    found = true;
                }
                Message::Text(text) => {
                    let (_, ack) = receiver.receive(&text).unwrap();
                    receipts.push(ack.unwrap());
                }
                _ => {}
            }
        }
        assert!(found);
        assert!(total > CHUNK_BYTES);
        for receipt in receipts {
            let TransportFrame::TransportAck {
                transfer_id,
                offset,
            } = serde_json::from_str(&receipt).unwrap()
            else {
                panic!()
            };
            at.send((transfer_id, offset)).await.unwrap();
        }
        while let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(200), client.next()).await
        {
            let (complete, ack) = receiver.receive(&text).unwrap();
            let TransportFrame::TransportAck {
                transfer_id,
                offset,
            } = serde_json::from_str(&ack.unwrap()).unwrap()
            else {
                panic!()
            };
            at.send((transfer_id, offset)).await.unwrap();
            if complete.is_some() {
                break;
            }
        }
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        let _ = client.close(None).await;
    }
    #[tokio::test]
    async fn legacy_large_response_retains_upgrade_reason_in_native_close() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(8);
        let (_at, ar) = mpsc::channel(8);
        let (_ct, cr) = mpsc::channel(8);
        tx.send(Message::Text("x".repeat(CHUNK_BYTES + 1).into()))
            .await
            .unwrap();
        let task = tokio::spawn(run_writer(socket, rx, false, false, ar, cr, |_| false));
        assert!(
            matches!(client.next().await.unwrap().unwrap(),Message::Text(t) if t.contains("requires an updated client"))
        );
        assert!(
            matches!(client.next().await.unwrap().unwrap(),Message::Close(Some(c)) if c.reason=="relay bounded transport requires an updated client")
        );
        task.await.unwrap();
        let _ = client.close(None).await;
    }
    // MP-08/MP-10/MP-11: ordinary session tail cannot precede its initial snapshot.
    #[tokio::test]
    async fn mp08_mp10_mp11_terminal_event_stays_behind_initial_snapshot() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(8);
        let (at, ar) = mpsc::channel(8);
        let (_ct, cr) = mpsc::channel(8);
        tx.send(Message::Text(serde_json::json!({"kind":"client_response","request_id":"snapshot","payload":"x".repeat(200000)}).to_string().into())).await.unwrap();
        let task = tokio::spawn(run_writer(socket, rx, true, false, ar, cr, |_| false));
        let mut message = client.next().await.unwrap().unwrap();
        tx.send(Message::Text(
            r#"{"kind":"client_event","subscription_id":"terminal","event_id":1,"payload":"tail"}"#
                .into(),
        ))
        .await
        .unwrap();
        drop(tx);
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let mut snapshot = false;
        loop {
            let Message::Text(text) = message else {
                panic!()
            };
            let (complete, ack) = receiver.receive(&text).unwrap();
            let TransportFrame::TransportAck {
                transfer_id,
                offset,
            } = serde_json::from_str(&ack.unwrap()).unwrap()
            else {
                panic!()
            };
            at.send((transfer_id, offset)).await.unwrap();
            if let Some(text) = complete {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["kind"] == "client_event" {
                    assert!(
                        snapshot,
                        "terminal tail cannot overtake its initial snapshot"
                    );
                    break;
                }
                snapshot = true;
            }
            message = tokio::time::timeout(Duration::from_secs(3), client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
        task.await.unwrap();
        let _ = client.close(None).await;
    }
}
