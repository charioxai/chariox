//! Bounded worker-side transport; dropping a pending request cancels its broker work.
use crate::scope::Scope;
use chariox_app_runtime::wire::{
    Channel, Failure, Message, Outcome, RemoteError, Sender, Success, WIRE_VERSION,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::UnixStream,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};

pub const CAPACITY: usize = 128;
const FRAME_BUDGET: Duration = Duration::from_secs(30);
type Pending = HashMap<String, oneshot::Sender<Outcome>>;

struct Outgoing {
    message: Message,
    live: Option<Arc<FrameState>>,
}

// The first write and cancellation compete once; retired requests stay unsent.
#[derive(Default)]
struct FrameState(AtomicU8);
const QUEUED: u8 = 0;
const SENT: u8 = 1;
const RETIRED: u8 = 2;
impl FrameState {
    fn start(&self) -> bool {
        self.0
            .compare_exchange(QUEUED, SENT, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    fn retire(&self) -> bool {
        self.0.swap(RETIRED, Ordering::AcqRel) == SENT
    }
}

pub fn error(code: &str, message: impl Into<String>) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: message.into(),
        retryable: Some(false),
    }
}
pub fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone)]
pub struct Ipc {
    generation: Arc<str>,
    next: Arc<AtomicU64>,
    pending: Arc<Mutex<Pending>>,
    outgoing: mpsc::Sender<Outgoing>,
    closed: watch::Sender<bool>,
}
pub struct Transport {
    pub ipc: Ipc,
    pub incoming: mpsc::Receiver<Message>,
    tasks: Vec<JoinHandle<()>>,
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.ipc.close();
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Ipc {
    pub fn start(stream: UnixStream, generation: String) -> wasmtime::Result<Transport> {
        let (mut reader, mut writer) =
            Channel::new(stream, generation.clone(), Sender::Supervisor)?.split();
        let (outgoing, mut messages) = mpsc::channel::<Outgoing>(CAPACITY);
        let (incoming, requests) = mpsc::channel(CAPACITY);
        let ipc = Ipc {
            generation: generation.into(),
            next: Arc::default(),
            pending: Arc::default(),
            outgoing,
            closed: watch::channel(false).0,
        };
        let writing = ipc.clone();
        let write = tokio::spawn(async move {
            let mut closed = writing.closed.subscribe();
            loop {
                let message = tokio::select! {
                    biased;
                    _ = closed.wait_for(|value| *value) => break,
                    message = messages.recv() => match message { Some(message) => message, None => break },
                };
                if message.live.as_ref().is_some_and(|live| !live.start()) {
                    continue;
                }
                if writer.send(&message.message, FRAME_BUDGET).await.is_err() {
                    break;
                }
            }
            writing.close();
        });
        let reading = ipc.clone();
        let read = tokio::spawn(async move {
            let mut closed = reading.closed.subscribe();
            loop {
                let message = tokio::select! {
                    biased;
                    _ = closed.wait_for(|value| *value) => break,
                    message = reader.receive(FRAME_BUDGET) => match message { Ok(message) => message, Err(_) => break },
                };
                match message {
                    Message::Response { id, outcome, .. } => {
                        if let Some(sender) = reading.pending.lock().unwrap().remove(&id) {
                            let _ = sender.send(outcome);
                        }
                    }
                    message => {
                        if incoming.try_send(message).is_err() {
                            break;
                        }
                    }
                }
            }
            reading.close();
        });
        Ok(Transport {
            ipc,
            incoming: requests,
            tasks: vec![read, write],
        })
    }
    pub fn close(&self) {
        self.closed.send_replace(true);
        self.pending.lock().unwrap().clear();
    }
    pub async fn closed(&self) {
        let _ = self.closed.subscribe().wait_for(|closed| *closed).await;
    }
    fn message(&self, id: String, method: String, params: Value, deadline_ms: u64) -> Message {
        Message::Request {
            version: WIRE_VERSION,
            generation: self.generation.to_string(),
            id,
            method,
            params,
            deadline_ms,
            context: None,
        }
    }
    pub async fn call(
        &self,
        scope: &Scope,
        method: &str,
        params: Value,
    ) -> Result<Value, RemoteError> {
        let id = format!("w-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let remaining = scope
            .deadline()
            .saturating_duration_since(tokio::time::Instant::now());
        let message = self.message(
            id.clone(),
            method.into(),
            params,
            wall_ms() + remaining.as_millis().min(30_000) as u64,
        );
        validate(&message, &self.generation)
            .map_err(|_| error("INVALID_ARGUMENT", "broker request exceeds the wire limits"))?;
        let (send, receive) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.len() >= CAPACITY {
                return Err(error("APP_BUSY", "too many broker requests"));
            }
            if *self.closed.borrow() {
                return Err(error("APP_IPC_CLOSED", "worker channel closed"));
            }
            pending.insert(id.clone(), send);
        }
        let live = Arc::new(FrameState::default());
        let mut guard = PendingCall {
            ipc: self.clone(),
            id: id.clone(),
            completed: false,
            live: live.clone(),
        };
        let request = async {
            self.outgoing
                .send(Outgoing {
                    message,
                    live: Some(live),
                })
                .await
                .map_err(|_| error("APP_IPC_CLOSED", "worker channel closed"))?;
            match receive.await {
                Ok(Outcome::Success(Success { result })) => Ok(result),
                Ok(Outcome::Failure(Failure { error })) => Err(error),
                Err(_) => Err(error("APP_IPC_CLOSED", "worker channel closed")),
            }
        };
        tokio::select! {
            biased;
            reason = scope.cancelled() => Err(error(reason, "App call ended; broker work cancelled")),
            _ = tokio::time::sleep_until(scope.deadline()) => Err(error("DEADLINE_EXCEEDED", "broker call deadline exceeded")),
            result = request => { guard.completed = true; result },
        }
    }
    /// Cleanup uses the bounded transport queue. If it cannot be queued,
    /// close the channel so the kernel releases all resources for this worker.
    pub fn cleanup(&self, method: &str, params: Value) {
        let id = format!("cleanup-{}", self.next.fetch_add(1, Ordering::Relaxed));
        if self
            .outgoing
            .try_send(Outgoing {
                message: self.message(id, method.into(), params, wall_ms() + 30_000),
                live: None,
            })
            .is_err()
        {
            self.close();
        }
    }
    pub async fn respond(
        &self,
        id: String,
        outcome: Result<Value, RemoteError>,
    ) -> wasmtime::Result<()> {
        let outcome = match outcome {
            Ok(result) => Outcome::Success(Success { result }),
            Err(error) => Outcome::Failure(Failure { error }),
        };
        let mut message = Message::Response {
            version: WIRE_VERSION,
            generation: self.generation.to_string(),
            id,
            outcome,
        };
        if validate(&message, &self.generation).is_err() {
            if let Message::Response { outcome, .. } = &mut message {
                *outcome = Outcome::Failure(Failure {
                    error: error("INVALID_OUTPUT", "App result exceeds the wire limits"),
                });
            }
        }
        self.outgoing
            .send(Outgoing {
                message,
                live: None,
            })
            .await
            .map_err(|_| wasmtime::Error::msg("App IPC closed"))
    }
    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}
struct PendingCall {
    ipc: Ipc,
    id: String,
    completed: bool,
    live: Arc<FrameState>,
}
impl Drop for PendingCall {
    fn drop(&mut self) {
        self.ipc.pending.lock().unwrap().remove(&self.id);
        let sent = self.live.retire();
        if !self.completed
            && sent
            && self
                .ipc
                .outgoing
                .try_send(Outgoing {
                    message: Message::Cancel {
                        version: WIRE_VERSION,
                        generation: self.ipc.generation.to_string(),
                        id: self.id.clone(),
                    },
                    live: None,
                })
                .is_err()
        {
            self.ipc.close();
        }
    }
}

fn validate(
    message: &Message,
    generation: &str,
) -> Result<(), chariox_app_runtime::wire::WireError> {
    let bytes = serde_json::to_vec(message)?;
    chariox_app_runtime::wire::decode(&bytes, generation, Sender::Worker).map(|_| ())
}
