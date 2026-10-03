use super::*;
use chariox_app_runtime::wire::RemoteError;
use chariox_app_runtime::worker_peer::ResponsePublication;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex as StdMutex,
    },
};
use tokio::sync::Notify;
struct Probe {
    pending: StdMutex<BTreeMap<String, Box<dyn ResponsePublication>>>,
    observed: Arc<Observed>,
    taken: Notify,
    large: bool,
}
#[derive(Default)]
struct Observed {
    published: AtomicUsize,
    discarded: AtomicUsize,
    busy: AtomicBool,
    poisoned: AtomicBool,
    discard_barrier: StdMutex<Option<Arc<Barrier>>>,
}
struct Guard {
    observed: Arc<Observed>,
    published: bool,
}
impl ResponsePublication for Guard {
    fn published(&mut self) {
        self.published = true;
        self.observed.published.fetch_add(1, Ordering::AcqRel);
        self.observed.busy.store(false, Ordering::Release);
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.published {
            self.observed.discarded.fetch_add(1, Ordering::AcqRel);
            self.observed.poisoned.store(true, Ordering::Release);
            self.observed.busy.store(false, Ordering::Release);
            let barrier = self.observed.discard_barrier.lock().unwrap().clone();
            if let Some(barrier) = barrier {
                barrier.hold();
            }
        }
    }
}
impl Broker for Probe {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        if request.method == "safe" {
            return Box::pin(async { Ok(Value::Null) });
        }
        let failure = if self.observed.poisoned.load(Ordering::Acquire) {
            Some("POISONED")
        } else if self.observed.busy.swap(true, Ordering::AcqRel) {
            Some("APP_BUSY")
        } else {
            None
        };
        if let Some(code) = failure {
            return Box::pin(async move {
                Err(RemoteError {
                    code: code.into(),
                    message: "fixed publication fixture".into(),
                    retryable: Some(false),
                })
            });
        }
        self.pending.lock().unwrap().insert(
            request.id,
            Box::new(Guard {
                observed: self.observed.clone(),
                published: false,
            }),
        );
        let value = if self.large {
            json!({"body":"x".repeat(4096)})
        } else {
            json!({"chunk":"once"})
        };
        Box::pin(async move { Ok(value) })
    }
    fn take_response_guard(&self, id: &str) -> Option<Box<dyn ResponsePublication>> {
        let guard = self.pending.lock().unwrap().remove(id);
        self.taken.notify_one();
        guard
    }
}
fn probe(large: bool) -> Arc<Probe> {
    Arc::new(Probe {
        pending: StdMutex::new(BTreeMap::new()),
        observed: Arc::default(),
        taken: Notify::new(),
        large,
    })
}
async fn send<T: tokio::io::AsyncWrite + Unpin>(write: &mut T, message: Message) {
    let bytes = serde_json::to_vec(&message).unwrap();
    write
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    write.write_all(&bytes).await.unwrap();
}
async fn read<T: tokio::io::AsyncRead + Unpin>(read: &mut T, first: Option<u8>) -> Message {
    let mut header = [0; 4];
    if let Some(first) = first {
        header[0] = first;
        read.read_exact(&mut header[1..]).await.unwrap();
    } else {
        read.read_exact(&mut header).await.unwrap();
    }
    let size = u32::from_be_bytes(header) as usize;
    assert!(size <= 64 * 1024);
    let mut bytes = vec![0; size];
    read.read_exact(&mut bytes).await.unwrap();
    chariox_app_runtime::wire::decode(&bytes, "1", Sender::Supervisor).unwrap()
}
#[tokio::test]
async fn complete_frame_releases_body_gate_before_actor_ack_and_next_request() {
    timeout(BUDGET, async {
        let (host, stream) = tokio::io::duplex(1024);
        let handler = probe(false);
        let (peer, _, task) = WorkerPeer::start(
            Channel::new(host, "1".into(), Sender::Worker).unwrap(),
            handler.clone(),
            PeerLimits::default(),
        )
        .unwrap();
        let (mut reading, mut writing) = tokio::io::split(stream);
        for id in ["one", "two"] {
            send(&mut writing, request(id, "read")).await;
            assert_eq!(
                result(read(&mut reading, None).await),
                json!({"chunk":"once"})
            );
        }
        peer.close();
        task.join().await.unwrap();
        assert_eq!(handler.observed.published.load(Ordering::Acquire), 2);
        assert_eq!(handler.observed.discarded.load(Ordering::Acquire), 0);
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn queued_reply_cancel_or_deadline_keeps_capacity_and_poisons_only_its_stream() {
    for expire in [false, true] {
        timeout(BUDGET, async {
            let (host, stream) = tokio::io::duplex(128);
            let handler = probe(false);
            let limits = PeerLimits {
                broker_handlers: 1,
                ..PeerLimits::default()
            };
            let (peer, _, task) = WorkerPeer::start(
                Channel::new(host, "1".into(), Sender::Worker).unwrap(),
                handler.clone(),
                limits,
            )
            .unwrap();
            let (mut reading, mut writing) = tokio::io::split(stream);
            let slot = peer.reserve(BUDGET).unwrap();
            let blocked =
                tokio::spawn(slot.request("blocking", json!({"body":"x".repeat(8192)}), None));
            // One byte proves the independent writer is already inside this
            // other frame. The guarded body reply must remain queued behind it.
            let first = reading.read_u8().await.unwrap();
            let mut call = request("body", "read");
            if expire {
                if let Message::Request { deadline_ms, .. } = &mut call {
                    *deadline_ms = deadline() - 30_000 + 75;
                }
            }
            send(&mut writing, call).await;
            handler.taken.notified().await;
            send(&mut writing, request("over-capacity", "safe")).await;
            if expire {
                tokio::time::sleep(Duration::from_millis(100)).await;
            } else {
                send(
                    &mut writing,
                    Message::Cancel {
                        version: WIRE_VERSION,
                        generation: "1".into(),
                        id: "body".into(),
                    },
                )
                .await;
            }
            let outbound = read(&mut reading, Some(first)).await;
            assert!(matches!(outbound, Message::Request { .. }));
            let mut seen = std::collections::BTreeSet::new();
            for _ in 0..2 {
                let message = read(&mut reading, None).await;
                let identity = id(&message);
                let code = error(message);
                if identity == "over-capacity" {
                    assert_eq!(code, "BUSY");
                } else {
                    assert_eq!(identity, "body");
                    assert!(matches!(code.as_str(), "CANCELLED" | "DEADLINE_EXCEEDED"));
                }
                seen.insert(identity);
            }
            assert_eq!(seen.len(), 2);
            send(&mut writing, request("health", "safe")).await;
            assert_eq!(result(read(&mut reading, None).await), Value::Null);
            send(&mut writing, request("next-body", "read")).await;
            assert_eq!(error(read(&mut reading, None).await), "POISONED");
            assert_eq!(handler.observed.published.load(Ordering::Acquire), 0);
            assert_eq!(handler.observed.discarded.load(Ordering::Acquire), 1);
            peer.close();
            task.join().await.unwrap();
            assert!(blocked.await.unwrap().is_err());
        })
        .await
        .unwrap();
    }
}
#[tokio::test]
async fn cancellation_during_partial_body_reply_closes_framing_and_discards_guard() {
    timeout(BUDGET, async {
        let (host, stream) = tokio::io::duplex(128);
        let handler = probe(true);
        let (peer, _, task) = WorkerPeer::start(
            Channel::new(host, "1".into(), Sender::Worker).unwrap(),
            handler.clone(),
            PeerLimits::default(),
        )
        .unwrap();
        let (mut reading, mut writing) = tokio::io::split(stream);
        send(&mut writing, request("body", "read")).await;
        let mut prefix = [0; 17];
        reading.read_exact(&mut prefix).await.unwrap();
        assert!(u32::from_be_bytes(prefix[..4].try_into().unwrap()) > 4096);
        send(
            &mut writing,
            Message::Cancel {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "body".into(),
            },
        )
        .await;
        assert_eq!(task.join().await.unwrap_err(), PeerError::Io);
        assert!(peer.is_closed());
        assert_eq!(handler.observed.published.load(Ordering::Acquire), 0);
        assert_eq!(handler.observed.discarded.load(Ordering::Acquire), 1);
    })
    .await
    .unwrap();
}

// A bounded fixture checkpoint outside application code. It lets the actor run
// on the other test worker while physical bytes or an ACK are held at a precise
// boundary; timeout becomes a test failure rather than an orphaned task.
#[derive(Default)]
struct Barrier {
    entered: Notify,
    released: AtomicBool,
    expired: AtomicBool,
}
impl Barrier {
    fn hold(&self) {
        self.entered.notify_one();
        let deadline = std::time::Instant::now() + BUDGET;
        while !self.released.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        if !self.released.load(Ordering::Acquire) {
            self.expired.store(true, Ordering::Release);
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrelated_request_before_unpublished_ack_cannot_reap_its_only_failure() {
    timeout(Duration::from_secs(4), async {
        let (host, stream) = tokio::io::duplex(128);
        let handler = probe(false);
        let barrier = Arc::new(Barrier::default());
        *handler.observed.discard_barrier.lock().unwrap() = Some(barrier.clone());
        let (peer, mut events, task) = WorkerPeer::start(
            Channel::new(host, "1".into(), Sender::Worker).unwrap(),
            handler.clone(),
            PeerLimits {
                broker_handlers: 1,
                ..PeerLimits::default()
            },
        )
        .unwrap();
        let (mut reading, mut writing) = tokio::io::split(stream);
        let slot = peer.reserve(BUDGET).unwrap();
        let blocked =
            tokio::spawn(slot.request("blocking", json!({"body":"x".repeat(8192)}), None));
        let first = reading.read_u8().await.unwrap();
        let mut call = request("body", "read");
        if let Message::Request { deadline_ms, .. } = &mut call {
            *deadline_ms = deadline() - 30_000 + 100;
        }
        send(&mut writing, call).await;
        handler.taken.notified().await;
        tokio::time::sleep(Duration::from_millis(125)).await;
        read(&mut reading, Some(first)).await;
        barrier.entered.notified().await; // finished=true, physical=false, ACK not sent
        send(&mut writing, request("new", "safe")).await;
        send(
            &mut writing,
            Message::Event {
                version: WIRE_VERSION,
                generation: "1".into(),
                name: "worker.barrier".into(),
                data: Value::Null,
            },
        )
        .await;
        assert_eq!(events.recv().await.unwrap().name, "worker.barrier");
        barrier.released.store(true, Ordering::Release);
        let mut codes = BTreeMap::new();
        for _ in 0..2 {
            let message = read(&mut reading, None).await;
            codes.insert(id(&message), error(message));
        }
        assert_eq!(codes.get("body").unwrap(), "DEADLINE_EXCEEDED");
        assert_eq!(codes.get("new").unwrap(), "BUSY");
        assert!(!barrier.expired.load(Ordering::Acquire));
        peer.close();
        task.join().await.unwrap();
        assert!(blocked.await.unwrap().is_err());
    })
    .await
    .unwrap();
}
struct CompleteWriteHook {
    stream: DuplexStream,
    remaining: usize,
    held: bool,
}
impl tokio::io::AsyncRead for CompleteWriteHook {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}
impl tokio::io::AsyncWrite for CompleteWriteHook {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if self.held {
            return std::task::Poll::Pending;
        }
        let result = std::pin::Pin::new(&mut self.stream).poll_write(cx, bytes);
        if let std::task::Poll::Ready(Ok(count)) = &result {
            if self.remaining > 0 {
                self.remaining = self.remaining.saturating_sub(*count);
                if self.remaining == 0 {
                    // The transport consumed the bytes but has not completed
                    // its write contract. Yield so the independent read half
                    // can observe cancellation; shutdown must poison framing.
                    self.held = true;
                    return std::task::Poll::Pending;
                }
            }
        }
        result
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
#[tokio::test]
async fn all_reply_bytes_without_completed_write_contract_cancel_closes_framing() {
    timeout(Duration::from_secs(4), async {
        let (host, stream) = tokio::io::duplex(1024);
        let handler = probe(false);
        let bytes = serde_json::to_vec(&response("body", json!({"chunk":"once"})))
            .unwrap()
            .len()
            + 4;
        let host = CompleteWriteHook {
            stream: host,
            remaining: bytes,
            held: false,
        };
        let (peer, _, task) = WorkerPeer::start(
            Channel::new(host, "1".into(), Sender::Worker).unwrap(),
            handler.clone(),
            PeerLimits::default(),
        )
        .unwrap();
        let (mut reading, mut writing) = tokio::io::split(stream);
        send(&mut writing, request("body", "read")).await;
        assert_eq!(
            result(read(&mut reading, None).await),
            json!({"chunk":"once"})
        );
        // All bytes arrived, but the host write never completed. The peer must
        // close instead of publishing another failure or accepting body work.
        send(
            &mut writing,
            Message::Cancel {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "body".into(),
            },
        )
        .await;
        assert_eq!(task.join().await.unwrap_err(), PeerError::Io);
        assert!(peer.is_closed());
        assert_eq!(
            reading.read_u8().await.unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        assert_eq!(handler.observed.published.load(Ordering::Acquire), 0);
        assert_eq!(handler.observed.discarded.load(Ordering::Acquire), 1);
    })
    .await
    .unwrap();
}
