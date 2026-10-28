//! Worker side of the existing App wire (FD3), on the kernel's own codec.
use chariox_app_runtime::wire::{
    Channel, Failure, Message, Outcome, RemoteError, Sender, Success, WIRE_VERSION,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};

const FRAME_BUDGET: Duration = Duration::from_secs(30);
const CALL_BUDGET_MS: u64 = 30_000;

/// A supervisor-to-worker request the guest must answer.
pub struct Inbound {
    pub id: String,
    pub method: String,
    pub params: Value,
    pub deadline_ms: u64,
    pub context: Option<Value>,
}

#[derive(Clone)]
pub struct Ipc {
    generation: Arc<str>,
    sequence: Arc<AtomicU64>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Outcome>>>>,
    outgoing: mpsc::Sender<Message>,
}

pub fn wall_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

impl Ipc {
    /// Starts the reader/writer tasks; inbound requests arrive on the receiver.
    pub fn start(stream: UnixStream, generation: String) -> (Self, mpsc::Receiver<Inbound>) {
        let channel = Channel::new(stream, generation.clone(), Sender::Supervisor).expect("generation");
        let (mut reader, mut writer) = channel.split();
        let (outgoing, mut queue) = mpsc::channel::<Message>(128);
        let (inbound, requests) = mpsc::channel::<Inbound>(64);
        let ipc = Self {
            generation: generation.into(),
            sequence: Arc::new(AtomicU64::new(0)),
            pending: Arc::default(),
            outgoing,
        };
        tokio::spawn(async move {
            while let Some(message) = queue.recv().await {
                if writer.send(&message, FRAME_BUDGET).await.is_err() {
                    std::process::exit(133);
                }
            }
        });
        let pending = ipc.pending.clone();
        tokio::spawn(async move {
            loop {
                match reader.receive(FRAME_BUDGET).await {
                    Ok(Message::Response { id, outcome, .. }) => {
                        if let Some(reply) = pending.lock().unwrap().remove(&id) {
                            let _ = reply.send(outcome);
                        }
                    }
                    Ok(Message::Request { id, method, params, deadline_ms, context, .. }) => {
                        if inbound.send(Inbound { id, method, params, deadline_ms, context }).await.is_err() {
                            return;
                        }
                    }
                    // Cancellation of a sync guest call cannot interrupt it; the
                    // per-call epoch deadline bounds it instead.
                    Ok(Message::Cancel { .. }) | Ok(Message::Event { .. }) => {}
                    // Channel loss ends the worker, as in the Node bootstrap.
                    Err(_) => std::process::exit(133),
                }
            }
        });
        (ipc, requests)
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RemoteError> {
        let id = format!("w-{}", self.sequence.fetch_add(1, Ordering::Relaxed) + 1);
        let (reply, receive) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), reply);
        let message = Message::Request {
            version: WIRE_VERSION,
            generation: self.generation.to_string(),
            id: id.clone(),
            method: method.into(),
            params,
            deadline_ms: wall_ms() + CALL_BUDGET_MS,
            context: None,
        };
        if self.outgoing.send(message).await.is_err() {
            return Err(error("APP_IPC_CLOSED", "worker channel closed"));
        }
        match tokio::time::timeout(Duration::from_millis(CALL_BUDGET_MS), receive).await {
            Ok(Ok(Outcome::Success(Success { result }))) => Ok(result),
            Ok(Ok(Outcome::Failure(Failure { error }))) => Err(error),
            Ok(Err(_)) => Err(error("APP_IPC_CLOSED", "worker channel closed")),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(error("DEADLINE_EXCEEDED", "kernel call deadline exceeded"))
            }
        }
    }

    pub async fn respond(&self, id: String, outcome: Result<Value, RemoteError>) {
        let outcome = match outcome {
            Ok(result) => Outcome::Success(Success { result }),
            Err(error) => Outcome::Failure(Failure { error }),
        };
        let message = Message::Response { version: WIRE_VERSION, generation: self.generation.to_string(), id, outcome };
        let _ = self.outgoing.send(message).await;
    }
}

pub fn error(code: &str, message: &str) -> RemoteError {
    RemoteError { code: code.into(), message: message.into(), retryable: Some(false) }
}
