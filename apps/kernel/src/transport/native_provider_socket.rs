//! MP-08 / MP-10 / MP-11: bounded, run-fenced official Codex display sockets.
use std::{collections::BTreeMap, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch, Mutex};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};

use super::native_provider_buffer::{
    NativeBufferedFrame, NativeConnectionOutputBudget, NativeOutputBudget, OUTPUT_FRAME_BYTES,
};
use super::native_provider_exchange::NativeExchangeState;

use crate::{
    error::DaemonError,
    local::{NativeProviderTransportOperation, NativeProviderTransportResult},
};

pub(crate) const FRAME_BYTES: usize = 256 * 1024;
const QUEUE_FRAMES: usize = 32;
const CONNECTION_LIMIT: usize = 32;

pub(crate) fn error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "native provider transport",
        message: message.into(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SocketScope {
    pub session_id: String,
    pub agent_id: String,
    pub attachment_id: String,
    pub provider_run_id: String,
    pub leased_agent_id: Option<String>,
    pub endpoint: String,
    pub working_directory: Option<String>,
}

struct Connection {
    scope: SocketScope,
    sender: mpsc::Sender<String>,
    exchange: Mutex<NativeExchangeState>,
    open_request_id: String,
    output_budget: NativeConnectionOutputBudget,
    touched: std::sync::Mutex<std::time::Instant>,
    shutdown: watch::Sender<bool>,
}

#[derive(Clone, Default)]
pub(crate) struct NativeProviderSockets(
    Arc<Mutex<BTreeMap<String, Arc<Connection>>>>,
    NativeOutputBudget,
);

/// Display/control reads only. User turns, tool execution and approval replies
/// continue through the kernel's normal native prompt/interaction adapters.
pub(crate) fn validate_frame(frame: &str) -> Result<(), DaemonError> {
    if frame.len() > FRAME_BYTES {
        return Err(error("native frame exceeds 256 KiB"));
    }
    let value: serde_json::Value =
        serde_json::from_str(frame).map_err(|_| error("native frame must be JSON"))?;
    let method = value
        .get("method")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error("native approval replies must use runtime interactions"))?;
    if !matches!(
        method,
        "initialize"
            | "initialized"
            | "thread/start"
            | "thread/resume"
            | "thread/read"
            | "thread/list"
            | "thread/loaded/list"
            | "thread/archive"
            | "thread/unarchive"
            | "thread/name/set"
            | "thread/compact/start"
            | "model/list"
            | "account/read"
            | "account/rateLimits/read"
            | "config/read"
            | "configRequirements/read"
            | "skills/list"
            | "hooks/list"
            | "plugin/list"
            | "plugin/installed"
            | "plugin/read"
            | "plugin/skill/read"
            | "mcpServerStatus/list"
            | "app/read"
            | "app/list"
            | "app/installed"
            | "thread/unsubscribe"
            | "thread/turns/list"
            | "thread/items/list"
            | "threadSection/list"
            | "permissionProfile/list"
            | "modelProvider/capabilities/read"
            | "experimentalFeature/list"
    ) {
        return Err(error(
            "native method must use the owning kernel prompt/control path",
        ));
    }
    Ok(())
}

// Native folder trust is the sole provider-local write admitted here.
// It cannot edit model, sandbox, credentials, features or another project.
fn validate_scoped_frame(frame: &str, scope: &SocketScope) -> Result<(), DaemonError> {
    if validate_frame(frame).is_ok() {
        return Ok(());
    }
    if frame.len() > FRAME_BYTES {
        return Err(error("native frame exceeds 256 KiB"));
    }
    let value: serde_json::Value =
        serde_json::from_str(frame).map_err(|_| error("native frame must be JSON"))?;
    let params = value
        .get("params")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| error("native trust update requires parameters"))?;
    let directory = scope
        .working_directory
        .as_deref()
        .filter(|path| std::path::Path::new(path).is_absolute())
        .ok_or_else(|| error("native trust update requires the owning workspace"))?;
    let key = format!(
        "projects.{}.trust_level",
        serde_json::to_string(directory)
            .map_err(|_| error("native workspace cannot be encoded"))?
    );
    let edits = params
        .get("edits")
        .and_then(serde_json::Value::as_array)
        .filter(|edits| edits.len() == 1)
        .ok_or_else(|| error("native trust update requires one scoped edit"))?;
    let edit = edits[0]
        .as_object()
        .ok_or_else(|| error("invalid native trust edit"))?;
    if value.get("method").and_then(serde_json::Value::as_str) != Some("config/batchWrite")
        || params.keys().any(|key| {
            !matches!(
                key.as_str(),
                "edits" | "filePath" | "expectedVersion" | "reloadUserConfig"
            )
        })
        || params.get("filePath").is_some_and(|path| !path.is_null())
        || params
            .get("expectedVersion")
            .is_some_and(|version| !version.is_null() && !version.is_string())
        || params
            .get("reloadUserConfig")
            .is_some_and(|reload| !reload.is_boolean())
        || edit.len() != 3
        || edit.get("keyPath").and_then(serde_json::Value::as_str) != Some(key.as_str())
        || edit
            .get("mergeStrategy")
            .and_then(serde_json::Value::as_str)
            != Some("replace")
        || !matches!(
            edit.get("value").and_then(serde_json::Value::as_str),
            Some("trusted" | "untrusted")
        )
    {
        return Err(error(
            "native config writes are limited to the owning workspace trust",
        ));
    }
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> Result<(), DaemonError> {
    let url = url::Url::parse(endpoint).map_err(|_| error("invalid owning provider endpoint"))?;
    if url.scheme() != "ws"
        || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_none()
    {
        return Err(error(
            "owning provider endpoint must be an official loopback WebSocket",
        ));
    }
    Ok(())
}

// Return no watch Ref: select branches may await after another branch wins.
async fn wait_for_shutdown(receiver: &mut watch::Receiver<bool>) {
    let _ = receiver.wait_for(|closed| *closed).await;
}

impl NativeProviderSockets {
    pub(crate) async fn execute<F, Fut>(
        &self,
        scope: SocketScope,
        operation: NativeProviderTransportOperation,
        authorize: F,
    ) -> Result<NativeProviderTransportResult, DaemonError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), DaemonError>> + Send,
    {
        authorize().await?;
        match operation {
            NativeProviderTransportOperation::Open { request_id } => {
                if request_id.is_empty()
                    || request_id.len() > 64
                    || !request_id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                {
                    return Err(error("native open request identity is invalid"));
                }
                validate_endpoint(&scope.endpoint)?;
                let mut entries = self.0.lock().await;
                if let Some((id, _)) = entries
                    .iter()
                    .find(|(_, entry)| entry.scope == scope && entry.open_request_id == request_id)
                {
                    return Ok(NativeProviderTransportResult {
                        connection_id: id.clone(),
                        sequence: 0,
                        frames: vec![],
                        gzip_fragment: None,
                        closed: false,
                    });
                }
                if entries.len() >= CONNECTION_LIMIT
                    || entries
                        .values()
                        .filter(|entry| entry.scope == scope)
                        .count()
                        >= 2
                {
                    return Err(error(
                        "native connection limit reached; close the existing attachment first",
                    ));
                }
                let config = WebSocketConfig::default()
                    .max_message_size(Some(OUTPUT_FRAME_BYTES))
                    .max_frame_size(Some(OUTPUT_FRAME_BYTES));
                let (socket, _) = tokio::time::timeout(
                    Duration::from_secs(5),
                    tokio_tungstenite::connect_async_with_config(
                        &scope.endpoint,
                        Some(config),
                        false,
                    ),
                )
                .await
                .map_err(|_| error("owning provider connection timed out"))?
                .map_err(|_| error("owning provider connection failed"))?;
                authorize().await?;
                let connection_id = format!("native-socket-{:032x}", rand::random::<u128>());
                let (sender, mut outbound) = mpsc::channel::<String>(QUEUE_FRAMES);
                let (inbound, receiver) = mpsc::channel::<NativeBufferedFrame>(QUEUE_FRAMES);
                let output_budget = self.1.connection();
                let (shutdown, mut shutdown_requested) = watch::channel(false);
                let entry = Arc::new(Connection {
                    scope,
                    sender,
                    shutdown,
                    exchange: Mutex::new(NativeExchangeState::new(receiver)),
                    open_request_id: request_id,
                    output_budget: output_budget.clone(),
                    touched: std::sync::Mutex::new(std::time::Instant::now()),
                });
                entries.insert(connection_id.clone(), entry.clone());
                let weak_entries = Arc::downgrade(&self.0);
                let id = connection_id.clone();
                tokio::spawn(async move {
                    let mut socket = socket;
                    let mut interval = tokio::time::interval(Duration::from_secs(1));
                    loop {
                        tokio::select! {
                            _ = wait_for_shutdown(&mut shutdown_requested) => break,
                            _ = interval.tick() => {
                                let idle = entry.touched.lock().expect("native touch lock").elapsed() > Duration::from_secs(120);
                                if idle || authorize().await.is_err() { break; }
                            }
                            frame = outbound.recv() => {
                                let Some(frame) = frame else { break; };
                                if authorize().await.is_err() || tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Text(frame.into()))).await.map_or(true, |result| result.is_err()) { break; }
                            }
                            message = socket.next() => {
                                match message {
                                    Some(Ok(Message::Text(frame))) => {
                                        // MP-08 / MP-11: neither capacity wait may mask Close,
                                        // attachment/run revocation or the existing idle bound.
                                        let forward = async {
                                            let frame = output_budget.reserve_wait(frame.to_string()).await?;
                                            authorize().await?;
                                            tokio::time::timeout(Duration::from_secs(120), inbound.send(frame))
                                                .await.map_err(|_| error("native output queue timed out"))?
                                                .map_err(|_| error("native output receiver closed"))
                                        };
                                        tokio::pin!(forward);
                                        let forwarded = loop {
                                            tokio::select! {
                                                _ = wait_for_shutdown(&mut shutdown_requested) => break false,
                                                result = &mut forward => break result.is_ok(),
                                                _ = interval.tick() => {
                                                    let idle = entry.touched.lock().expect("native touch lock").elapsed() > Duration::from_secs(120);
                                                    if idle || authorize().await.is_err() { break false; }
                                                }
                                            }
                                        };
                                        if !forwarded { break; }
                                    }
                                    Some(Ok(Message::Ping(data))) => { if tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Pong(data))).await.map_or(true, |result| result.is_err()) { break; } }
                                    Some(Ok(Message::Pong(_))) => {}
                                    _ => break,
                                }
                            }
                        }
                    }
                    // Bounded close even when the official server stops reading.
                    let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
                    // Retain buffered output and the final receipt after upstream
                    // close. A lost final reply must remain recoverable under the
                    // same authorization and idle/byte bounds as a live socket.
                    drop(inbound);
                    drop(outbound);
                    loop {
                        tokio::select! {
                            _ = wait_for_shutdown(&mut shutdown_requested) => break,
                            _ = interval.tick() => {}
                        }
                        let Some(entries) = weak_entries.upgrade() else {
                            break;
                        };
                        if !entries.lock().await.contains_key(&id) {
                            break;
                        }
                        let idle = entry.touched.lock().expect("native touch lock").elapsed()
                            > Duration::from_secs(120);
                        if idle || authorize().await.is_err() {
                            break;
                        }
                    }
                    if let Some(entries) = weak_entries.upgrade() {
                        entries.lock().await.remove(&id);
                    }
                });
                Ok(NativeProviderTransportResult {
                    connection_id,
                    sequence: 0,
                    frames: vec![],
                    gzip_fragment: None,
                    closed: false,
                })
            }
            NativeProviderTransportOperation::Exchange {
                connection_id,
                frames,
                sequence,
            } => {
                if frames.len() > QUEUE_FRAMES
                    || frames.iter().map(String::len).sum::<usize>() > FRAME_BYTES
                {
                    return Err(error("native outbound queue exceeded"));
                }
                let mut rejected = Vec::new();
                let mut admitted = Vec::new();
                for frame in &frames {
                    // Optional native features may receive an RPC refusal without
                    // tearing down the display connection or forwarding a turn.
                    if validate_scoped_frame(&frame, &scope).is_ok() {
                        admitted.push(frame.clone());
                    } else {
                        let value: serde_json::Value = serde_json::from_str(&frame)
                            .map_err(|_| error("native frame must be JSON"))?;
                        if let Some(id) = value
                            .get("id")
                            .filter(|id| id.is_string() || id.is_number())
                        {
                            rejected.push(serde_json::json!({"id":id,"error":{"code":-32601,"message":"Use the owning kernel prompt/control or RuntimeInteraction path"}}).to_string());
                        }
                    }
                }
                if rejected.iter().map(String::len).sum::<usize>() > FRAME_BYTES {
                    return Err(error("native refusal batch exceeds the response bound"));
                }
                let entry = self
                    .0
                    .lock()
                    .await
                    .get(&connection_id)
                    .cloned()
                    .ok_or_else(|| {
                        error("native connection closed; reattach to the current run")
                    })?;
                if entry.scope != scope {
                    return Err(error("native connection attachment or run mismatch"));
                }
                let mut exchange = entry.exchange.lock().await;
                authorize().await?;
                *entry.touched.lock().expect("native touch lock") = std::time::Instant::now();
                if let Some(result) = exchange.begin(&connection_id, sequence, &frames)? {
                    return Ok(result);
                }
                let mut output = rejected
                    .into_iter()
                    .map(|data| entry.output_budget.reserve(data))
                    .collect::<Result<Vec<_>, _>>()?;
                // Reserve the whole batch before enqueueing any frame. A full
                // queue must never leave a partially repeated operation.
                if !admitted.is_empty() {
                    let permits = entry.sender.try_reserve_many(admitted.len()).map_err(|_| {
                        error("native input backpressure; reattach without replaying input")
                    })?;
                    for (permit, frame) in permits.zip(admitted) {
                        permit.send(frame);
                    }
                }
                if output.is_empty() {
                    if let Some(frame) = exchange.take_frame() {
                        output.push(frame);
                    }
                }
                // No await between enqueue/drain and retaining its receipt.
                exchange.complete(&connection_id, sequence, frames, output)
            }
            NativeProviderTransportOperation::Close { connection_id } => {
                let mut entries = self.0.lock().await;
                if let Some(entry) = entries.get(&connection_id) {
                    if entry.scope != scope {
                        return Err(error("native connection attachment or run mismatch"));
                    }
                    // Cancellation also interrupts byte/channel backpressure.
                    entry.shutdown.send_replace(true);
                    entries.remove(&connection_id);
                }
                Ok(NativeProviderTransportResult {
                    connection_id,
                    sequence: 0,
                    frames: vec![],
                    gzip_fragment: None,
                    closed: true,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests;
