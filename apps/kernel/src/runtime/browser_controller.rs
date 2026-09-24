//! Exact-target CDP integration below the Room Environment authority.
//!
//! This module does not launch Chromium, select a tab, authorize an actor, or
//! expose an endpoint to a viewer. The managed execution owner supplies the
//! target and retains its process lease through the controller's drain.

mod actor;
mod protocol;
mod types;

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::Instant,
};
use tokio_tungstenite::{client_async_with_config, tungstenite::protocol::WebSocketConfig};
pub(super) use types::{
    BrowserError, BrowserInput, BrowserReference, BrowserSnapshot, BrowserTarget, ScreencastFrame,
};

const MAX_WIRE_BYTES: usize = 2 * 1024 * 1024;
const COMMAND_CAPACITY: usize = 8;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
static NEXT_CONTROLLER_EPOCH: AtomicU64 = AtomicU64::new(1);

/// Only trusted kernel execution code can construct this controller. Holding
/// the lease in the task prevents caller cancellation from releasing Chromium
/// resources before the WebSocket and outstanding request have been dropped.
pub(super) struct BrowserController {
    handle: BrowserControllerHandle,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<(), BrowserError>>>,
}

#[derive(Clone)]
pub(super) struct BrowserControllerHandle {
    target: BrowserTarget,
    epoch: u64,
    commands: mpsc::Sender<Command>,
    frames: watch::Receiver<Option<Arc<ScreencastFrame>>>,
}

enum Operation {
    Snapshot,
    Input(BrowserReference, BrowserInput),
}
enum Reply {
    Snapshot(BrowserSnapshot),
    Input(BrowserReference),
}
struct Command {
    operation: Operation,
    _reservation: Option<Arc<dyn Send + Sync>>,
    transmitted: Arc<AtomicBool>,
    deadline: Instant,
    response: oneshot::Sender<Result<Reply, BrowserError>>,
}

impl BrowserController {
    pub(super) async fn connect(
        target: BrowserTarget,
        execution_lease: Arc<dyn Send + Sync>,
    ) -> Result<Self, BrowserError> {
        target.validate()?;
        let stream =
            tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(target.endpoint))
                .await
                .map_err(|_| BrowserError::Deadline)?
                .map_err(|_| BrowserError::Unavailable)?;
        stream
            .set_nodelay(true)
            .map_err(|_| BrowserError::Unavailable)?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_WIRE_BYTES))
            .max_frame_size(Some(MAX_WIRE_BYTES));
        let url = format!(
            "ws://{}/devtools/page/{}",
            target.endpoint, target.target_id
        );
        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(3),
            client_async_with_config(url, stream, Some(config)),
        )
        .await
        .map_err(|_| BrowserError::Deadline)?
        .map_err(|_| BrowserError::Unavailable)?;
        Self::start(target, execution_lease, socket).await
    }

    async fn start<S>(
        target: BrowserTarget,
        execution_lease: Arc<dyn Send + Sync>,
        socket: tokio_tungstenite::WebSocketStream<S>,
    ) -> Result<Self, BrowserError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
    {
        let epoch = NEXT_CONTROLLER_EPOCH
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                old.checked_add(1)
            })
            .map_err(|_| BrowserError::Unavailable)?;
        let (commands, receive) = mpsc::channel(COMMAND_CAPACITY);
        let (frame_send, frames) = watch::channel(None);
        let (stop, stopped) = watch::channel(false);
        let (started, ready) = oneshot::channel();
        let actor = actor::BrowserActor::new(
            target.clone(),
            epoch,
            socket,
            frame_send,
            stopped,
            execution_lease,
        );
        let task = tokio::spawn(async move { actor.run(receive, started).await });
        let controller = Self {
            handle: BrowserControllerHandle {
                target,
                epoch,
                commands,
                frames,
            },
            stop,
            task: Some(task),
        };
        match tokio::time::timeout(Duration::from_secs(5), ready).await {
            Ok(Ok(Ok(()))) => Ok(controller),
            Ok(Ok(Err(error))) => Err(error),
            _ => Err(BrowserError::Unavailable),
        }
    }

    pub(super) fn handle(&self) -> BrowserControllerHandle {
        self.handle.clone()
    }

    pub(super) async fn shutdown(mut self) -> Result<(), BrowserError> {
        self.stop.send_replace(true);
        let mut task = self.task.take().expect("owned controller task");
        match tokio::time::timeout(Duration::from_secs(1), &mut task).await {
            Ok(result) => result.map_err(|_| BrowserError::Unavailable)?,
            Err(_) => {
                task.abort();
                let _ = task.await;
                Err(BrowserError::Deadline)
            }
        }
    }
}

impl Drop for BrowserController {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl BrowserControllerHandle {
    pub(super) fn target(&self) -> &BrowserTarget {
        &self.target
    }
    pub(super) fn frames(&self) -> watch::Receiver<Option<Arc<ScreencastFrame>>> {
        self.frames.clone()
    }
    pub(super) async fn snapshot(&self) -> Result<BrowserSnapshot, BrowserError> {
        match self.request(Operation::Snapshot, None).await? {
            Reply::Snapshot(snapshot) => Ok(snapshot),
            _ => Err(BrowserError::Protocol),
        }
    }
    /// The Room owner supplies its admitted input reservation. The command
    /// retains it even if this future is dropped; transmitted input is never retried.
    pub(super) async fn input(
        &self,
        reference: BrowserReference,
        input: BrowserInput,
        reservation: Arc<dyn Send + Sync>,
    ) -> Result<BrowserReference, BrowserError> {
        reference.validate(&self.target, self.epoch)?;
        input.validate()?;
        match self
            .request(Operation::Input(reference, input), Some(reservation))
            .await?
        {
            Reply::Input(reference) => Ok(reference),
            _ => Err(BrowserError::Protocol),
        }
    }
    async fn request(
        &self,
        operation: Operation,
        reservation: Option<Arc<dyn Send + Sync>>,
    ) -> Result<Reply, BrowserError> {
        let (response, result) = oneshot::channel();
        let transmitted = Arc::new(AtomicBool::new(false));
        self.commands
            .try_send(Command {
                operation,
                _reservation: reservation,
                transmitted: transmitted.clone(),
                deadline: Instant::now() + COMMAND_TIMEOUT,
                response,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => BrowserError::Busy,
                mpsc::error::TrySendError::Closed(_) => BrowserError::Unavailable,
            })?;
        result.await.map_err(|_| {
            if transmitted.load(Ordering::Acquire) {
                BrowserError::OutcomeUncertain
            } else {
                BrowserError::Unavailable
            }
        })?
    }
}

#[cfg(test)]
mod tests;
