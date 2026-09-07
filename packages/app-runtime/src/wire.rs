//! Versioned, bounded messages on an inherited App worker stream.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const WIRE_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_DEADLINE_MS: u64 = 30_000;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

/// Strict variants reject extra authority fields and ambiguous result/error
/// responses. `context` is legal only on supervisor-to-worker requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Message {
    Request {
        version: u32,
        generation: String,
        id: String,
        method: String,
        params: Value,
        deadline_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<Value>,
    },
    Response {
        version: u32,
        generation: String,
        id: String,
        #[serde(flatten)]
        outcome: Outcome,
    },
    Cancel {
        version: u32,
        generation: String,
        id: String,
    },
    Event {
        version: u32,
        generation: String,
        name: String,
        data: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Outcome {
    Success(Success),
    Failure(Failure),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Success {
    pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub error: RemoteError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sender {
    Supervisor,
    Worker,
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("app_ipc_closed")]
    Closed,
    #[error("app_ipc_timeout")]
    Timeout,
    #[error("app_ipc_frame_limit")]
    FrameLimit,
    #[error("app_ipc_invalid_message: {0}")]
    Invalid(&'static str),
    #[error("app_ipc_stale_generation")]
    StaleGeneration,
    #[error("app_ipc_io: {0}")]
    Io(#[from] std::io::Error),
    #[error("app_ipc_json: {0}")]
    Json(#[from] serde_json::Error),
}

impl Message {
    pub fn generation(&self) -> &str {
        match self {
            Self::Request { generation, .. }
            | Self::Response { generation, .. }
            | Self::Cancel { generation, .. }
            | Self::Event { generation, .. } => generation,
        }
    }

    pub fn validate(&self, generation: &str, sender: Sender) -> Result<(), WireError> {
        let version = match self {
            Self::Request { version, .. }
            | Self::Response { version, .. }
            | Self::Cancel { version, .. }
            | Self::Event { version, .. } => *version,
        };
        if version != WIRE_VERSION {
            return Err(WireError::Invalid("version"));
        }
        valid_identifier(self.generation())?;
        if self.generation() != generation {
            return Err(WireError::StaleGeneration);
        }
        match self {
            Self::Request {
                id,
                method,
                deadline_ms,
                context,
                ..
            } => {
                valid_identifier(id)?;
                valid_identifier(method)?;
                if *deadline_ms == 0 || *deadline_ms > MAX_SAFE_INTEGER {
                    return Err(WireError::Invalid("deadline_ms"));
                }
                if sender == Sender::Worker && context.is_some() {
                    return Err(WireError::Invalid("worker cannot supply caller context"));
                }
                if context.as_ref().is_some_and(|value| !value.is_object()) {
                    return Err(WireError::Invalid("context must be an object"));
                }
            }
            Self::Response { id, outcome, .. } => {
                valid_identifier(id)?;
                if let Outcome::Failure(failure) = outcome {
                    valid_identifier(&failure.error.code)?;
                    if failure.error.message.len() > 4096 {
                        return Err(WireError::Invalid("error message limit"));
                    }
                }
            }
            Self::Cancel { id, .. } => valid_identifier(id)?,
            Self::Event { name, .. } => valid_identifier(name)?,
        }
        Ok(())
    }
}

fn valid_identifier(value: &str) -> Result<(), WireError> {
    if value.is_empty()
        || value.len() > 128
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace() || ch == '\u{feff}')
    {
        return Err(WireError::Invalid("identifier"));
    }
    Ok(())
}

/// Check the envelope before typed decoding: Serde cannot combine a flattened
/// result/error union with deny_unknown_fields on its enclosing enum.
pub fn decode(bytes: &[u8], generation: &str, sender: Sender) -> Result<Message, WireError> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return Err(WireError::FrameLimit);
    }
    let crate::wire_json::UniqueJson(value) = serde_json::from_slice(bytes)?;
    validate_json(&value, 0)?;
    let object = value
        .as_object()
        .ok_or(WireError::Invalid("object required"))?;
    let kind = object.get("kind").and_then(Value::as_str);
    let allowed: &[&str] = match kind {
        Some("request") => &[
            "kind",
            "version",
            "generation",
            "id",
            "method",
            "params",
            "deadline_ms",
            "context",
        ],
        Some("response") => &["kind", "version", "generation", "id", "result", "error"],
        Some("cancel") => &["kind", "version", "generation", "id"],
        Some("event") => &["kind", "version", "generation", "name", "data"],
        _ => return Err(WireError::Invalid("kind")),
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(WireError::Invalid("unknown envelope field"));
    }
    for key in allowed {
        if ["context", "result", "error"].contains(key) {
            continue;
        }
        if !object.contains_key(*key) {
            return Err(WireError::Invalid("missing envelope field"));
        }
    }
    if kind == Some("response") && object.contains_key("result") == object.contains_key("error") {
        return Err(WireError::Invalid(
            "response must contain exactly one outcome",
        ));
    }
    if object
        .get("context")
        .is_some_and(|context| !context.is_object())
    {
        return Err(WireError::Invalid("context must be an object"));
    }
    if sender == Sender::Worker && object.contains_key("context") {
        return Err(WireError::Invalid("worker cannot supply caller context"));
    }
    if object
        .get("error")
        .and_then(|error| error.get("retryable"))
        .is_some_and(|retryable| !retryable.is_boolean())
    {
        return Err(WireError::Invalid("error retryable must be boolean"));
    }
    // UniqueJson rejects duplicates and normalizes safe integer spellings before
    // typed decoding, matching JavaScript's single number representation.
    let message: Message = serde_json::from_value(value)?;
    message.validate(generation, sender)?;
    Ok(message)
}

fn validate_json(value: &Value, depth: usize) -> Result<(), WireError> {
    if value.is_array() || value.is_object() {
        if depth >= 64 {
            return Err(WireError::Invalid("JSON depth limit"));
        }
        match value {
            Value::Array(items) => {
                for item in items {
                    validate_json(item, depth + 1)?;
                }
            }
            Value::Object(items) => {
                for item in items.values() {
                    validate_json(item, depth + 1)?;
                }
            }
            _ => unreachable!(),
        }
    }
    if let Value::Number(number) = value {
        let number = number.as_f64().ok_or(WireError::Invalid("JSON number"))?;
        if !number.is_finite() || (number.fract() == 0.0 && number.abs() > MAX_SAFE_INTEGER as f64)
        {
            return Err(WireError::Invalid("unsafe JSON integer; use a string"));
        }
    }
    Ok(())
}

/// Both halves share a terminal failure latch. A dedicated reader can wait for
/// worker requests while the writer dispatches tools or answers broker calls.
/// Cancelling a partial operation poisons both halves, never resets framing.
pub struct Channel<T> {
    reader: Reader<tokio::io::ReadHalf<T>>,
    writer: Writer<tokio::io::WriteHalf<T>>,
}

pub struct Reader<T> {
    stream: T,
    generation: String,
    remote: Sender,
    closed: tokio::sync::watch::Sender<bool>,
}

pub struct Writer<T> {
    stream: T,
    generation: String,
    local: Sender,
    closed: tokio::sync::watch::Sender<bool>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> Channel<T> {
    pub fn new(stream: T, generation: String, remote: Sender) -> Result<Self, WireError> {
        valid_identifier(&generation)?;
        let (read, write) = tokio::io::split(stream);
        let (closed, _) = tokio::sync::watch::channel(false);
        let local = match remote {
            Sender::Supervisor => Sender::Worker,
            Sender::Worker => Sender::Supervisor,
        };
        Ok(Self {
            reader: Reader {
                stream: read,
                generation: generation.clone(),
                remote,
                closed: closed.clone(),
            },
            writer: Writer {
                stream: write,
                generation,
                local,
                closed,
            },
        })
    }

    pub fn is_closed(&self) -> bool {
        *self.reader.closed.borrow()
    }

    pub fn split(
        self,
    ) -> (
        Reader<tokio::io::ReadHalf<T>>,
        Writer<tokio::io::WriteHalf<T>>,
    ) {
        (self.reader, self.writer)
    }

    pub async fn receive(&mut self, budget: Duration) -> Result<Message, WireError> {
        self.reader.receive(budget).await
    }

    pub async fn send(&mut self, message: &Message, budget: Duration) -> Result<(), WireError> {
        self.writer.send(message, budget).await
    }
}

impl<T: AsyncRead + Unpin> Reader<T> {
    /// Idle waiting has no frame deadline. Once the first byte arrives, the
    /// budget covers the rest of the header and body. Worker heartbeat policy
    /// belongs to the supervisor, separately from partial-frame protection.
    pub async fn receive(&mut self, budget: Duration) -> Result<Message, WireError> {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return Err(WireError::Closed);
        }
        // A one-byte read cannot consume part of its result and remain pending.
        // Cancelling this idle wait therefore leaves framing intact.
        let first = tokio::select! {
            biased;
            _ = closed.changed() => return Err(WireError::Closed),
            result = self.stream.read_u8() => match result {
                Ok(first) => first,
                Err(error) => {
                    self.closed.send_replace(true);
                    return Err(WireError::Io(error));
                }
            },
        };
        let mut guard = IoGuard::new(self.closed.clone());
        let read = async {
            let mut header = [first, 0, 0, 0];
            self.stream.read_exact(&mut header[1..]).await?;
            let length = u32::from_be_bytes(header) as usize;
            if length == 0 || length > MAX_FRAME_BYTES {
                return Err(WireError::FrameLimit);
            }
            let mut bytes = vec![0; length];
            self.stream.read_exact(&mut bytes).await?;
            decode(&bytes, &self.generation, self.remote)
        };
        let result = tokio::select! {
            biased;
            _ = closed.changed() => Err(WireError::Closed),
            result = tokio::time::timeout(budget, read) => result.map_err(|_| WireError::Timeout)?,
        };
        guard.completed = result.is_ok();
        result
    }
}

impl<T: AsyncWrite + Unpin> Writer<T> {
    /// Awaiting a write supplies backpressure. The supervisor must use a bounded
    /// outbound mailbox rather than spawning unbounded writer tasks.
    pub async fn send(&mut self, message: &Message, budget: Duration) -> Result<(), WireError> {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return Err(WireError::Closed);
        }
        message.validate(&self.generation, self.local)?;
        let mut writer = BoundedWriter(Vec::new());
        serde_json::to_writer(&mut writer, message)?;
        // Apply the same JSON-number/depth contract to outgoing values.
        decode(&writer.0, &self.generation, self.local)?;
        let mut guard = IoGuard::new(self.closed.clone());
        let write = async {
            self.stream.write_u32(writer.0.len() as u32).await?;
            self.stream.write_all(&writer.0).await?;
            self.stream.flush().await.map_err(WireError::Io)
        };
        let result = tokio::select! {
            biased;
            _ = closed.changed() => Err(WireError::Closed),
            result = tokio::time::timeout(budget, write) => result.map_err(|_| WireError::Timeout)?,
        };
        guard.completed = result.is_ok();
        result
    }
}

struct IoGuard {
    closed: tokio::sync::watch::Sender<bool>,
    completed: bool,
}
impl IoGuard {
    fn new(closed: tokio::sync::watch::Sender<bool>) -> Self {
        Self {
            closed,
            completed: false,
        }
    }
}
impl Drop for IoGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.closed.send_replace(true);
        }
    }
}

struct BoundedWriter(Vec<u8>);
impl std::io::Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_FRAME_BYTES - self.0.len() {
            return Err(std::io::Error::other("app_ipc_frame_limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
