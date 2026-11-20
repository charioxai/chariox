//! The relay socket lives on its own thread and runtime. Connecting, reading,
//! answering the relay's pings, the silent-relay deadline and writing never
//! wait for kernel work: a busy host or a blocked kernel runtime delays
//! replies, never the frames that keep the kernel's relay link up.

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use chariox_relay::frame_transport::MAX_UNACKED_FRAMES;
use chariox_relay::protocol::RelayEnvelope;
use futures_util::{Stream, StreamExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{protocol::CloseFrame, Error, Message};

pub(super) enum ReaderEnd {
    SocketClose(Option<CloseFrame>),
    RelayClose(String),
    Failed(&'static str),
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("chariox-relay-socket")
            .thread_stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
            .enable_all()
            .build()
            .expect("the relay socket runtime must start")
    })
}

/// Runs `work` on the socket runtime. The socket's I/O registers with the
/// runtime that creates it, so the connection is made here too.
pub(super) fn spawn<F>(work: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(work)
}

/// Aborts the socket task when the connection it belongs to ends, whichever
/// way the connector leaves it.
pub(super) struct SocketTask(pub(super) JoinHandle<()>);

impl Drop for SocketTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Socket control and upload credit continue while the connector is withholding
/// its download receipts. The relay's wire window bounds queued data, and the
/// inbox has space for a binary envelope and checkpoint per unacknowledged frame.
pub(super) fn spawn_reader<R>(
    mut reader: R,
    pong_tx: mpsc::Sender<Vec<u8>>,
    upload_receipt_tx: Option<mpsc::Sender<(u32, u32)>>,
    idle_limit: Duration,
) -> (
    mpsc::Receiver<Message>,
    oneshot::Receiver<ReaderEnd>,
    SocketTask,
)
where
    R: Stream<Item = Result<Message, Error>> + Unpin + Send + 'static,
{
    let (inbound_tx, inbound_rx) = mpsc::channel(2 * MAX_UNACKED_FRAMES);
    let (end_tx, end_rx) = oneshot::channel();
    let task = spawn(async move {
        let ended = loop {
            let frame = match tokio::time::timeout(idle_limit, reader.next()).await {
                Err(_) => break ReaderEnd::Failed("relay stopped sending frames"),
                Ok(Some(Ok(frame))) => frame,
                Ok(Some(Err(_)) | None) => break ReaderEnd::Failed("relay read failed or ended"),
            };
            match frame {
                Message::Ping(payload) => {
                    let _ = pong_tx.try_send(payload.to_vec());
                    continue;
                }
                Message::Pong(_) | Message::Frame(_) => continue,
                Message::Close(close) => break ReaderEnd::SocketClose(close),
                Message::Text(ref text) => {
                    if let Some(receipts) = &upload_receipt_tx {
                        if let Some(receipt) = crate::transport::relay_upload::receipt(text) {
                            if receipts.try_send(receipt).is_err() {
                                break ReaderEnd::Failed("relay upload receipt lost");
                            }
                            continue;
                        }
                    }
                    // The relay sends this small control envelope outside its data window.
                    if text.starts_with("{\"kind\":\"close\"") {
                        if let Ok(RelayEnvelope::Close { reason, .. }) = serde_json::from_str(text)
                        {
                            break ReaderEnd::RelayClose(reason);
                        }
                    }
                }
                Message::Binary(_) => {}
            }
            match inbound_tx.try_send(frame) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => return,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    break ReaderEnd::Failed("relay exceeded its receive window");
                }
            }
        };
        let _ = end_tx.send(ended);
    });
    (inbound_rx, end_rx, SocketTask(task))
}
