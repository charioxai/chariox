//! MP-08 / MP-10 / MP-11: bounded opaque multiplexing (532 / peer 140).
use crate::frame_transport::{
    TransportFrame, CHUNK_BYTES, MAX_ASSEMBLY_BYTES, MAX_MESSAGE_BYTES, MAX_TRANSFERS,
    PEER_RECEIPT_FRAMES, SMALL_RESERVE_BYTES, WINDOW_BYTES,
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
/// Display events keep one FIFO per subscription (the stream), not one for all
/// streams, so a small band frame is never queued behind another stream's bulk.
fn lane(message: &Message, display_route: &impl Fn(&str) -> bool) -> (Lane, Option<String>) {
    if let Message::Binary(bytes) = message {
        return match crate::binary_event::decode(bytes) {
            Ok((header, _))
                if header.kind == "client_event" && display_route(&header.subscription_id) =>
            {
                (Lane::Event, Some(header.subscription_id))
            }
            _ => (Lane::Ordered, None),
        };
    }
    let Message::Text(text) = message else {
        return (Lane::Ordered, None);
    };
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return (Lane::Ordered, None),
    };
    match value.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
        "client_response"
        | "client_metadata_response"
        | "daemon_peer_response"
        | "client_connected" => (Lane::Reply, None),
        "client_event" => match value.get("subscription_id").and_then(|v| v.as_str()) {
            Some(id) if display_route(id) => (Lane::Event, Some(id.to_string())),
            _ => (Lane::Ordered, None),
        },
        kind if kind.ends_with("request") => (Lane::Request, None),
        _ => (Lane::Ordered, None),
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
    key: Option<String>,
    small: bool,
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
    pending: VecDeque<(u32, u32, usize, bool)>,
    bytes: usize,
    // Bytes of `bytes` held by single-chunk messages.
    small: usize,
}
impl WireCredit {
    fn receipt(&mut self, id: u32, offset: u32, active: &mut VecDeque<Transfer>) -> bool {
        let Some(&(expected_id, expected_offset, bytes, small)) = self.pending.front() else {
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
        if small {
            self.small -= bytes;
        }
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
    key: Option<String>,
}
impl Queued {
    fn new(message: Message, display_route: &impl Fn(&str) -> bool) -> Self {
        let (lane, key) = lane(&message, display_route);
        Self { message, lane, key }
    }
    // A single-chunk message: interactive frames, never bulk.
    fn small(&self) -> bool {
        self.message.len() <= CHUNK_BYTES
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
    // Requests retain one FIFO and display events one FIFO per subscription;
    // unknown lifecycle envelopes are barriers. Correlated replies may pass
    // them to avoid catalog deadlock.
    let same = |lane: Lane, key: &Option<String>| lane == current && *key == pending[index].key;
    !active
        .iter()
        .any(|t| same(t.lane, &t.key) || t.lane == Lane::Ordered)
        && !pending
            .iter()
            .take(index)
            .any(|m| matches!(m.lane, Lane::Ordered) || same(m.lane, &m.key))
}
// Bulk transfers leave the last slot of each idle control lane (requests,
// correlated replies) free, so display bulk can never hold every receiver slot.
fn leaves_control_slots(lane: Lane, active: &VecDeque<Transfer>) -> bool {
    let idle = [Lane::Reply, Lane::Request]
        .into_iter()
        .filter(|&l| l != lane && !active.iter().any(|t| t.lane == l))
        .count();
    MAX_TRANSFERS - active.len() > idle
}
// Consecutive small sends allowed while a bulk transfer waits for its turn.
const SMALL_STREAK: usize = 4;
pub(super) async fn run_writer<S, F>(
    mut writer: S,
    mut outgoing: mpsc::Receiver<Message>,
    chunks: bool,
    binary_events: bool,
    mut receipts: mpsc::Receiver<(u32, u32)>,
    mut controls: mpsc::Receiver<Message>,
    display_route: F,
) -> &'static str
where
    S: Sink<Message> + Unpin,
    F: Fn(&str) -> bool,
{
    let capacity = outgoing.max_capacity();
    let mut pending = VecDeque::new();
    let mut active = VecDeque::<Transfer>::new();
    let mut credit = WireCredit::default();
    let mut next_id = 0u32;
    let mut small_streak = 0;
    let mut open = true;
    let mut receipts_open = true;
    let mut controls_open = true;
    loop {
        while let Ok(message) = controls.try_recv() {
            let terminal = matches!(message, Message::Close(_));
            if !write(&mut writer, message).await || terminal {
                return "control_write";
            }
        }
        while let Ok((id, offset)) = receipts.try_recv() {
            if !credit.receipt(id, offset, &mut active) {
                return "bad_receipt";
            }
        }
        active.retain(|t| !t.finished());
        while pending.len() < capacity {
            match outgoing.try_recv() {
                Ok(message) => match receiver_message(message, binary_events) {
                    Some(m) => pending.push_back(Queued::new(m, &display_route)),
                    None => return "bad_outgoing_message",
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
                return "control_write";
            }
            continue;
        }
        if !chunks {
            if let Some(queued) = pending.pop_front() {
                let message = queued.message;
                if message.len() > CHUNK_BYTES {
                    upgrade(&mut writer).await;
                    return "unchunked_oversize";
                }
                if !write(&mut writer, message).await {
                    return "write_failed";
                }
                continue;
            }
        } else if active.len() < MAX_TRANSFERS {
            let reserved: usize = active.iter().map(|t| t.text.len()).sum();
            // Start small messages first, otherwise in enqueue order; replies and
            // the event/request lanes can pass blocked transfers but never
            // reorder members of their FIFO. A barrier ends the small-first scan.
            let startable = |i: usize| {
                eligible(&pending, i, &active)
                    && (pending[i].small() || leaves_control_slots(pending[i].lane, &active))
                    && pending[i].message.len() <= MAX_ASSEMBLY_BYTES - reserved
            };
            let barrier = pending
                .iter()
                .position(|m| m.lane == Lane::Ordered)
                .map_or(pending.len(), |i| i + 1);
            if let Some(index) = (0..barrier)
                .find(|&i| pending[i].small() && startable(i))
                .or_else(|| (0..pending.len()).find(|&i| startable(i)))
            {
                let queued = pending.remove(index).unwrap();
                let class = queued.lane;
                let key = queued.key.clone();
                let small = queued.small();
                let message = queued.message;
                if message.len() > MAX_MESSAGE_BYTES {
                    upgrade(&mut writer).await;
                    return "message_too_large";
                }
                let id = 0; // Assigned on the first wire write, not queue admission.
                let (text, binary) = match message {
                    Message::Text(s) => (s.to_string(), None),
                    Message::Binary(_) => (String::new(), Some(message)),
                    _ => {
                        if !write(&mut writer, message).await {
                            return "write_failed";
                        }
                        continue;
                    }
                };
                let transfer = Transfer {
                    id,
                    key,
                    small,
                    text,
                    binary,
                    offset: 0,
                    lane: class,
                    waiting: 0,
                };
                if small {
                    active.push_front(transfer);
                } else {
                    active.push_back(transfer);
                }
            }
        }
        let mut sent = false;
        // Round robin prevents continuous display traffic starving snapshots.
        // Small messages go first, but after SMALL_STREAK of them a waiting bulk
        // transfer gets the next chunk.
        let bulk_turn = small_streak >= SMALL_STREAK && active.iter().any(|t| !t.small);
        'passes: for pass in 0..2 {
            for _ in 0..active.len() {
                let mut transfer = active.pop_front().unwrap();
                if pass == 0 && bulk_turn && transfer.small {
                    active.push_back(transfer);
                    continue;
                }
                let starting = transfer.id == 0;
                if starting {
                    let Some(id) = next_id.checked_add(1) else {
                        return "transfer_id_exhausted";
                    };
                    transfer.id = id;
                }
                if let Some((message, end)) = transfer.next_chunk() {
                    let bytes = message.len() + transfer.binary.as_ref().map_or(0, Message::len);
                    // While bulk is active, small traffic may hold only its reserve,
                    // so it can never keep the window above what a bulk chunk needs.
                    let fits = credit.pending.len() < PEER_RECEIPT_FRAMES
                        && if transfer.small {
                            credit.bytes + bytes <= WINDOW_BYTES + SMALL_RESERVE_BYTES
                                && (active.iter().all(|t| t.small)
                                    || credit.small + bytes <= SMALL_RESERVE_BYTES)
                        } else {
                            credit.bytes + bytes <= WINDOW_BYTES
                        };
                    if fits {
                        if starting {
                            next_id = transfer.id;
                        }
                        credit.bytes += bytes;
                        if transfer.small {
                            credit.small += bytes;
                        }
                        credit
                            .pending
                            .push_back((transfer.id, end as u32, bytes, transfer.small));
                        small_streak = if transfer.small { small_streak + 1 } else { 0 };
                        transfer.offset = end;
                        transfer.waiting += 1;
                        let binary = transfer.binary.take();
                        active.push_back(transfer);
                        if let Some(binary) = binary {
                            if !write(&mut writer, binary).await {
                                return "write_failed";
                            }
                        }
                        if !write(&mut writer, message).await {
                            return "write_failed";
                        }
                        sent = true;
                        break 'passes;
                    }
                }
                if starting {
                    transfer.id = 0;
                }
                active.push_back(transfer);
            }
            if !bulk_turn {
                break;
            }
        }
        if sent {
            tokio::task::yield_now().await;
            continue;
        }
        if !open && pending.is_empty() && active.is_empty() && credit.pending.is_empty() {
            return "drained";
        }
        // A closed receipt lane can no longer drain credit. Do not spin forever.
        if chunks && !receipts_open && !active.is_empty() {
            return "receipts_closed";
        }
        tokio::select! {
            message=controls.recv(),if controls_open=>{
                if let Some(message)=message {let terminal=matches!(message,Message::Close(_));if !write(&mut writer,message).await||terminal{return "control_write";}}else{controls_open=false;}
            }
            receipt=receipts.recv(),if receipts_open=>{
                let Some((id,offset))=receipt else{receipts_open=false;continue;};
                if !credit.receipt(id,offset,&mut active){return "bad_receipt";}
            }
            message=outgoing.recv(),if open && pending.len()<capacity=>{
                match message {Some(message)=>match receiver_message(message,binary_events){Some(m)=>pending.push_back(Queued::new(m, &display_route)),None=>return "bad_outgoing_message"},None=>open=false}
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
                key: None,
                small: true,
                text: "x".into(),
                binary: None,
                offset: 1,
                lane: Lane::Reply,
                waiting: 1,
            },
            Transfer {
                id: 2,
                key: None,
                small: true,
                text: "y".into(),
                binary: None,
                offset: 1,
                lane: Lane::Event,
                waiting: 1,
            },
        ]);
        let mut credit = WireCredit {
            pending: VecDeque::from([(1, 1, 100, false), (2, 1, 200, false)]),
            bytes: 300,
            small: 0,
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
    // remains outstanding; all events keep wire order and the window plus small reserve bound.
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
            assert!(
                wire <= WINDOW_BYTES + SMALL_RESERVE_BYTES,
                "combined byte credit exceeded"
            );
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
            ids.len() < (WINDOW_BYTES + SMALL_RESERVE_BYTES) / 4096 + 2,
            "byte window must stop an unacknowledged stream"
        );
        assert_eq!(ids, (1..=ids.len() as u64).collect::<Vec<_>>());
    }
    // An older peer's receipt queue holds PEER_RECEIPT_FRAMES entries: tiny
    // events must never put more frames than that on the wire unacknowledged,
    // whatever the byte window is.
    #[tokio::test]
    async fn tiny_events_never_exceed_the_peer_receipt_queue() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(4096);
        let (_ack_tx, ack_rx) = mpsc::channel(32);
        let (_ct, cr) = mpsc::channel(8);
        for _ in 0..4000 {
            tx.send(Message::Text("{}".into())).await.unwrap();
        }
        let task = tokio::spawn(run_writer(socket, rx, true, false, ack_rx, cr, |_| true));
        let mut frames = 0;
        while let Ok(Some(Ok(_))) =
            tokio::time::timeout(Duration::from_millis(300), client.next()).await
        {
            frames += 1;
        }
        task.abort();
        let _ = task.await;
        assert!(frames > 0, "nothing was sent");
        assert!(
            frames <= PEER_RECEIPT_FRAMES,
            "{frames} unacknowledged frames exceed the peer receipt queue"
        );
    }
    // S1: a large message waits one round trip per window. 480 KB (a mirror
    // first packet) must need at most two flights, not one per 64 KiB.
    #[tokio::test]
    async fn s1_large_message_completes_in_two_round_trips() {
        let (wire_tx, mut wire) = mpsc::unbounded_channel();
        let sink = Box::pin(futures_util::sink::unfold(
            wire_tx,
            |tx, message: Message| async move {
                let _ = tx.send(message);
                Ok::<_, std::convert::Infallible>(tx)
            },
        ));
        let (tx, rx) = mpsc::channel(4);
        let (ack_tx, ack_rx) = mpsc::channel(crate::frame_transport::MAX_UNACKED_FRAMES);
        let (_ct, cr) = mpsc::channel(8);
        let reply = serde_json::json!({"kind":"client_response","request_id":"mirror","payload":"y".repeat(480_000)}).to_string();
        tx.send(Message::Text(reply.clone().into())).await.unwrap();
        let task = tokio::spawn(run_writer(sink, rx, true, true, ack_rx, cr, |_| true));
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let (mut flights, mut done) = (0, None);
        loop {
            // A flight is everything the writer sends before it must wait for receipts.
            let mut flight = Vec::new();
            while let Ok(Some(message)) =
                tokio::time::timeout(Duration::from_millis(40), wire.recv()).await
            {
                flight.push(message);
            }
            if flight.is_empty() {
                break;
            }
            flights += 1;
            for message in flight {
                let Message::Text(text) = message else {
                    panic!("unexpected frame")
                };
                let (complete, receipt) = receiver.receive(&text).unwrap();
                done = done.or(complete);
                let TransportFrame::TransportAck {
                    transfer_id,
                    offset,
                } = serde_json::from_str(&receipt.unwrap()).unwrap()
                else {
                    panic!("receipt expected")
                };
                ack_tx.send((transfer_id, offset)).await.unwrap();
            }
        }
        task.abort();
        assert_eq!(done, Some(reply));
        assert!(flights <= 2, "480 KB needed {flights} round trips");
    }
    // MP-08/MP-10: queued mirror credits answer back-to-back. A written reply
    // must not hold one of the two reply slots until its WAN receipt returns
    // (appsfix r14: packets 2-3 of one click arrived one RTT apart).
    #[tokio::test]
    async fn mp08_mp10_small_replies_do_not_wait_for_receipts() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(32);
        let (_ack_tx, ack_rx) = mpsc::channel(32);
        let (_ct, cr) = mpsc::channel(8);
        for id in 1..=4 {
            let reply = serde_json::json!({"kind": "client_response", "request_id": format!("m{id}"), "payload": "x".repeat(600)});
            tx.send(Message::Text(reply.to_string().into()))
                .await
                .unwrap();
        }
        let task = tokio::spawn(run_writer(socket, rx, true, true, ack_rx, cr, |_| true));
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let mut ids = Vec::new();
        while let Ok(Some(Ok(Message::Text(text)))) =
            tokio::time::timeout(Duration::from_millis(150), client.next()).await
        {
            if let (Some(complete), _) = receiver.receive(&text).unwrap() {
                let value: serde_json::Value = serde_json::from_str(&complete).unwrap();
                ids.push(value["request_id"].as_str().unwrap().to_string());
            }
        }
        task.abort();
        let _ = task.await;
        assert_eq!(ids, ["m1", "m2", "m3", "m4"], "replies stalled on receipts");
    }
    #[test]
    fn mp08_mp10_mp11_retired_writes_keep_exact_ordered_byte_credit() {
        let mut active = VecDeque::new();
        let mut credit = WireCredit {
            pending: VecDeque::from([(1, 10, 100, false), (2, 0, 200, false)]),
            bytes: 300,
            small: 0,
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

    fn display_event(subscription: &str, event_id: u64, payload: String) -> Message {
        Message::Text(
            serde_json::json!({"kind":"client_event","subscription_id":subscription,"event_id":event_id,"payload":payload})
                .to_string()
                .into(),
        )
    }
    // MP-08/MP-10/MP-11: a small band frame must not wait for the page stream's
    // queued bulk frames; each display subscription keeps its own FIFO.
    #[tokio::test]
    async fn small_band_event_overtakes_queued_page_frames_of_another_subscription() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(16);
        let (at, ar) = mpsc::channel(256);
        let (_ct, cr) = mpsc::channel(8);
        for id in 1..=3 {
            tx.send(display_event("page", id, "p".repeat(60_000)))
                .await
                .unwrap();
        }
        tx.send(display_event("band", 1, "band".into()))
            .await
            .unwrap();
        drop(tx);
        let task = tokio::spawn(run_writer(socket, rx, true, false, ar, cr, |_| true));
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let mut done = Vec::new();
        while done.len() < 4 {
            let Message::Text(text) = tokio::time::timeout(Duration::from_secs(3), client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
            else {
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
                done.push(format!(
                    "{}{}",
                    value["subscription_id"].as_str().unwrap(),
                    value["event_id"]
                ));
            }
        }
        assert!(
            done.iter().position(|d| d == "band1").unwrap() <= 1,
            "band frame waited behind page frames: {done:?}"
        );
        let pages: Vec<_> = done.iter().filter(|d| d.starts_with("page")).collect();
        assert_eq!(
            pages,
            ["page1", "page2", "page3"],
            "one subscription stays FIFO"
        );
        task.await.unwrap();
        let _ = client.close(None).await;
    }
    // MP-08/MP-10/MP-11: a small frame still goes out when bulk data fills the
    // receipt window, using a bounded reserve instead of waiting a round trip.
    #[tokio::test]
    async fn small_event_uses_reserved_credit_when_bulk_fills_the_window() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(16);
        let (_at, ar) = mpsc::channel(256);
        let (_ct, cr) = mpsc::channel(8);
        tx.send(display_event("page", 1, "p".repeat(400_000)))
            .await
            .unwrap();
        let task = tokio::spawn(run_writer(socket, rx, true, false, ar, cr, |_| true));
        let mut wire = 0;
        while let Ok(Some(Ok(message))) =
            tokio::time::timeout(Duration::from_millis(150), client.next()).await
        {
            wire += message.len();
        }
        assert!(
            wire >= WINDOW_BYTES - CHUNK_BYTES,
            "bulk must fill the window first"
        );
        tx.send(display_event("band", 1, "band".into()))
            .await
            .unwrap();
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let band = loop {
            let Message::Text(text) =
                tokio::time::timeout(Duration::from_millis(500), client.next())
                    .await
                    .expect("a small frame must not wait for a receipt round trip")
                    .unwrap()
                    .unwrap()
            else {
                panic!()
            };
            if let Some(text) = receiver.receive(&text).unwrap().0 {
                break text;
            }
        };
        assert!(band.contains("band"));
        task.abort();
        let _ = task.await;
    }
    // MP-08/MP-10/MP-11: a continuously replenished small producer must not
    // starve an already-started bulk reply, with prompt or round-trip-delayed receipts.
    async fn bulk_reply_under_small_producer(ack_delay: Duration) {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(16);
        let (at, ar) = mpsc::channel(256);
        let (_ct, cr) = mpsc::channel(8);
        let reply = serde_json::json!({"kind":"client_response","request_id":"catalog","payload":"x".repeat(400_000)});
        tx.send(Message::Text(reply.to_string().into()))
            .await
            .unwrap();
        let task = tokio::spawn(run_writer(socket, rx, true, false, ar, cr, |_| true));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let producer = tokio::spawn(async move {
            for id in 1..200_000u64 {
                if flag.load(std::sync::atomic::Ordering::Relaxed)
                    || tx
                        .send(display_event("band", id, "b".repeat(300)))
                        .await
                        .is_err()
                {
                    break;
                }
            }
        });
        let (due_tx, mut due_rx) = mpsc::unbounded_channel::<(tokio::time::Instant, (u32, u32))>();
        let pacer = tokio::spawn(async move {
            while let Some((due, ack)) = due_rx.recv().await {
                tokio::time::sleep_until(due).await;
                if at.send(ack).await.is_err() {
                    break;
                }
            }
        });
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let outcome = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let Message::Text(text) = client.next().await.unwrap().unwrap() else {
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
                let _ = due_tx.send((
                    tokio::time::Instant::now() + ack_delay,
                    (transfer_id, offset),
                ));
                if complete.is_some_and(|t| t.contains("catalog")) {
                    break;
                }
            }
        })
        .await;
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        task.abort();
        pacer.abort();
        producer.abort();
        outcome.expect("the started bulk reply starved behind small events");
    }
    #[tokio::test]
    async fn bulk_reply_advances_under_continuous_small_events() {
        bulk_reply_under_small_producer(Duration::ZERO).await;
    }
    #[tokio::test]
    async fn bulk_reply_advances_under_small_events_with_delayed_receipts() {
        bulk_reply_under_small_producer(Duration::from_millis(30)).await;
    }
    // MP-08/MP-10/MP-11: bulk display events from distinct subscriptions must
    // leave receiver capacity for a later request and correlated reply.
    #[tokio::test]
    async fn requests_and_replies_start_while_bulk_events_hold_the_transfers() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(16);
        let (_at, ar) = mpsc::channel(256);
        let (_ct, cr) = mpsc::channel(8);
        for sub in ["s1", "s2", "s3", "s4"] {
            tx.send(display_event(sub, 1, "p".repeat(300_000)))
                .await
                .unwrap();
        }
        let task = tokio::spawn(run_writer(socket, rx, true, false, ar, cr, |_| true));
        while let Ok(Some(Ok(_))) =
            tokio::time::timeout(Duration::from_millis(200), client.next()).await
        {}
        tx.send(Message::Text(
            r#"{"kind":"client_response","request_id":"reply","payload":"ok"}"#.into(),
        ))
        .await
        .unwrap();
        tx.send(Message::Text(
            r#"{"kind":"daemon_request","relay_request_id":"R"}"#.into(),
        ))
        .await
        .unwrap();
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let mut seen = Vec::new();
        while seen.len() < 2 {
            let Message::Text(text) =
                tokio::time::timeout(Duration::from_millis(500), client.next())
                    .await
                    .expect("a request or reply waited for a whole bulk event")
                    .unwrap()
                    .unwrap()
            else {
                panic!()
            };
            if let Some(done) = receiver.receive(&text).unwrap().0 {
                assert!(
                    !done.contains("subscription_id"),
                    "a bulk event finished first"
                );
                seen.push(done);
            }
        }
        task.abort();
        let _ = task.await;
    }
}
