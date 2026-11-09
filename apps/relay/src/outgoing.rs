//! A connection's outgoing queue as the connections that feed it see it.
//!
//! Every connection has one writer that drains one queue. The queue is bounded by message
//! count and, for bulk messages, by bytes, so one slow receiver holds at most a fixed amount
//! of relay memory however much the other side sends. A sender whose message is refused
//! learns it from the result; it never has to give up its own connection for it.
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

use crate::frame_transport::CHUNK_BYTES;

/// Bulk bytes one receiver may have queued: about one second at 64 Mbit/s, enough to keep a
/// fast client's pipe full. A single message larger than this still passes an empty bulk queue.
pub(crate) const RECEIVER_BULK_BUDGET: usize = 8 * 1024 * 1024;

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

    /// A message of `len` bytes left the queue: it was written, or became a transfer.
    pub(crate) fn release(&self, len: usize) {
        self.messages.fetch_sub(1, Ordering::Relaxed);
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

    pub(crate) fn stats(&self) -> QueueStats {
        QueueStats {
            queued_bytes_high_water: self.high_water.load(Ordering::Relaxed),
            dropped_replies: self.dropped_replies.load(Ordering::Relaxed),
            backlog_closed: self.backlog_closed(),
        }
    }
}

/// The sending half of one connection's queue plus the means to close that connection.
#[derive(Debug, Clone)]
pub(crate) struct RelaySender {
    tx: mpsc::Sender<Message>,
    control: mpsc::Sender<Message>,
    queue: Arc<OutgoingQueue>,
}

/// A sender that does not keep its connection's queue open.
#[derive(Debug, Clone)]
pub(crate) struct WeakRelaySender {
    tx: mpsc::WeakSender<Message>,
    control: mpsc::WeakSender<Message>,
    queue: Arc<OutgoingQueue>,
}

impl RelaySender {
    /// A queue of at most `max_messages` messages, and the receiver its writer drains.
    /// `control` is the writer's control lane, which `close_backlogged` uses.
    pub(crate) fn channel(
        max_messages: usize,
        control: mpsc::Sender<Message>,
    ) -> (Self, mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel(max_messages);
        (
            Self {
                tx,
                control,
                queue: Arc::new(OutgoingQueue::new(max_messages)),
            },
            rx,
        )
    }

    /// A sender whose connection nobody closes, for tests that read its queue directly.
    #[cfg(test)]
    pub(crate) fn detached(tx: mpsc::Sender<Message>) -> Self {
        let (control, _closed) = mpsc::channel(1);
        Self {
            queue: Arc::new(OutgoingQueue::new(tx.max_capacity())),
            tx,
            control,
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
        if self.tx.is_closed() {
            return Err(SendRefused::Gone);
        }
        let len = message.len();
        self.queue.admit(len)?;
        self.tx.try_send(message).map_err(|error| {
            self.queue.release(len);
            match error {
                mpsc::error::TrySendError::Full(_) => SendRefused::Backlogged,
                mpsc::error::TrySendError::Closed(_) => SendRefused::Gone,
            }
        })
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
            control: self.control.downgrade(),
            queue: self.queue.clone(),
        }
    }

    pub(crate) fn record_dropped_reply(&self) {
        self.queue.dropped_replies.fetch_add(1, Ordering::Relaxed);
    }

    /// Asks the receiver's connection to end with "try again later". Once is enough; the
    /// writer sends the close ahead of everything still queued. True the first time.
    pub(crate) fn close_backlogged(&self) -> bool {
        if self.queue.backlog_closed.swap(true, Ordering::Relaxed) {
            return false;
        }
        let _ = self.control.try_send(Message::Close(Some(CloseFrame {
            code: CloseCode::Again,
            reason: "try again later".into(),
        })));
        true
    }
}

impl WeakRelaySender {
    pub(crate) fn upgrade(&self) -> Option<RelaySender> {
        Some(RelaySender {
            tx: self.tx.upgrade()?,
            control: self.control.upgrade()?,
            queue: self.queue.clone(),
        })
    }
}

/// What stands in for a writer in tests: a message leaves the queue when it is received.
#[cfg(test)]
pub(crate) struct DetachedReceiver {
    rx: mpsc::Receiver<Message>,
    queue: Arc<OutgoingQueue>,
}

#[cfg(test)]
impl DetachedReceiver {
    pub(crate) async fn recv(&mut self) -> Option<Message> {
        let message = self.rx.recv().await?;
        self.queue.release(message.len());
        Some(message)
    }

    pub(crate) fn try_recv(&mut self) -> Result<Message, mpsc::error::TryRecvError> {
        let message = self.rx.try_recv()?;
        self.queue.release(message.len());
        Ok(message)
    }
}
