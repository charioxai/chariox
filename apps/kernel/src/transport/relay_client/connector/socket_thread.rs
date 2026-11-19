//! The relay socket lives on its own thread and runtime. Connecting, reading,
//! answering the relay's pings, the silent-relay deadline and writing never
//! wait for kernel work: a busy host or a blocked kernel runtime delays
//! replies, never the frames that keep the kernel's relay link up.

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{Error, Message};

/// What the reader hands to the connector: a frame, or why the socket ended.
pub(super) type Inbound = Result<Message, &'static str>;

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

/// Reads the relay socket on the socket runtime. Pings are answered at once
/// through `pong_tx`; every other frame goes to the connector. The relay sends
/// a ping every 30 seconds, so `idle_limit` of silence means the link is gone.
///
/// The queue toward the connector needs no bound of its own: the relay sends
/// data frames only within the transport window the connector's receipts open.
pub(super) fn spawn_reader<R>(
    mut reader: R,
    pong_tx: mpsc::Sender<Vec<u8>>,
    idle_limit: Duration,
) -> (mpsc::UnboundedReceiver<Inbound>, SocketTask)
where
    R: Stream<Item = Result<Message, Error>> + Unpin + Send + 'static,
{
    let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
    let task = spawn(async move {
        let ended = loop {
            let frame = match tokio::time::timeout(idle_limit, reader.next()).await {
                Err(_) => break "relay stopped sending frames",
                Ok(Some(Ok(frame))) => frame,
                Ok(Some(Err(_)) | None) => break "relay read failed or ended",
            };
            match frame {
                Message::Ping(payload) => {
                    let _ = pong_tx.try_send(payload.to_vec());
                }
                Message::Pong(_) | Message::Frame(_) => {}
                frame => {
                    if inbound_tx.send(Ok(frame)).is_err() {
                        return;
                    }
                }
            }
        };
        let _ = inbound_tx.send(Err(ended));
    });
    (inbound_rx, SocketTask(task))
}
