//! MP-08 / MP-10 / MP-11: bounded opaque multiplexing (532 / peer 140).
#[cfg(test)]
use crate::drain_window::START_WINDOW_BYTES;
use crate::drain_window::{DrainWindow, SendStamp};
use crate::frame_transport::{
    binary_chunk, binary_chunk_payload, text_chunk, TransportFrame, CHUNK_BYTES,
    MAX_ASSEMBLY_BYTES, MAX_MESSAGE_BYTES, MAX_TRANSFERS, PEER_RECEIPT_FRAMES, SMALL_RESERVE_BYTES,
};
use crate::meter::{ConnectionMeter, Direction, CONTROL_SCOPE, DISPLAY_TUNNEL_SCOPE};
use crate::outgoing::{backlog_close_frame, OutgoingQueue};
use futures_util::{Sink, SinkExt};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

enum Stream {
    Subscription(String),
    DisplayTunnel,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    Reply,
    Request,
    Event,
    Ordered,
}
/// Display events keep one FIFO per subscription (the stream), not one for all
/// streams, so a small band frame is never queued behind another stream's bulk.
/// The lane, the per-stream queue key (display events only) and the
/// stream a frame belongs to (any scope, for byte metering).
fn lane(message: &Message, usage: &WriterUsage) -> (Lane, Option<String>, Option<Stream>) {
    if let Message::Binary(bytes) = message {
        if matches!(
            crate::binary_response::decode(bytes),
            Ok((
                crate::binary_response::ResponseHeader::ClientResponse { .. },
                _
            ))
        ) {
            return (Lane::Reply, None, None);
        }
        if bytes.starts_with(b"CXR3") {
            return (Lane::Request, None, None);
        }
        return match crate::binary_event::decode(bytes) {
            Ok((header, _)) if header.kind == "client_event" => {
                let id = header.subscription_id;
                let key = usage.is_display(&id).then(|| id.clone());
                (
                    if key.is_some() {
                        Lane::Event
                    } else {
                        Lane::Ordered
                    },
                    key,
                    Some(Stream::Subscription(id)),
                )
            }
            _ => (Lane::Ordered, None, None),
        };
    }
    let Message::Text(text) = message else {
        return (Lane::Ordered, None, None);
    };
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return (Lane::Ordered, None, None),
    };
    match value.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
        "client_response"
        | "client_metadata_response"
        | "daemon_peer_response"
        | "client_connected" => (Lane::Reply, None, None),
        "client_event" => match value.get("subscription_id").and_then(|v| v.as_str()) {
            Some(id) if usage.is_display(id) => (
                Lane::Event,
                Some(id.to_string()),
                Some(Stream::Subscription(id.to_string())),
            ),
            Some(id) => (
                Lane::Ordered,
                None,
                Some(Stream::Subscription(id.to_string())),
            ),
            None => (Lane::Ordered, None, None),
        },
        kind if crate::meter::is_display_tunnel_data_kind(kind) => {
            (Lane::Ordered, None, Some(Stream::DisplayTunnel))
        }
        kind if kind.ends_with("request") => (Lane::Request, None, None),
        _ => (Lane::Ordered, None, None),
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
    scope: Arc<str>,
    key: Option<String>,
    small: bool,
    text: String,
    binary: Option<Message>,
    offset: usize,
    lane: Lane,
    waiting: usize,
}
impl Transfer {
    /// The next frame; a text frame stays within `limit` bytes, and so does a binary one toward a
    /// receiver that takes chunks shorter than a full one (`adaptive`).
    fn next_chunk(&self, limit: usize, adaptive: bool) -> Option<(Message, usize)> {
        if let Some(Message::Binary(binary)) = &self.binary {
            if binary.len() > CHUNK_BYTES {
                if self.offset == binary.len() {
                    return None;
                }
                let end = (self.offset + binary_chunk_payload(limit, adaptive)).min(binary.len());
                return Some((
                    Message::Binary(
                        binary_chunk(
                            self.id,
                            self.offset,
                            binary.len(),
                            &binary[self.offset..end],
                        )
                        .into(),
                    ),
                    end,
                ));
            }
            let checkpoint = serde_json::to_string(&TransportFrame::TransportChunk {
                transfer_id: self.id,
                offset: 0,
                total_bytes: 0,
                payload: String::new(),
            })
            .ok()?;
            return Some((Message::Text(checkpoint.into()), 0));
        }
        text_chunk(self.id, &self.text, self.offset, limit)
            .map(|(frame, end)| (Message::Text(frame.into()), end))
    }
    fn finished(&self) -> bool {
        // MP-08/MP-10/MP-11: assembly/lane occupancy ends when all bytes are
        // written in FIFO order, not a WAN receipt later. WireCredit retains
        // every unacknowledged byte independently, including binary checkpoints.
        match &self.binary {
            Some(binary) => binary.len() > CHUNK_BYTES && self.offset == binary.len(),
            None => self.offset == self.text.len(),
        }
    }
}
// Receipts follow wire order, including across transfer IDs. A later transfer's
// receipt cannot forge credit for earlier chunks or binary envelopes.
struct WireCredit {
    pending: VecDeque<(u32, u32, usize, bool, SendStamp)>,
    bytes: usize,
    // Bytes of `bytes` held by single-chunk messages.
    small: usize,
    drain: DrainWindow,
}
impl WireCredit {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            bytes: 0,
            small: 0,
            drain: DrainWindow::new(Instant::now()),
        }
    }
    fn sent(&mut self, id: u32, end: u32, bytes: usize, small: bool) {
        let stamp = self.drain.sent(Instant::now(), self.pending.is_empty());
        self.bytes += bytes;
        if small {
            self.small += bytes;
        }
        self.pending.push_back((id, end, bytes, small, stamp));
    }
    fn receipt(&mut self, id: u32, offset: u32, active: &mut VecDeque<Transfer>) -> bool {
        let Some(&(expected_id, expected_offset, bytes, small, stamp)) = self.pending.front()
        else {
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
        self.drain.received(Instant::now(), bytes, stamp, !small);
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
fn receiver_message(
    message: Message,
    binary_events: bool,
    binary_requests: bool,
) -> Option<Message> {
    if binary_requests {
        if let Message::Text(text) = &message {
            if text.starts_with("{\"kind\":\"daemon_request\"") {
                if let Ok(envelope) = serde_json::from_str::<crate::protocol::RelayEnvelope>(text) {
                    if let Ok(Some(bytes)) = crate::binary_request::from_envelope(&envelope) {
                        return Some(Message::Binary(bytes.into()));
                    }
                }
            }
        }
    }
    if binary_events {
        if let Message::Text(text) = &message {
            if text.starts_with("{\"kind\":\"client_response\"") {
                if let Ok(envelope) = serde_json::from_str::<crate::protocol::RelayEnvelope>(text) {
                    if let Ok(Some(bytes)) = crate::binary_response::from_envelope(&envelope) {
                        return Some(Message::Binary(bytes.into()));
                    }
                }
            }
        }
    }

    // Negotiated events keep their ciphertext opaque at every size. The bounded
    // transfer chunks them below; only non-binary clients need the JSON envelope.
    if let Message::Binary(bytes) = &message {
        if !binary_events {
            let envelope = if bytes.starts_with(b"CXR2") {
                crate::binary_response::into_envelope(bytes).ok()?
            } else {
                crate::binary_event::legacy_client_envelope(bytes).ok()?
            };
            return serde_json::to_string(&envelope)
                .ok()
                .map(|s| Message::Text(s.into()));
        }
    }
    Some(message)
}
/// Counts what the writer puts on the socket, by the scope of the stream the
/// frame belongs to (control when it belongs to none). Chunk frames count as
/// sent, so the totals are the payload bytes the peer receives.
type ScopeOf = Box<dyn Fn(&str) -> Option<Arc<str>> + Send>;
pub(super) struct WriterUsage {
    meter: ConnectionMeter,
    /// The scope of this socket's own subscription, if the id is one.
    scope_of: ScopeOf,
    /// What the writer has taken off this connection's queue.
    queue: Arc<OutgoingQueue>,
}
impl WriterUsage {
    pub(super) fn new(
        meter: ConnectionMeter,
        scope_of: impl Fn(&str) -> Option<Arc<str>> + Send + 'static,
        queue: Arc<OutgoingQueue>,
    ) -> Self {
        Self {
            meter,
            scope_of: Box::new(scope_of),
            queue,
        }
    }
    /// Display subscriptions keep their own queue per stream.
    fn is_display(&self, subscription_id: &str) -> bool {
        (self.scope_of)(subscription_id)
            .is_some_and(|scope| crate::frame_transport::is_display_scope(Some(&scope)))
    }
    /// A writer without a meter whose every subscription is (or is not) a display one.
    #[cfg(test)]
    fn fixture(display: bool) -> Self {
        Self::fixture_with_queue(
            display,
            crate::outgoing::RelaySender::detached(mpsc::channel(1).0).queue(),
        )
    }
    #[cfg(test)]
    fn fixture_with_queue(display: bool, queue: Arc<OutgoingQueue>) -> Self {
        Self::new(
            ConnectionMeter::detached(),
            move |_| display.then(|| Arc::from(crate::frame_transport::DISPLAY_EVENT_SCOPE)),
            queue,
        )
    }
    fn scope(&self, stream: Option<&Stream>) -> Arc<str> {
        match stream {
            Some(Stream::Subscription(id)) => (self.scope_of)(id),
            Some(Stream::DisplayTunnel) => Some(Arc::from(DISPLAY_TUNNEL_SCOPE)),
            None => None,
        }
        .unwrap_or_else(|| Arc::from(CONTROL_SCOPE))
    }
    fn released(&self, queued: &Queued) {
        self.queue.release(queued.admitted);
    }
    fn sent(&self, scope: &str, message: &Message) {
        if matches!(message, Message::Text(_) | Message::Binary(_)) {
            self.meter.record(Direction::Out, scope, message.len());
        }
    }
}
struct Queued {
    message: Message,
    /// The size the sender queued it at; `receiver_message` may change the message.
    admitted: usize,
    lane: Lane,
    key: Option<String>,
    scope: Arc<str>,
}
impl Queued {
    fn new(message: Message, admitted: usize, usage: &WriterUsage) -> Self {
        let (lane, key, stream) = lane(&message, usage);
        let scope = usage.scope(stream.as_ref());
        Self {
            message,
            admitted,
            lane,
            key,
            scope,
        }
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
        // A single-chunk reply completes as it is written, so bulk replies never hold it back.
        return pending[index].small()
            || active.iter().filter(|t| t.lane == Lane::Reply).count() < 2;
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
// correlated replies) free, so display bulk can never hold every receiver slot,
// and always one slot, so a small frame never waits for a whole bulk transfer.
fn leaves_control_slots(lane: Lane, active: &VecDeque<Transfer>) -> bool {
    let idle = [Lane::Reply, Lane::Request]
        .into_iter()
        .filter(|&l| l != lane && !active.iter().any(|t| t.lane == l))
        .count();
    MAX_TRANSFERS - active.len() > idle.max(1)
}
// Consecutive small sends allowed while a bulk transfer waits for its turn.
const SMALL_STREAK: usize = 4;
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_writer<S>(
    mut writer: S,
    mut outgoing: mpsc::Receiver<Message>,
    chunks: bool,
    binary_events: bool,
    binary_requests: bool,
    mut receipts: mpsc::Receiver<(u32, u32)>,
    mut controls: mpsc::Receiver<Message>,
    usage: WriterUsage,
) -> &'static str
where
    S: Sink<Message> + Unpin,
{
    let capacity = outgoing.max_capacity();
    let mut pending = VecDeque::new();
    let mut active = VecDeque::<Transfer>::new();
    let mut credit = WireCredit::new();
    let mut next_id = 0u32;
    let mut small_streak = 0;
    let mut open = true;
    let mut receipts_open = true;
    let mut controls_open = true;
    loop {
        // A receiver too far behind is closed before anything else is written to it.
        if usage.queue.backlog_closed() {
            let _ = write(&mut writer, backlog_close_frame()).await;
            return "receiver_backlogged";
        }
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
                Ok(message) => {
                    let admitted = message.len();
                    match receiver_message(message, binary_events, binary_requests) {
                        Some(m) => pending.push_back(Queued::new(m, admitted, &usage)),
                        None => return "bad_outgoing_message",
                    }
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    open = false;
                    break;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
            }
        }
        if let Some(index) = pending.iter().position(|queued| control(&queued.message)) {
            let queued = pending.remove(index).unwrap();
            usage.released(&queued);
            let message = queued.message;
            let terminal = matches!(message, Message::Close(_));
            if !write(&mut writer, message).await || terminal {
                return "control_write";
            }
            continue;
        }
        if !chunks {
            if let Some(queued) = pending.pop_front() {
                usage.released(&queued);
                let message = queued.message;
                if message.len() > CHUNK_BYTES {
                    upgrade(&mut writer).await;
                    return "unchunked_oversize";
                }
                usage.sent(&queued.scope, &message);
                if !write(&mut writer, message).await {
                    return "write_failed";
                }
                continue;
            }
        } else if active.len() < MAX_TRANSFERS {
            let reserved: usize = active
                .iter()
                .map(|t| t.binary.as_ref().map_or(t.text.len(), Message::len))
                .sum();
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
                usage.released(&queued);
                let class = queued.lane;
                let key = queued.key.clone();
                let scope = queued.scope.clone();
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
                    scope,
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
                if let Some((message, end)) =
                    transfer.next_chunk(credit.drain.chunk_bytes(), binary_requests)
                {
                    let small_binary = transfer
                        .binary
                        .as_ref()
                        .is_some_and(|b| b.len() <= CHUNK_BYTES);
                    let bytes = message.len()
                        + if small_binary {
                            transfer.binary.as_ref().map_or(0, Message::len)
                        } else {
                            0
                        };
                    // While bulk is active, small traffic may hold only its reserve,
                    // so it can never keep the window above what a bulk chunk needs.
                    // The bulk window follows the measured drain rate, so bulk
                    // never queues more than about two round trips ahead of a small frame.
                    let window = credit.drain.bytes();
                    let fits = credit.pending.len() < PEER_RECEIPT_FRAMES
                        && if transfer.small {
                            credit.bytes + bytes <= window + SMALL_RESERVE_BYTES
                                && (active.iter().all(|t| t.small)
                                    || credit.small + bytes <= SMALL_RESERVE_BYTES)
                        } else {
                            credit.bytes == 0 || credit.bytes + bytes <= window
                        };
                    if !fits && !transfer.small {
                        credit.drain.blocked(Instant::now());
                    }
                    if fits {
                        if starting {
                            next_id = transfer.id;
                        }
                        credit.sent(transfer.id, end as u32, bytes, transfer.small);
                        small_streak = if transfer.small { small_streak + 1 } else { 0 };
                        transfer.offset = end;
                        transfer.waiting += 1;
                        let binary = if small_binary {
                            transfer.binary.take()
                        } else {
                            None
                        };
                        let scope = transfer.scope.clone();
                        active.push_back(transfer);
                        if let Some(binary) = binary {
                            usage.sent(&scope, &binary);
                            if !write(&mut writer, binary).await {
                                return "write_failed";
                            }
                        }
                        usage.sent(&scope, &message);
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
            () = usage.queue.close_requested() => {}
            message=controls.recv(),if controls_open=>{
                if let Some(message)=message {let terminal=matches!(message,Message::Close(_));if !write(&mut writer,message).await||terminal{return "control_write";}}else{controls_open=false;}
            }
            receipt=receipts.recv(),if receipts_open=>{
                let Some((id,offset))=receipt else{receipts_open=false;continue;};
                if !credit.receipt(id,offset,&mut active){return "bad_receipt";}
            }
            message=outgoing.recv(),if open && pending.len()<capacity=>{
                match message {Some(message)=>{let admitted=message.len();match receiver_message(message,binary_events,binary_requests){Some(m)=>pending.push_back(Queued::new(m,admitted,&usage)),None=>return "bad_outgoing_message"}},None=>open=false}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outgoing::RelaySender;
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            true,
            false,
            ack_rx,
            cr,
            WriterUsage::fixture(true),
        ));
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
                scope: CONTROL_SCOPE.into(),
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
                scope: CONTROL_SCOPE.into(),
                key: None,
                small: true,
                text: "y".into(),
                binary: None,
                offset: 1,
                lane: Lane::Event,
                waiting: 1,
            },
        ]);
        let mut credit = WireCredit::new();
        credit.sent(1, 1, 100, false);
        credit.sent(2, 1, 200, false);
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            true,
            false,
            ack_rx,
            cr,
            WriterUsage::fixture(true),
        ));
        let mut wire = 0;
        let mut ids = Vec::new();
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        while let Ok(Some(Ok(message))) =
            tokio::time::timeout(Duration::from_millis(150), client.next()).await
        {
            wire += message.len();
            assert!(
                wire <= START_WINDOW_BYTES + SMALL_RESERVE_BYTES,
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
            ids.len() < (START_WINDOW_BYTES + SMALL_RESERVE_BYTES) / 4096 + 2,
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            false,
            false,
            ack_rx,
            cr,
            WriterUsage::fixture(true),
        ));
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
    // S1: a large message waits one round trip per window. On a connection
    // whose window has measured the path, 480 KB (a mirror first packet) must
    // need at most two flights, not one per 64 KiB.
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
        let task = tokio::spawn(run_writer(
            sink,
            rx,
            true,
            true,
            false,
            ack_rx,
            cr,
            WriterUsage::fixture(true),
        ));
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let mut flights_for = async |id: &str, bytes: usize| {
            let reply = serde_json::json!({"kind":"client_response","request_id":id,"payload":"y".repeat(bytes)}).to_string();
            tx.send(Message::Text(reply.clone().into())).await.unwrap();
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
            assert_eq!(done, Some(reply));
            flights
        };
        flights_for("warm-up", 2_000_000).await;
        let flights = flights_for("mirror", 480_000).await;
        task.abort();
        assert!(flights <= 2, "480 KB needed {flights} round trips");
    }
    // MP-08 / MP-09 / MP-10 / MP-11 (relayflood): a receiver that cannot write holds at most
    // its bulk byte budget, however much is offered to it.
    #[tokio::test]
    async fn bulk_to_one_receiver_stays_within_its_byte_budget() {
        use crate::outgoing::{SendRefused, RECEIVER_BULK_BUDGET};
        const REPLY: usize = 350_000;
        let sink = Box::pin(futures_util::sink::unfold(
            (),
            |(), _: Message| async move {
                std::future::pending::<()>().await;
                Ok::<_, std::convert::Infallible>(())
            },
        ));
        let (_control, controls) = mpsc::channel(8);
        let (sender, outgoing) = RelaySender::channel(1024);
        let (_ack_tx, ack_rx) = mpsc::channel(16);
        let queue = sender.queue();
        let task = tokio::spawn(run_writer(
            sink,
            outgoing,
            true,
            true,
            false,
            ack_rx,
            controls,
            WriterUsage::fixture_with_queue(true, queue.clone()),
        ));
        let (mut admitted, mut refused) = (0, 0);
        for n in 0..100 {
            let reply = serde_json::json!({"kind":"client_response","request_id":format!("r{n}"),"payload":"y".repeat(REPLY)});
            match sender.try_send(Message::Text(reply.to_string().into())) {
                Ok(()) => admitted += 1,
                Err(SendRefused::Backlogged) => refused += 1,
                Err(SendRefused::Gone) => panic!("the writer ended"),
            }
            assert!(queue.queued_bytes() <= RECEIVER_BULK_BUDGET + REPLY + 100);
            tokio::task::yield_now().await;
        }
        task.abort();
        assert!(refused > 0, "all {admitted} replies were queued");
        assert!(queue.stats().queued_bytes_high_water <= RECEIVER_BULK_BUDGET + REPLY + 100);
    }
    // A message larger than the budget still reaches a receiver whose bulk queue is empty.
    #[tokio::test]
    async fn a_message_over_the_budget_passes_an_empty_queue() {
        use crate::outgoing::{SendRefused, RECEIVER_BULK_BUDGET};
        let (sender, mut receiver) = RelaySender::detached_channel(8);
        let big = || Message::Binary(vec![0; RECEIVER_BULK_BUDGET + 1].into());
        sender.try_send(big()).expect("an empty queue admits it");
        assert_eq!(sender.try_send(big()), Err(SendRefused::Backlogged));
        receiver.recv().await.unwrap();
        sender.try_send(big()).expect("the queue emptied");
    }
    // Control and small replies are written ahead of the bulk behind them.
    #[tokio::test]
    async fn control_is_not_delayed_by_a_bulk_backlog() {
        let (wire_tx, mut wire) = mpsc::unbounded_channel();
        let sink = Box::pin(futures_util::sink::unfold(
            wire_tx,
            |tx, message: Message| async move {
                let _ = tx.send(message);
                Ok::<_, std::convert::Infallible>(tx)
            },
        ));
        let (control, controls) = mpsc::channel(8);
        let (sender, outgoing) = RelaySender::channel(64);
        let (_ack_tx, ack_rx) = mpsc::channel(16);
        for n in 0..20 {
            let bulk = serde_json::json!({"kind":"client_response","request_id":format!("bulk-{n}"),"payload":"y".repeat(300_000)});
            sender
                .try_send(Message::Text(bulk.to_string().into()))
                .unwrap();
        }
        let small =
            serde_json::json!({"kind":"client_response","request_id":"small-reply","payload":"ok"});
        sender
            .try_send(Message::Text(small.to_string().into()))
            .unwrap();
        control.try_send(Message::Ping(Vec::new().into())).unwrap();
        let queue = sender.queue();
        let task = tokio::spawn(run_writer(
            sink,
            outgoing,
            true,
            true,
            false,
            ack_rx,
            controls,
            WriterUsage::fixture_with_queue(true, queue),
        ));
        let mut written = Vec::new();
        while let Ok(Some(message)) =
            tokio::time::timeout(Duration::from_millis(300), wire.recv()).await
        {
            written.push(message);
        }
        task.abort();
        assert!(
            matches!(written.first(), Some(Message::Ping(_))),
            "the ping goes first"
        );
        let small_at = written
            .iter()
            .position(|m| matches!(m, Message::Text(t) if t.contains("small-reply")))
            .expect("the small reply was written while bulk waited for credit");
        let bulk_frames_before = written[..small_at]
            .iter()
            .filter(|m| matches!(m, Message::Text(t) if t.contains("bulk-")))
            .count();
        assert!(
            bulk_frames_before <= 4,
            "{bulk_frames_before} bulk frames before the small reply"
        );
    }
    // MP-08 / MP-09 / MP-10 / MP-11 (relayflood): on a 256 KiB/s link carrying three bulk transfers
    // a small reply is delayed only by what the writer already put on the wire, never by the
    // bulk still queued behind the credit window.
    #[tokio::test]
    async fn a_small_reply_waits_only_for_the_window_on_a_slow_link() {
        const RATE: usize = 256 * 1024;
        let start = tokio::time::Instant::now();
        let (wire_tx, mut wire) = mpsc::unbounded_channel();
        let sink = Box::pin(futures_util::sink::unfold(
            wire_tx,
            |tx, message: Message| async move {
                tokio::time::sleep(Duration::from_secs_f64(message.len() as f64 / RATE as f64))
                    .await;
                let _ = tx.send((tokio::time::Instant::now(), message));
                Ok::<_, std::convert::Infallible>(tx)
            },
        ));
        let (_control, controls) = mpsc::channel(8);
        let (sender, outgoing) = RelaySender::channel(1024);
        let (ack_tx, ack_rx) = mpsc::channel(crate::frame_transport::MAX_UNACKED_FRAMES);
        let queue = sender.queue();
        let task = tokio::spawn(run_writer(
            sink,
            outgoing,
            true,
            true,
            false,
            ack_rx,
            controls,
            WriterUsage::fixture_with_queue(false, queue),
        ));
        let bulk = |kind: &str, n: usize| {
            let message = serde_json::json!({"kind":kind,"request_id":format!("bulk-{n}"),"payload":"y".repeat(20 * 1024 * 1024)});
            Message::Text(message.to_string().into())
        };
        for (n, kind) in ["client_response", "daemon_request", "client_event"]
            .into_iter()
            .enumerate()
        {
            while sender.try_send(bulk(kind, n)).is_err() {
                tokio::task::yield_now().await;
            }
        }
        // The peer reads each frame, then returns its receipt.
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let mut sent_small_at = None;
        let mut small_delay = None;
        while small_delay.is_none() {
            let (arrived, message) = wire.recv().await.expect("the writer ended");
            let Message::Text(text) = message else {
                continue;
            };
            let (_, receipt) = receiver.receive(&text).unwrap();
            if let Some(receipt) = receipt {
                let TransportFrame::TransportAck {
                    transfer_id,
                    offset,
                } = serde_json::from_str(&receipt).unwrap()
                else {
                    panic!("receipt expected")
                };
                ack_tx.send((transfer_id, offset)).await.unwrap();
            }
            if sent_small_at.is_none() && arrived.duration_since(start) > Duration::from_secs(2) {
                let small = serde_json::json!({"kind":"client_response","request_id":"small-reply","payload":"ok"});
                sender
                    .try_send(Message::Text(small.to_string().into()))
                    .unwrap();
                sent_small_at = Some(tokio::time::Instant::now());
            }
            if text.contains("small-reply") {
                small_delay = Some(arrived.duration_since(sent_small_at.unwrap()));
            }
        }
        task.abort();
        let delay = small_delay.unwrap();
        let bound = Duration::from_secs_f64(
            (START_WINDOW_BYTES + SMALL_RESERVE_BYTES) as f64 / RATE as f64 + 0.5,
        );
        assert!(
            delay <= bound,
            "the small reply waited {delay:?}, bound {bound:?}"
        );
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relayproof r3): on a 1 Mbit/s link with a 60 ms round
    // trip, a small reply queued behind bulk waits for what the window keeps in the
    // bottleneck queue. Frames sized to the measured rate (about 25 ms each, at least 4 KiB)
    // and a floor of two such frames keep that near one frame; 16 KiB frames and a 32 KiB
    // floor kept about 200 ms queued (the bar is RTT + 100 ms).
    #[tokio::test(start_paused = true)]
    async fn a_small_reply_waits_one_short_frame_on_a_1_mbit_link() {
        const RATE: f64 = 125_000.0;
        const ONE_WAY: Duration = Duration::from_millis(30);
        // The socket takes every frame at once; the bottleneck sends them in order at RATE.
        let (wire_tx, mut wire_rx) = mpsc::unbounded_channel::<Message>();
        let sink = Box::pin(futures_util::sink::unfold(
            wire_tx,
            |tx, message: Message| async move {
                let _ = tx.send(message);
                Ok::<_, std::convert::Infallible>(tx)
            },
        ));
        let (arrived_tx, mut arrived) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut free_at = tokio::time::Instant::now();
            while let Some(message) = wire_rx.recv().await {
                free_at = free_at.max(tokio::time::Instant::now())
                    + Duration::from_secs_f64(message.len() as f64 / RATE);
                tokio::time::sleep_until(free_at).await;
                let arrived_tx = arrived_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(ONE_WAY).await;
                    let _ = arrived_tx.send((tokio::time::Instant::now(), message));
                });
            }
        });
        let (_control, controls) = mpsc::channel(8);
        let (sender, outgoing) = RelaySender::channel(1024);
        let (ack_tx, ack_rx) = mpsc::channel(crate::frame_transport::MAX_UNACKED_FRAMES);
        let task = tokio::spawn(run_writer(
            sink,
            outgoing,
            true,
            true,
            false,
            ack_rx,
            controls,
            WriterUsage::fixture_with_queue(false, sender.queue()),
        ));
        let bulk = serde_json::json!({"kind":"client_response","request_id":"bulk","payload":"y".repeat(2 * 1024 * 1024)});
        sender
            .try_send(Message::Text(bulk.to_string().into()))
            .unwrap();
        let start = tokio::time::Instant::now();
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        // Eight small replies, 300 ms apart once the window has measured the path.
        let mut next_small = start + Duration::from_secs(3);
        let (mut sent_small_at, mut delays) = (None, Vec::new());
        while delays.len() < 8 {
            let (at, message) = arrived.recv().await.expect("the writer ended");
            let Message::Text(text) = message else {
                continue;
            };
            if text.contains("small-reply") {
                delays.push(at.duration_since(sent_small_at.take().unwrap()));
            }
            let (_, receipt) = receiver.receive(&text).unwrap();
            if let Some(receipt) = receipt {
                let TransportFrame::TransportAck {
                    transfer_id,
                    offset,
                } = serde_json::from_str(&receipt).unwrap()
                else {
                    panic!("receipt expected")
                };
                let ack_tx = ack_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(ONE_WAY).await;
                    let _ = ack_tx.send((transfer_id, offset)).await;
                });
            }
            if sent_small_at.is_none() && at >= next_small {
                let small = serde_json::json!({"kind":"client_response","request_id":"small-reply","payload":"ok"});
                sender
                    .try_send(Message::Text(small.to_string().into()))
                    .unwrap();
                sent_small_at = Some(tokio::time::Instant::now());
                next_small = at + Duration::from_millis(300);
            }
        }
        task.abort();
        // One way (30 ms), the frame on the wire and what the window keeps queued.
        let bound = Duration::from_millis(30 + 100);
        let worst = delays.iter().max().unwrap();
        assert!(
            *worst <= bound,
            "the small replies waited {delays:?}, bound {bound:?}"
        );
    }
    // The same bound for the binary frames every bulk answer travels in now: toward a peer that
    // offered binary requests the chunks follow the measured link like text frames do, so a
    // small reply waits one short chunk and not a full 16 KiB (131 ms at 1 Mbit/s).
    #[tokio::test(start_paused = true)]
    async fn a_small_reply_waits_one_short_chunk_behind_a_binary_answer_on_a_1_mbit_link() {
        const RATE: f64 = 125_000.0;
        const ONE_WAY: Duration = Duration::from_millis(30);
        let (wire_tx, mut wire_rx) = mpsc::unbounded_channel::<Message>();
        let sink = Box::pin(futures_util::sink::unfold(
            wire_tx,
            |tx, message: Message| async move {
                let _ = tx.send(message);
                Ok::<_, std::convert::Infallible>(tx)
            },
        ));
        let (arrived_tx, mut arrived) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut free_at = tokio::time::Instant::now();
            while let Some(message) = wire_rx.recv().await {
                free_at = free_at.max(tokio::time::Instant::now())
                    + Duration::from_secs_f64(message.len() as f64 / RATE);
                tokio::time::sleep_until(free_at).await;
                let arrived_tx = arrived_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(ONE_WAY).await;
                    let _ = arrived_tx.send((tokio::time::Instant::now(), message));
                });
            }
        });
        let (_control, controls) = mpsc::channel(8);
        let (sender, outgoing) = RelaySender::channel(1024);
        let (ack_tx, ack_rx) = mpsc::channel(crate::frame_transport::MAX_UNACKED_FRAMES);
        let task = tokio::spawn(run_writer(
            sink,
            outgoing,
            true,
            true,
            true,
            ack_rx,
            controls,
            WriterUsage::fixture_with_queue(false, sender.queue()),
        ));
        let answer = |request_id: &str, ciphertext: &[u8]| {
            let header = crate::binary_response::ResponseHeader::ClientResponse {
                request_id: request_id.into(),
                sender_public_key: "opaque-key".into(),
                nonce: "abcdefghijklmnop".into(),
            };
            Message::Binary(
                crate::binary_response::encode(&header, ciphertext)
                    .unwrap()
                    .into(),
            )
        };
        sender
            .try_send(answer("bulk", &vec![7; 2 * 1024 * 1024]))
            .unwrap();
        let start = tokio::time::Instant::now();
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let mut next_small = start + Duration::from_secs(3);
        let (mut sent_small_at, mut delays, mut shortest_chunk) = (None, Vec::new(), usize::MAX);
        let mut sizes = Vec::new();
        while delays.len() < 8 {
            let (at, message) = arrived.recv().await.expect("the writer ended");
            let receipt = match &message {
                Message::Text(text) => receiver.receive(text).unwrap().1,
                Message::Binary(bytes) if bytes.starts_with(b"CXB1") => {
                    if bytes.len() > 16 + 64 {
                        shortest_chunk = shortest_chunk.min(bytes.len());
                    }
                    sizes.push(bytes.len());
                    receiver.receive_binary(bytes).unwrap().1
                }
                Message::Binary(bytes) => {
                    if let Ok((
                        crate::binary_response::ResponseHeader::ClientResponse {
                            request_id, ..
                        },
                        _,
                    )) = crate::binary_response::decode(bytes)
                    {
                        if request_id == "small-reply" {
                            delays.push(at.duration_since(sent_small_at.take().unwrap()));
                        }
                    }
                    None
                }
                _ => None,
            };
            if let Some(receipt) = receipt {
                let TransportFrame::TransportAck {
                    transfer_id,
                    offset,
                } = serde_json::from_str(&receipt).unwrap()
                else {
                    panic!("receipt expected")
                };
                let ack_tx = ack_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(ONE_WAY).await;
                    let _ = ack_tx.send((transfer_id, offset)).await;
                });
            }
            if sent_small_at.is_none() && at >= next_small {
                sender.try_send(answer("small-reply", &[1; 64])).unwrap();
                sent_small_at = Some(tokio::time::Instant::now());
                next_small = at + Duration::from_millis(300);
            }
        }
        task.abort();
        assert!(
            shortest_chunk < CHUNK_BYTES,
            "the chunks follow the link: shortest {shortest_chunk} bytes"
        );
        let bound = Duration::from_millis(30 + 100);
        let worst = delays.iter().max().unwrap();
        assert!(
            *worst <= bound,
            "the small replies waited {delays:?}, bound {bound:?}; last chunks {:?}",
            &sizes[sizes.len().saturating_sub(12)..]
        );
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            true,
            false,
            ack_rx,
            cr,
            WriterUsage::fixture(true),
        ));
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
    // A fresh Room attach must fit before the first receipt returns.
    #[tokio::test]
    async fn cold_room_reply_finishes_before_its_first_receipt() {
        let (socket, mut client) = sockets().await;
        let (tx, rx) = mpsc::channel(8);
        let (_ack_tx, ack_rx) = mpsc::channel(32);
        let (_ct, cr) = mpsc::channel(8);
        let reply = serde_json::json!({"kind": "client_response", "request_id": "cold-room", "payload": "x".repeat(60 * 1024)}).to_string();
        tx.send(Message::Text(reply.clone().into())).await.unwrap();
        drop(tx);
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            true,
            false,
            ack_rx,
            cr,
            WriterUsage::fixture(true),
        ));
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        let completed = tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(Ok(Message::Text(text))) = client.next().await {
                if let (Some(complete), _) = receiver.receive(&text).unwrap() {
                    return Some(complete);
                }
            }
            None
        })
        .await;
        task.abort();
        let _ = task.await;
        assert_eq!(
            completed.ok().flatten().as_deref(),
            Some(reply.as_str()),
            "cold reply waited for a receipt"
        );
    }

    #[test]
    fn mp08_mp10_mp11_retired_writes_keep_exact_ordered_byte_credit() {
        let mut active = VecDeque::new();
        let mut credit = WireCredit::new();
        credit.sent(1, 10, 100, false);
        credit.sent(2, 0, 200, false);
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            true,
            false,
            ar,
            cr,
            WriterUsage::fixture(true),
        ));
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
            assert!(total <= START_WINDOW_BYTES);
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            false,
            false,
            false,
            ar,
            cr,
            WriterUsage::fixture(false),
        ));
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            false,
            false,
            ar,
            cr,
            WriterUsage::fixture(false),
        ));
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            false,
            false,
            ar,
            cr,
            WriterUsage::fixture(true),
        ));
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            false,
            false,
            ar,
            cr,
            WriterUsage::fixture(true),
        ));
        let mut wire = 0;
        while let Ok(Some(Ok(message))) =
            tokio::time::timeout(Duration::from_millis(150), client.next()).await
        {
            wire += message.len();
        }
        assert!(
            wire >= START_WINDOW_BYTES - CHUNK_BYTES,
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            false,
            false,
            ar,
            cr,
            WriterUsage::fixture(true),
        ));
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
        let task = tokio::spawn(run_writer(
            socket,
            rx,
            true,
            false,
            false,
            ar,
            cr,
            WriterUsage::fixture(true),
        ));
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
    #[test]
    fn mp08_mp09_mp10_mp11_large_negotiated_events_stay_binary() {
        let header = crate::binary_event::EventHeader {
            kind: "client_event".into(),
            subscription_id: "display".into(),
            event_id: 1,
            sender_public_key: "opaque".into(),
            nonce: "abcdefghijklmnop".into(),
        };
        let binary = crate::binary_event::encode(&header, &[7; 100 * 1024]).unwrap();
        assert!(
            matches!(
                receiver_message(Message::Binary(binary.into()), true, false),
                Some(Message::Binary(_))
            ),
            "negotiated binary events must not add a base64 JSON envelope above the chunk boundary"
        );
    }

    #[tokio::test]
    async fn mp08_mp09_mp10_mp11_large_binary_wire_chunks_preserve_ciphertext() {
        let header = crate::binary_event::EventHeader {
            kind: "client_event".into(),
            subscription_id: "display".into(),
            event_id: 1,
            sender_public_key: "opaque".into(),
            nonce: "abcdefghijklmnop".into(),
        };
        let event = crate::binary_event::encode(&header, &[0xfe; 100 * 1024]).unwrap();
        let response = crate::binary_response::encode(
            &crate::binary_response::ResponseHeader::ClientResponse {
                request_id: "mirror".into(),
                sender_public_key: "opaque".into(),
                nonce: "abcdefghijklmnop".into(),
            },
            &[0xfe; 100 * 1024],
        )
        .unwrap();
        // Both opaque kinds use the same bounded, receipt-driven wire path.
        for original in [event, response] {
            let (socket, mut client) = sockets().await;
            let (tx, rx) = mpsc::channel(8);
            let (at, ar) = mpsc::channel(64);
            let (_ct, cr) = mpsc::channel(8);
            tx.send(Message::Binary(original.clone().into()))
                .await
                .unwrap();
            drop(tx);
            let task = tokio::spawn(run_writer(
                socket,
                rx,
                true,
                true,
                false,
                ar,
                cr,
                WriterUsage::fixture(true),
            ));
            let mut receiver = crate::frame_transport::FrameReceiver::default();
            let mut bytes = 0;
            let mut chunks = 0;
            loop {
                let message = tokio::time::timeout(Duration::from_secs(2), client.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                let Message::Binary(wire) = message else {
                    panic!("opaque ciphertext must not become a JSON envelope");
                };
                assert!(wire.len() <= CHUNK_BYTES);
                bytes += wire.len();
                chunks += 1;
                let (complete, ack) = receiver.receive_binary(&wire).unwrap();
                let TransportFrame::TransportAck {
                    transfer_id,
                    offset,
                } = serde_json::from_str(&ack.unwrap()).unwrap()
                else {
                    panic!("receipt");
                };
                at.send((transfer_id, offset)).await.unwrap();
                if let Some(complete) = complete {
                    assert_eq!(complete, original);
                    break;
                }
            }
            assert!(chunks > 1);
            assert_eq!(
                bytes,
                original.len() + chunks * crate::frame_transport::BINARY_CHUNK_HEADER_BYTES
            );
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), task)
                    .await
                    .unwrap()
                    .unwrap(),
                "drained"
            );
        }
    }

    // MP-08 / MP-09 / MP-10 / MP-11 (relayproof r4): the eight-entry control lane carries the
    // peer's upload receipts, so a slow bidirectional receiver can fill it while the writer waits
    // on the socket. A backlogged receiver must still be closed then, and closed by the writer.
    #[tokio::test]
    async fn a_backlogged_receiver_is_closed_while_the_control_lane_is_full() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let (wire_tx, mut wire) = mpsc::unbounded_channel();
        let sink = Box::pin(futures_util::sink::unfold(
            (wire_tx, gate.clone()),
            |(tx, gate), message: Message| async move {
                gate.acquire().await.unwrap().forget();
                let _ = tx.send(message);
                Ok::<_, std::convert::Infallible>((tx, gate))
            },
        ));
        let (control, controls) = mpsc::channel(8);
        let (sender, outgoing) = RelaySender::channel(64);
        let (_ack_tx, ack_rx) = mpsc::channel(16);
        let queue = sender.queue();
        let small = serde_json::json!({"kind":"client_response","request_id":"r","payload":"ok"});
        sender
            .try_send(Message::Text(small.to_string().into()))
            .unwrap();
        let task = tokio::spawn(run_writer(
            sink,
            outgoing,
            true,
            true,
            false,
            ack_rx,
            controls,
            WriterUsage::fixture_with_queue(true, queue),
        ));
        // The writer is stuck in the socket write of the first message.
        tokio::time::sleep(Duration::from_millis(50)).await;
        for _ in 0..8 {
            control.try_send(Message::Ping(Vec::new().into())).unwrap();
        }
        assert!(sender.close_backlogged());
        gate.add_permits(1_000);
        let mut closed = false;
        while let Ok(Some(message)) =
            tokio::time::timeout(Duration::from_millis(500), wire.recv()).await
        {
            closed |=
                matches!(message, Message::Close(Some(frame)) if u16::from(frame.code) == 1013);
        }
        let ended = tokio::time::timeout(Duration::from_millis(500), task).await;
        assert!(closed, "the receiver was never told to try again later");
        assert!(
            ended.is_ok(),
            "the writer kept running after the close was due"
        );
    }
}

#[cfg(test)]
#[path = "slow_link_tests.rs"]
mod slow_link_tests;

#[cfg(test)]
mod binary_reply_regression {
    use super::*;
    #[test]
    fn mp08_mp09_mp10_mp11_successful_large_replies_keep_raw_ciphertext() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let text = serde_json::to_string(&crate::protocol::RelayEnvelope::ClientResponse {
            request_id: "mirror-request".into(),
            encrypted_response: Some(crate::protocol::EncryptedRelayPayload {
                sender_public_key: "opaque-key".into(),
                nonce: "abcdefghijklmnop".into(),
                ciphertext: STANDARD.encode(vec![0xfe; 100 * 1024]),
            }),
            error: None,
        })
        .unwrap();
        let binary = receiver_message(Message::Text(text.into()), true, false).unwrap();
        assert!(
            matches!(binary, Message::Binary(_)),
            "negotiated successful replies must not base64 ciphertext"
        );
        assert!(
            matches!(
                lane(&binary, &WriterUsage::fixture(true)),
                (Lane::Reply, None, None)
            ),
            "response correlation keeps the reply lane"
        );
    }
}
