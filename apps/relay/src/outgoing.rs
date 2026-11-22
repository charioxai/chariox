//! A connection's outgoing queue as the connections that feed it see it.
//!
//! Every connection has one writer that drains one queue. The queue is bounded by message
//! count and, for bulk messages, by bytes, so one slow receiver holds at most a fixed amount
//! of relay memory however much the other side sends. A sender whose message is refused
//! learns it from the result; it never has to give up its own connection for it.
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

use crate::frame_transport::{peer_reply_block_size, peer_reply_chunk, CHUNK_BYTES};
use crate::packed_peer_reply::{PackedPeerReply, PackedReplyPlan};

/// Bulk bytes one receiver may have queued: about one second at 64 Mbit/s, enough to keep a
/// fast client's pipe full. A single message larger than this still passes an empty bulk queue.
pub(crate) const RECEIVER_BULK_BUDGET: usize = 8 * 1024 * 1024;

pub(crate) enum Outgoing {
    Frame(Message),
    PeerReply(PeerReply),
}

#[cfg(test)]
impl From<Message> for Outgoing {
    fn from(message: Message) -> Self {
        Self::Frame(message)
    }
}

impl Outgoing {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Frame(message) => message.len(),
            Self::PeerReply(reply) => reply.len(),
        }
    }

    pub(crate) fn admitted_len(&self) -> usize {
        match self {
            Self::PeerReply(PeerReply::Packed(reply)) => reply.charge(),
            _ => self.len(),
        }
    }

    #[cfg(test)]
    fn into_test_frame(self) -> Message {
        match self {
            Self::Frame(message) => message,
            Self::PeerReply(PeerReply::Text(reply)) => {
                Message::Text(reply.blocks.into_iter().collect::<String>().into())
            }
            Self::PeerReply(PeerReply::Packed(reply)) => {
                Message::Text(reply.into_test_text().into())
            }
        }
    }
}

pub(crate) enum PeerReply {
    Text(PeerReplyText),
    Packed(PackedPeerReply),
}

impl PeerReply {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Text(reply) => reply.len,
            Self::Packed(reply) => reply.len(),
        }
    }

    pub(crate) fn next_chunk(
        &self,
        id: u32,
        offset: usize,
        limit: usize,
    ) -> Option<(String, usize)> {
        match self {
            Self::Text(reply) => {
                let block = reply.blocks.front()?;
                peer_reply_chunk(
                    id,
                    &block[offset - reply.written..],
                    offset,
                    reply.len,
                    limit,
                )
            }
            Self::Packed(reply) => reply.next_chunk(id, offset, limit),
        }
    }

    pub(crate) fn written(&mut self, end: usize) -> Option<usize> {
        match self {
            Self::Text(reply) => reply.written(end),
            Self::Packed(reply) => reply.written(end),
        }
    }
}

/// Serialize into independently owned pieces so writing one does not keep
/// every earlier byte alive until the entire reply has been written.
#[derive(Default)]
pub(crate) struct PeerReplyText {
    blocks: VecDeque<Box<str>>,
    prefix: String,
    len: usize,
    written: usize,
}

impl PeerReplyText {
    fn serialize(value: &impl serde::Serialize) -> Self {
        let mut reply = Self::default();
        serde_json::to_writer(&mut reply, value).expect("a relay envelope is always valid JSON");
        reply.finish_block();
        reply
    }

    fn written(&mut self, end: usize) -> Option<usize> {
        let len = self.blocks.front()?.len();
        if end != self.written + len {
            return None;
        }
        drop(self.blocks.pop_front());
        self.written = end;
        Some(len)
    }

    fn finish_block(&mut self) {
        if !self.prefix.is_empty() {
            self.blocks
                .push_back(std::mem::take(&mut self.prefix).into_boxed_str());
        }
    }
}

impl Write for PeerReplyText {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
        let block_size = peer_reply_block_size();
        while !text.is_empty() {
            let mut end = text.len().min(block_size - self.prefix.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                self.finish_block();
                continue;
            }
            if self.prefix.is_empty() {
                self.prefix.reserve_exact(block_size);
            }
            self.prefix.push_str(&text[..end]);
            text = &text[end..];
        }
        self.len += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Why a message was not queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendRefused {
    /// The receiver is too far behind: its message or byte budget is spent.
    Backlogged,
    /// The receiver's connection has ended.
    Gone,
}

impl std::fmt::Display for SendRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Backlogged => "receiver queue is full",
            Self::Gone => "receiver connection is closed",
        })
    }
}

/// What one connection's queue went through, for its `relay_peer_removed` line and the health counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct QueueStats {
    pub(crate) queued_bytes_high_water: usize,
    pub(crate) dropped_replies: u64,
    pub(crate) backlog_closed: bool,
}

#[derive(Debug)]
pub(crate) struct OutgoingQueue {
    max_messages: usize,
    messages: AtomicUsize,
    bytes: AtomicUsize,
    bulk_bytes: AtomicUsize,
    high_water: AtomicUsize,
    dropped_replies: AtomicU64,
    backlog_closed: AtomicBool,
    /// Wakes the writer when the receiver is to be closed; the permit is kept, so a request
    /// made while the writer is busy with the socket is not lost.
    close_requested: Notify,
}

impl OutgoingQueue {
    fn new(max_messages: usize) -> Self {
        Self {
            max_messages,
            messages: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
            bulk_bytes: AtomicUsize::new(0),
            high_water: AtomicUsize::new(0),
            dropped_replies: AtomicU64::new(0),
            backlog_closed: AtomicBool::new(false),
            close_requested: Notify::new(),
        }
    }

    fn admit(&self, len: usize) -> Result<(), SendRefused> {
        if self.messages.fetch_add(1, Ordering::Relaxed) >= self.max_messages {
            self.messages.fetch_sub(1, Ordering::Relaxed);
            return Err(SendRefused::Backlogged);
        }
        if len > CHUNK_BYTES {
            let queued = self.bulk_bytes.fetch_add(len, Ordering::Relaxed);
            if queued != 0 && queued + len > RECEIVER_BULK_BUDGET {
                self.bulk_bytes.fetch_sub(len, Ordering::Relaxed);
                self.messages.fetch_sub(1, Ordering::Relaxed);
                return Err(SendRefused::Backlogged);
            }
        }
        let queued = self.bytes.fetch_add(len, Ordering::Relaxed) + len;
        self.high_water.fetch_max(queued, Ordering::Relaxed);
        Ok(())
    }

    /// A message of `len` bytes was written or could not enter the writer.
    pub(crate) fn release(&self, len: usize) {
        self.messages.fetch_sub(1, Ordering::Relaxed);
        self.release_bytes(len);
    }

    /// The writer took the message slot. WireCredit still owns the byte
    /// reservation until the writer receives the corresponding exact receipt.
    pub(crate) fn start_transfer(self: &Arc<Self>, len: usize) -> Option<BulkCharge> {
        self.messages.fetch_sub(1, Ordering::Relaxed);
        if len > CHUNK_BYTES {
            Some(BulkCharge {
                queue: Arc::clone(self),
                bytes: len,
            })
        } else {
            self.release_bytes(len);
            None
        }
    }

    fn release_bytes(&self, len: usize) {
        self.bytes.fetch_sub(len, Ordering::Relaxed);
        if len > CHUNK_BYTES {
            self.bulk_bytes.fetch_sub(len, Ordering::Relaxed);
        }
    }

    pub(crate) fn queued_messages(&self) -> usize {
        self.messages.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn queued_bytes(&self) -> usize {
        self.bytes.load(Ordering::Relaxed)
    }

    pub(crate) fn backlog_closed(&self) -> bool {
        self.backlog_closed.load(Ordering::Relaxed)
    }

    /// Resolves once `close_backlogged` has asked for the receiver's connection to end.
    pub(crate) async fn close_requested(&self) {
        self.close_requested.notified().await;
    }

    pub(crate) fn stats(&self) -> QueueStats {
        QueueStats {
            queued_bytes_high_water: self.high_water.load(Ordering::Relaxed),
            dropped_replies: self.dropped_replies.load(Ordering::Relaxed),
            backlog_closed: self.backlog_closed(),
        }
    }
}

pub(crate) struct BulkCharge {
    queue: Arc<OutgoingQueue>,
    bytes: usize,
}

impl Drop for BulkCharge {
    fn drop(&mut self) {
        self.queue.bytes.fetch_sub(self.bytes, Ordering::Relaxed);
        self.queue
            .bulk_bytes
            .fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

impl BulkCharge {
    pub(crate) fn split(&mut self, bytes: usize) -> Self {
        self.bytes -= bytes;
        Self {
            queue: Arc::clone(&self.queue),
            bytes,
        }
    }
}

/// The sending half of one connection's queue plus the means to close that connection.
#[derive(Debug, Clone)]
pub(crate) struct RelaySender {
    tx: mpsc::Sender<Outgoing>,
    queue: Arc<OutgoingQueue>,
}

/// A sender that does not keep its connection's queue open.
#[derive(Debug, Clone)]
pub(crate) struct WeakRelaySender {
    tx: mpsc::WeakSender<Outgoing>,
    queue: Arc<OutgoingQueue>,
}

impl RelaySender {
    /// A queue of at most `max_messages` messages, and the receiver its writer drains.
    pub(crate) fn channel(max_messages: usize) -> (Self, mpsc::Receiver<Outgoing>) {
        let (tx, rx) = mpsc::channel(max_messages);
        (
            Self {
                tx,
                queue: Arc::new(OutgoingQueue::new(max_messages)),
            },
            rx,
        )
    }

    /// A sender whose connection nobody closes, for tests that read its queue directly.
    #[cfg(test)]
    pub(crate) fn detached(tx: mpsc::Sender<Outgoing>) -> Self {
        Self {
            queue: Arc::new(OutgoingQueue::new(tx.max_capacity())),
            tx,
        }
    }

    /// `detached` with the receiver its test reads from.
    #[cfg(test)]
    pub(crate) fn detached_channel(max_messages: usize) -> (Self, DetachedReceiver) {
        let (tx, rx) = mpsc::channel(max_messages);
        let sender = Self::detached(tx);
        let queue = sender.queue();
        (sender, DetachedReceiver { rx, queue })
    }

    pub(crate) fn queue(&self) -> Arc<OutgoingQueue> {
        self.queue.clone()
    }

    pub(crate) fn try_send(&self, message: Message) -> Result<(), SendRefused> {
        self.try_send_sized(message.len(), || Outgoing::Frame(message))
    }

    /// An obviously oversized candidate can be refused before scanning its JSON.
    /// The real reservation is still made atomically after the full size is known.
    pub(crate) fn check_peer_reply_size(&self, ciphertext_bytes: usize) -> Result<(), SendRefused> {
        if self.tx.is_closed() {
            return Err(SendRefused::Gone);
        }
        let reserved = self.queue.bulk_bytes.load(Ordering::Relaxed);
        if reserved != 0 && ciphertext_bytes > RECEIVER_BULK_BUDGET.saturating_sub(reserved) {
            return Err(SendRefused::Backlogged);
        }
        Ok(())
    }

    /// Reserve capacity before allocating a serialized reply that might be refused.
    pub(crate) fn try_send_peer_reply(
        &self,
        len: usize,
        reply: &impl serde::Serialize,
    ) -> Result<(), SendRefused> {
        self.try_send_sized(len, || {
            Outgoing::PeerReply(PeerReply::Text(PeerReplyText::serialize(reply)))
        })
    }

    pub(crate) fn try_send_packed_peer_reply(
        &self,
        plan: PackedReplyPlan<'_>,
    ) -> Result<(), SendRefused> {
        self.try_send_sized(plan.charge(), || {
            Outgoing::PeerReply(PeerReply::Packed(plan.into_reply()))
        })
    }

    fn try_send_sized(
        &self,
        len: usize,
        message: impl FnOnce() -> Outgoing,
    ) -> Result<(), SendRefused> {
        if self.tx.is_closed() {
            return Err(SendRefused::Gone);
        }
        self.queue.admit(len)?;
        let permit = self.tx.try_reserve().map_err(|error| {
            self.queue.release(len);
            match error {
                mpsc::error::TrySendError::Full(_) => SendRefused::Backlogged,
                mpsc::error::TrySendError::Closed(_) => SendRefused::Gone,
            }
        })?;
        let message = message();
        debug_assert_eq!(message.admitted_len(), len);
        permit.send(message);
        Ok(())
    }

    pub(crate) fn same_channel(&self, other: &Self) -> bool {
        self.tx.same_channel(&other.tx)
    }

    pub(crate) fn queued_messages(&self) -> usize {
        self.queue.queued_messages()
    }

    pub(crate) fn downgrade(&self) -> WeakRelaySender {
        WeakRelaySender {
            tx: self.tx.downgrade(),
            queue: self.queue.clone(),
        }
    }

    pub(crate) fn record_dropped_reply(&self) {
        self.queue.dropped_replies.fetch_add(1, Ordering::Relaxed);
    }

    /// Asks the receiver's connection to end with "try again later". Once is enough; the
    /// writer sends the close ahead of everything still queued. True the first time. The
    /// request does not travel through a lane that could be full: the writer reads it from
    /// the queue.
    pub(crate) fn close_backlogged(&self) -> bool {
        if self.queue.backlog_closed.swap(true, Ordering::Relaxed) {
            return false;
        }
        self.queue.close_requested.notify_one();
        true
    }
}

impl WeakRelaySender {
    pub(crate) fn upgrade(&self) -> Option<RelaySender> {
        Some(RelaySender {
            tx: self.tx.upgrade()?,
            queue: self.queue.clone(),
        })
    }
}

/// The frame that ends a backlogged receiver's connection.
pub(crate) fn backlog_close_frame() -> Message {
    Message::Close(Some(CloseFrame {
        code: CloseCode::Again,
        reason: crate::protocol::REPLY_QUEUE_FULL_CLOSE_REASON.into(),
    }))
}

/// What stands in for a writer in tests: a message leaves the queue when it is received.
#[cfg(test)]
pub(crate) struct DetachedReceiver {
    rx: mpsc::Receiver<Outgoing>,
    queue: Arc<OutgoingQueue>,
}

#[cfg(test)]
impl DetachedReceiver {
    pub(crate) async fn recv(&mut self) -> Option<Message> {
        let message = self.rx.recv().await?;
        self.queue.release(message.admitted_len());
        Some(message.into_test_frame())
    }

    pub(crate) fn try_recv(&mut self) -> Result<Message, mpsc::error::TryRecvError> {
        let message = self.rx.try_recv()?;
        self.queue.release(message.admitted_len());
        Ok(message.into_test_frame())
    }
}
