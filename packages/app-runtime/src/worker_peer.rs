//! One bounded, bidirectional peer on a worker's inherited SDK channel.
//!
//! Kernel broker policy and readiness are injected; worker messages never create
//! caller authority. WorkerProcess retains the process, sandbox and FD lifecycle.

mod actor;
mod publication;
mod request;
mod transport;
mod validation;

use crate::wire::{Channel, Message, RemoteError};
pub use request::{CallResponse, RequestSlot};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot, watch, Semaphore},
    task::JoinHandle,
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PeerError {
    #[error("app_peer_closed")]
    Closed,
    #[error("app_peer_busy")]
    Busy,
    #[error("app_peer_invalid")]
    Invalid,
    #[error("app_peer_deadline")]
    Deadline,
    #[error("app_peer_protocol")]
    Protocol,
    #[error("app_peer_io")]
    Io,
    #[error("app_peer_broker_failed")]
    Broker,
}
pub type Result<T> = std::result::Result<T, PeerError>;

#[derive(Clone, Copy, Debug)]
pub struct PeerLimits {
    pub pending_calls: usize,
    pub broker_handlers: usize,
    pub queued_frames: usize,
    pub frame_timeout: Duration,
    pub max_deadline: Duration,
}
impl Default for PeerLimits {
    fn default() -> Self {
        Self {
            pending_calls: 64,
            broker_handlers: 16,
            queued_frames: 4,
            frame_timeout: Duration::from_secs(5),
            max_deadline: Duration::from_secs(30),
        }
    }
}
impl PeerLimits {
    fn validate(self) -> Result<Self> {
        if !(1..=64).contains(&self.pending_calls)
            || !(1..=16).contains(&self.broker_handlers)
            || !(1..=16).contains(&self.queued_frames)
            || self.frame_timeout.is_zero()
            || self.frame_timeout > Duration::from_secs(30)
            || self.max_deadline < Duration::from_millis(1)
            || self.max_deadline > Duration::from_secs(30)
        {
            return Err(PeerError::Invalid);
        }
        Ok(self)
    }
}

/// Cooperative cancellation is observable without dropping the broker future.
/// Its admission slot remains held until that future actually returns.
#[derive(Clone)]
pub struct BrokerCancellation(watch::Receiver<bool>);
impl BrokerCancellation {
    pub fn is_cancelled(&self) -> bool {
        // Losing the actor also withdraws authority to start queued work, even
        // when it could not publish an explicit cancellation before dropping.
        *self.0.borrow() || self.0.has_changed().is_err()
    }
    pub async fn cancelled(&mut self) {
        while !*self.0.borrow_and_update() {
            if self.0.changed().await.is_err() {
                return;
            }
        }
    }
}

pub struct BrokerRequest {
    pub id: String,
    pub method: String,
    pub params: serde_json::Value,
    pub deadline: Instant,
    pub cancellation: BrokerCancellation,
}

pub type BrokerFuture =
    Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, RemoteError>> + Send>>;
pub type BrokerDrainFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
/// Host-only completion ownership. Dropping an unpublished guard must release
/// or poison its exact operation; callbacks must be nonblocking and infallible.
pub trait ResponsePublication: Send + 'static {
    fn published(&mut self);
}
pub trait Broker: Send + Sync + 'static {
    /// One validated request, including worker.ready. No method is acknowledged
    /// automatically. The implementation retains kernel-owned identity/policy;
    /// request params carry no authenticated owner, agent, grant or generation.
    fn handle(&self, request: BrokerRequest) -> BrokerFuture;
    /// Transfers bounded completion ownership after the handler has finished.
    /// Apps cannot construct or select this guard through the wire protocol.
    fn take_response_guard(&self, _request_id: &str) -> Option<Box<dyn ResponsePublication>> {
        None
    }
    /// Host-only lifecycle notification. Must be nonblocking and idempotent;
    /// it cancels background work without releasing its resource reservations.
    fn begin_draining(&self) {}
    /// Joins background tasks after admitted request handlers have finished.
    /// The process owner retains its native/domain leases through this future.
    fn drain(&self) -> BrokerDrainFuture {
        Box::pin(async {})
    }
}

#[derive(Debug)]
pub struct ControlEvent {
    pub name: String,
    pub data: serde_json::Value,
}

pub struct PeerTask(JoinHandle<Result<()>>);
impl PeerTask {
    /// Stop the peer first. Completion waits for admitted broker callbacks to
    /// finish; an ignored cancellation does not prove an effect was cancelled.
    pub async fn join(self) -> Result<()> {
        self.0.await.map_err(|_| PeerError::Broker)?
    }
}

#[derive(Clone)]
pub struct WorkerPeer {
    control: Arc<Control>,
}
struct Control {
    generation: String,
    sequence: AtomicU64,
    slots: Arc<Semaphore>,
    commands: mpsc::Sender<Call>,
    stopped: watch::Sender<bool>,
    limits: PeerLimits,
}
impl Drop for Control {
    fn drop(&mut self) {
        self.stopped.send_replace(true);
    }
}

struct Call {
    message: Message,
    deadline: Instant,
    reply: oneshot::Sender<Result<Message>>,
    permit: tokio::sync::OwnedSemaphorePermit,
}

impl WorkerPeer {
    /// Call inside the kernel Tokio runtime, after channel/process admission.
    /// No App code, socket listener or ambient network connection is launched.
    pub fn start<T>(
        channel: Channel<T>,
        broker: Arc<dyn Broker>,
        limits: PeerLimits,
    ) -> Result<(Self, mpsc::Receiver<ControlEvent>, PeerTask)>
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let limits = limits.validate()?;
        let generation = channel.generation().to_string();
        let (commands, receiver) = mpsc::channel(limits.pending_calls);
        let (events, event_receiver) = mpsc::channel(limits.queued_frames);
        let (stopped, stop_receiver) = watch::channel(false);
        let peer = Self {
            control: Arc::new(Control {
                generation: generation.clone(),
                sequence: AtomicU64::new(0),
                slots: Arc::new(Semaphore::new(limits.pending_calls)),
                commands,
                stopped: stopped.clone(),
                limits,
            }),
        };
        let task = tokio::spawn(actor::run(
            channel,
            generation,
            broker,
            limits,
            receiver,
            events,
            stopped,
            stop_receiver,
        ));
        Ok((peer, event_receiver, PeerTask(task)))
    }

    /// Reserves bounded transport admission before the kernel prepares a call.
    /// IDs belong to this peer and are never accepted from a worker or client.
    pub fn reserve(&self, timeout: Duration) -> Result<RequestSlot> {
        if self.is_closed() {
            return Err(PeerError::Closed);
        }
        if timeout < Duration::from_millis(1) || timeout > self.control.limits.max_deadline {
            return Err(PeerError::Invalid);
        }
        let permit = self
            .control
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| PeerError::Busy)?;
        let sequence = self
            .control
            .sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |sequence| {
                sequence.checked_add(1)
            })
            .map_err(|_| PeerError::Closed)?;
        let deadline = Instant::now() + timeout;
        let deadline_ms = wall_ms()?
            .checked_add(timeout.as_millis() as u64)
            .filter(|value| *value <= 9_007_199_254_740_991)
            .ok_or(PeerError::Invalid)?;
        Ok(RequestSlot::new(
            self.clone(),
            format!("host-{}", sequence + 1),
            deadline_ms,
            deadline,
            permit,
        ))
    }
    pub fn is_closed(&self) -> bool {
        *self.control.stopped.borrow()
    }
    pub fn close(&self) {
        self.control.stopped.send_replace(true);
    }

    /// Observe actor/I/O shutdown as well as explicit owner cancellation. This
    /// only signals closure; PeerTask::join still drains admitted broker work.
    pub async fn closed(&self) {
        let mut stopped = self.control.stopped.subscribe();
        while !*stopped.borrow_and_update() {
            if stopped.changed().await.is_err() {
                return;
            }
        }
    }
}

fn wall_ms() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or(PeerError::Invalid)
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    #[tokio::test]
    async fn losing_last_sender_cancels_queued_work_without_a_value_change() {
        let (sender, receiver) = watch::channel(false);
        let other_sender = sender.clone();
        let mut cancellation = BrokerCancellation(receiver);
        drop(sender);
        assert!(!cancellation.is_cancelled());
        drop(other_sender);
        assert!(cancellation.is_cancelled());
        tokio::time::timeout(Duration::from_secs(1), cancellation.cancelled())
            .await
            .expect("closed cancellation must wake asynchronous waiters");
        assert!(cancellation.clone().is_cancelled());
    }

    #[tokio::test]
    async fn explicit_cancellation_survives_sender_close_and_async_observation() {
        let (sender, receiver) = watch::channel(false);
        let mut cancellation = BrokerCancellation(receiver);
        sender.send_replace(true);
        assert!(cancellation.is_cancelled());
        cancellation.cancelled().await;
        drop(sender);
        assert!(cancellation.is_cancelled());
        tokio::time::timeout(Duration::from_secs(1), cancellation.cancelled())
            .await
            .expect("observing cancellation must not clear it");
    }
}
