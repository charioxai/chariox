use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[cfg(test)]
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    net::SocketAddr,
    sync::{Mutex, OnceLock},
};

use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use chariox_relay::protocol::{
    RelayEnvelope, RelayKernelPresence, RelayMachinePresence, RelayMetadataQuery,
};

use crate::config::DaemonConfig;
use crate::error::DaemonError;

/// Prepare a metadata-only admission using the existing paired Cloud path.
/// Keep the kernel's runtime token and Cloud profile out of the returned
/// temporary config so callers can reuse this admission for one inventory scan.
pub(crate) async fn metadata_config(config: &DaemonConfig) -> Result<DaemonConfig, DaemonError> {
    let mut discovery = config.clone();
    if let Some(profile) = config.cloud_relay.as_ref() {
        let issued = crate::runtime::cloud_api_client::issue_cloud_relay_inventory_discovery_token(
            profile,
            &config.daemon_id,
        )
        .await?;
        discovery.relay_token = Some(issued.token);
        discovery.cloud_relay = None;
    }
    Ok(discovery)
}

static RELAY_METADATA_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);
const RELAY_METADATA_ATTEMPTS: usize = 3;
const RELAY_METADATA_RETRY_BASE_DELAY_MS: u64 = 250;
const RELAY_METADATA_CLOSE_TIMEOUT: Duration = Duration::from_millis(250);

#[cfg(test)]
const MAX_RELAY_DISCOVERY_TEST_TRACE_EVENTS: usize = 4096;

#[cfg(test)]
static NEXT_RELAY_DISCOVERY_TEST_CALL_ID: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
static NEXT_RELAY_DISCOVERY_TEST_ATTEMPT_ID: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
static NEXT_RELAY_DISCOVERY_TEST_TRACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static RELAY_DISCOVERY_TEST_TRACE_EVENTS: OnceLock<Mutex<VecDeque<RelayDiscoveryTestTraceEvent>>> =
    OnceLock::new();

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RelayDiscoveryTestTraceIdentity {
    relay_endpoint_hash: u64,
    peer_call_id: u64,
    attempt_id: u64,
    attempt_number: usize,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
struct RelayDiscoveryTestTraceEvent {
    sequence: u64,
    identity: RelayDiscoveryTestTraceIdentity,
    stage: &'static str,
    local_addr: Option<SocketAddr>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct RelayDiscoveryTestTraceCheckpoint {
    relay_endpoint_hash: u64,
    sequence: u64,
}

#[cfg(test)]
pub(crate) struct TemporaryPeerTestTrace {
    identity: RelayDiscoveryTestTraceIdentity,
    drop_stage: Option<&'static str>,
    local_addr: Option<SocketAddr>,
}

#[cfg(test)]
impl TemporaryPeerTestTrace {
    pub(crate) fn new(relay_url: Option<&str>) -> Self {
        let identity = RelayDiscoveryTestTraceIdentity {
            relay_endpoint_hash: relay_url.map(relay_endpoint_hash).unwrap_or_default(),
            peer_call_id: NEXT_RELAY_DISCOVERY_TEST_CALL_ID.fetch_add(1, Ordering::Relaxed),
            attempt_id: 0,
            attempt_number: 0,
        };
        let mut trace = Self::scope(identity, "temporary_peer_future_dropped");
        trace.record("temporary_peer_call_started", None);
        trace
    }

    fn attempt(identity: RelayDiscoveryTestTraceIdentity) -> Self {
        let mut trace = Self::scope(identity, "discovery_future_dropped");
        trace.record("discovery_attempt_started", None);
        trace
    }

    fn socket(identity: RelayDiscoveryTestTraceIdentity) -> Self {
        Self::scope(identity, "discovery_socket_dropped")
    }

    fn scope(identity: RelayDiscoveryTestTraceIdentity, drop_stage: &'static str) -> Self {
        Self {
            identity,
            drop_stage: Some(drop_stage),
            local_addr: None,
        }
    }

    fn set_local_addr(&mut self, local_addr: Option<SocketAddr>) {
        self.local_addr = local_addr;
    }

    pub(crate) fn finish(&mut self, stage: &'static str) {
        self.record(stage, None);
        self.drop_stage = None;
    }

    pub(crate) fn peer_call_id(&self) -> u64 {
        self.identity.peer_call_id
    }

    pub(crate) fn record(&mut self, stage: &'static str, local_addr: Option<SocketAddr>) {
        record_relay_discovery_test_trace(self.identity, stage, local_addr);
    }
}

#[cfg(test)]
impl Drop for TemporaryPeerTestTrace {
    fn drop(&mut self) {
        if let Some(stage) = self.drop_stage {
            if stage != "discovery_socket_dropped" || self.local_addr.is_some() {
                record_relay_discovery_test_trace(self.identity, stage, self.local_addr);
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn relay_discovery_test_trace_checkpoint(
    relay_url: &str,
) -> RelayDiscoveryTestTraceCheckpoint {
    let events = RELAY_DISCOVERY_TEST_TRACE_EVENTS.get_or_init(|| Mutex::new(VecDeque::new()));
    let _events = events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    RelayDiscoveryTestTraceCheckpoint {
        relay_endpoint_hash: relay_endpoint_hash(relay_url),
        sequence: NEXT_RELAY_DISCOVERY_TEST_TRACE_SEQUENCE.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
pub(crate) fn take_relay_discovery_test_trace(
    checkpoint: RelayDiscoveryTestTraceCheckpoint,
    accepted_peer: Option<SocketAddr>,
) -> String {
    let events = RELAY_DISCOVERY_TEST_TRACE_EVENTS.get_or_init(|| Mutex::new(VecDeque::new()));
    let mut events = events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let accepted_attempts = events
        .iter()
        .filter(|event| {
            event.identity.relay_endpoint_hash == checkpoint.relay_endpoint_hash
                && accepted_peer.is_some_and(|peer| event.local_addr == Some(peer))
        })
        .map(|event| event.identity)
        .collect::<Vec<_>>();
    let prior_non_success_attempts = events
        .iter()
        .filter(|event| {
            event.identity.relay_endpoint_hash == checkpoint.relay_endpoint_hash
                && event.identity.attempt_id != 0
                && event.sequence < checkpoint.sequence
                && event.stage == "discovery_connect_started"
        })
        .map(|event| event.identity)
        .filter(|identity| {
            !events.iter().any(|event| {
                event.identity == *identity && event.stage == "discovery_attempt_returned_ok"
            })
        })
        .collect::<Vec<_>>();
    let relevant_attempts = accepted_attempts
        .iter()
        .chain(prior_non_success_attempts.iter())
        .copied()
        .collect::<Vec<_>>();
    let selected = events
        .iter()
        .filter(|event| {
            event.identity.relay_endpoint_hash == checkpoint.relay_endpoint_hash
                && (relevant_attempts.contains(&event.identity)
                    || event.sequence >= checkpoint.sequence)
        })
        .copied()
        .collect::<Vec<_>>();
    let mut rendered = selected
        .iter()
        .map(|event| {
            let origin = if event.sequence < checkpoint.sequence {
                "before-checkpoint"
            } else {
                "after-checkpoint"
            };
            format!(
                "seq={},origin={origin},call={},attempt_number={},attempt_id={},stage={},local={:?}",
                event.sequence,
                event.identity.peer_call_id,
                event.identity.attempt_number,
                event.identity.attempt_id,
                event.stage,
                event.local_addr,
            )
        })
        .collect::<Vec<_>>();
    if rendered.len() > 64 {
        let excess = rendered.len() - 64;
        rendered.drain(..excess);
    }
    events.retain(|event| {
        !(event.identity.relay_endpoint_hash == checkpoint.relay_endpoint_hash
            && (relevant_attempts.contains(&event.identity)
                || event.sequence >= checkpoint.sequence))
    });
    let correlation = if accepted_attempts.is_empty() {
        "accepted socket did not match a client-local address"
    } else {
        "accepted socket matched client-local address"
    };
    format!(
        "checkpoint_seq={},accepted_peer={accepted_peer:?},correlation={correlation},events=[{}]",
        checkpoint.sequence,
        rendered.join(";"),
    )
}

#[cfg(test)]
pub(crate) fn relay_discovery_test_local_addr(
    socket: &tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Option<SocketAddr> {
    match socket.get_ref() {
        tokio_tungstenite::MaybeTlsStream::Plain(stream) => stream.local_addr().ok(),
        _ => None,
    }
}

#[cfg(test)]
fn relay_endpoint_hash(relay_url: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    relay_url.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
fn record_relay_discovery_test_trace(
    identity: RelayDiscoveryTestTraceIdentity,
    stage: &'static str,
    local_addr: Option<SocketAddr>,
) {
    let events = RELAY_DISCOVERY_TEST_TRACE_EVENTS.get_or_init(|| Mutex::new(VecDeque::new()));
    let mut events = events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if events.len() == MAX_RELAY_DISCOVERY_TEST_TRACE_EVENTS {
        events.pop_front();
    }
    events.push_back(RelayDiscoveryTestTraceEvent {
        sequence: NEXT_RELAY_DISCOVERY_TEST_TRACE_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        identity,
        stage,
        local_addr,
    });
}

pub async fn list_live_machines(
    config: &DaemonConfig,
) -> Result<Vec<RelayMachinePresence>, DaemonError> {
    let response = query_relay(config, RelayMetadataQuery::ListLiveMachines).await?;
    match response {
        RelayEnvelope::ClientMetadataResponse {
            machines: Some(machines),
            error: None,
            ..
        } => Ok(machines),
        RelayEnvelope::ClientMetadataResponse {
            error: Some(error), ..
        } => Err(DaemonError::LocalTransport {
            operation: "list_remote_machines",
            message: error.message,
        }),
        other => Err(DaemonError::LocalTransport {
            operation: "list_remote_machines",
            message: format!("unexpected relay response: {other:?}"),
        }),
    }
}

pub async fn list_live_kernels_for_machine(
    config: &DaemonConfig,
    machine_ref: &str,
) -> Result<Vec<RelayKernelPresence>, DaemonError> {
    let response = query_relay(
        config,
        RelayMetadataQuery::ListLiveKernelsForMachine {
            machine_ref: machine_ref.to_string(),
        },
    )
    .await?;
    match response {
        RelayEnvelope::ClientMetadataResponse {
            kernels: Some(kernels),
            error: None,
            ..
        } => Ok(kernels),
        RelayEnvelope::ClientMetadataResponse {
            error: Some(error), ..
        } => Err(DaemonError::LocalTransport {
            operation: "list_remote_machine_kernels",
            message: error.message,
        }),
        other => Err(DaemonError::LocalTransport {
            operation: "list_remote_machine_kernels",
            message: format!("unexpected relay response: {other:?}"),
        }),
    }
}

pub async fn get_live_kernel(
    config: &DaemonConfig,
    kernel_ref: &str,
) -> Result<RelayKernelPresence, DaemonError> {
    #[cfg(test)]
    {
        let peer_call_id = NEXT_RELAY_DISCOVERY_TEST_CALL_ID.fetch_add(1, Ordering::Relaxed);
        return get_live_kernel_inner(config, kernel_ref, Some(peer_call_id)).await;
    }
    #[cfg(not(test))]
    get_live_kernel_inner(config, kernel_ref).await
}

#[cfg(test)]
pub(crate) async fn get_live_kernel_for_temporary_peer_test_call(
    config: &DaemonConfig,
    kernel_ref: &str,
    peer_call_id: u64,
) -> Result<RelayKernelPresence, DaemonError> {
    get_live_kernel_inner(config, kernel_ref, Some(peer_call_id)).await
}

async fn get_live_kernel_inner(
    config: &DaemonConfig,
    kernel_ref: &str,
    #[cfg(test)] peer_call_id: Option<u64>,
) -> Result<RelayKernelPresence, DaemonError> {
    #[cfg(test)]
    let peer_call_id = peer_call_id
        .unwrap_or_else(|| NEXT_RELAY_DISCOVERY_TEST_CALL_ID.fetch_add(1, Ordering::Relaxed));
    let mut last_error = None;
    for attempt in 0..RELAY_METADATA_ATTEMPTS {
        #[cfg(test)]
        let result = {
            let identity = RelayDiscoveryTestTraceIdentity {
                relay_endpoint_hash: config
                    .relay_url
                    .as_deref()
                    .map(relay_endpoint_hash)
                    .unwrap_or_default(),
                peer_call_id,
                attempt_id: NEXT_RELAY_DISCOVERY_TEST_ATTEMPT_ID.fetch_add(1, Ordering::Relaxed),
                attempt_number: attempt + 1,
            };
            let mut trace = TemporaryPeerTestTrace::attempt(identity);
            let result =
                find_live_kernel_once_with_test_trace(config, kernel_ref, &mut trace).await;
            trace.finish(if result.is_ok() {
                "discovery_attempt_returned_ok"
            } else {
                "discovery_attempt_returned_error"
            });
            result
        };
        #[cfg(not(test))]
        let result = find_live_kernel_once(config, kernel_ref).await;
        match result {
            Ok(Some(kernel)) => return Ok(kernel),
            Ok(None) => {
                last_error = Some(DaemonError::LocalTransport {
                    operation: "get_live_kernel",
                    message: format!("kernel `{kernel_ref}` is not currently visible on relay"),
                });
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if attempt + 1 < RELAY_METADATA_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(
                RELAY_METADATA_RETRY_BASE_DELAY_MS * (attempt as u64 + 1),
            ))
            .await;
        }
    }
    Err(last_error.unwrap_or_else(|| DaemonError::LocalTransport {
        operation: "get_live_kernel",
        message: format!("kernel `{kernel_ref}` did not appear on relay"),
    }))
}

pub(crate) async fn find_live_kernel_once(
    config: &DaemonConfig,
    kernel_ref: &str,
) -> Result<Option<RelayKernelPresence>, DaemonError> {
    let response = query_relay_once(
        config,
        RelayMetadataQuery::GetLiveKernel {
            kernel_ref: kernel_ref.to_string(),
        },
    )
    .await?;
    match response {
        RelayEnvelope::ClientMetadataResponse {
            kernel,
            error: None,
            ..
        } => Ok(kernel),
        RelayEnvelope::ClientMetadataResponse {
            error: Some(error), ..
        } => Err(DaemonError::LocalTransport {
            operation: "get_live_kernel",
            message: error.message,
        }),
        other => Err(DaemonError::LocalTransport {
            operation: "get_live_kernel",
            message: format!("unexpected relay response: {other:?}"),
        }),
    }
}

#[cfg(test)]
async fn find_live_kernel_once_with_test_trace(
    config: &DaemonConfig,
    kernel_ref: &str,
    trace: &mut TemporaryPeerTestTrace,
) -> Result<Option<RelayKernelPresence>, DaemonError> {
    let response = query_relay_once_with_test_trace(
        config,
        RelayMetadataQuery::GetLiveKernel {
            kernel_ref: kernel_ref.to_string(),
        },
        trace,
    )
    .await?;
    match response {
        RelayEnvelope::ClientMetadataResponse {
            kernel,
            error: None,
            ..
        } => Ok(kernel),
        RelayEnvelope::ClientMetadataResponse {
            error: Some(error), ..
        } => Err(DaemonError::LocalTransport {
            operation: "get_live_kernel",
            message: error.message,
        }),
        other => Err(DaemonError::LocalTransport {
            operation: "get_live_kernel",
            message: format!("unexpected relay response: {other:?}"),
        }),
    }
}

async fn query_relay(
    config: &DaemonConfig,
    query: RelayMetadataQuery,
) -> Result<RelayEnvelope, DaemonError> {
    let mut last_error = None;
    for attempt in 0..RELAY_METADATA_ATTEMPTS {
        match query_relay_once(config, query.clone()).await {
            Ok(response) => return Ok(response),
            Err(error) if relay_metadata_error_is_retryable(&error) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_discovery",
                    "relay metadata query attempt failed",
                    serde_json::json!({
                        "attempt": attempt + 1,
                        "max_attempts": RELAY_METADATA_ATTEMPTS,
                        "error": error.to_string(),
                    }),
                );
                last_error = Some(error);
                if attempt + 1 < RELAY_METADATA_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(
                        RELAY_METADATA_RETRY_BASE_DELAY_MS
                            * u64::try_from(attempt + 1).unwrap_or(1),
                    ))
                    .await;
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| DaemonError::LocalTransport {
        operation: "relay_metadata_query",
        message: "relay metadata query failed without an error".to_string(),
    }))
}

async fn query_relay_once(
    config: &DaemonConfig,
    query: RelayMetadataQuery,
) -> Result<RelayEnvelope, DaemonError> {
    #[cfg(test)]
    return query_relay_once_inner(config, query, None).await;
    #[cfg(not(test))]
    query_relay_once_inner(config, query).await
}

#[cfg(test)]
async fn query_relay_once_with_test_trace(
    config: &DaemonConfig,
    query: RelayMetadataQuery,
    trace: &mut TemporaryPeerTestTrace,
) -> Result<RelayEnvelope, DaemonError> {
    query_relay_once_inner(config, query, Some(trace)).await
}

async fn query_relay_once_inner(
    config: &DaemonConfig,
    query: RelayMetadataQuery,
    #[cfg(test)] mut trace: Option<&mut TemporaryPeerTestTrace>,
) -> Result<RelayEnvelope, DaemonError> {
    let discovery_config = metadata_config(config).await?;
    let config = &discovery_config;
    let relay_url = config
        .relay_url
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "relay_metadata_query",
            message: "relay_url is not configured".to_string(),
        })?;
    let relay_token = config
        .relay_token
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "relay_metadata_query",
            message: "relay_token is not configured".to_string(),
        })?;
    let request_timeout = Duration::from_millis(config.relay_request_timeout_ms);
    #[cfg(test)]
    if let Some(trace) = trace.as_deref_mut() {
        trace.record("discovery_connect_started", None);
    }
    #[cfg(test)]
    let mut socket_trace = trace
        .as_deref()
        .map(|trace| TemporaryPeerTestTrace::socket(trace.identity));
    let (mut socket, _) = timeout(request_timeout, connect_async(&relay_url))
        .await
        .map_err(|_| {
            #[cfg(test)]
            if let Some(trace) = trace.as_deref_mut() {
                trace.record("discovery_connect_timed_out", None);
                trace.record("discovery_connect_future_cancelled", None);
            }
            DaemonError::LocalTransport {
                operation: "connect relay metadata socket",
                message: format!("timed out after {}ms", config.relay_request_timeout_ms),
            }
        })?
        .map_err(|error| {
            #[cfg(test)]
            if let Some(trace) = trace.as_deref_mut() {
                trace.record("discovery_connect_failed", None);
            }
            DaemonError::LocalTransport {
                operation: "connect relay metadata socket",
                message: error.to_string(),
            }
        })?;
    #[cfg(test)]
    {
        let local_addr = relay_discovery_test_local_addr(&socket);
        if let Some(trace) = trace.as_deref_mut() {
            trace.record("discovery_connect_completed", local_addr);
        }
        if let Some(socket_trace) = socket_trace.as_mut() {
            socket_trace.set_local_addr(local_addr);
        }
    }
    let request_id = format!(
        "relay-meta-{}-{}",
        std::process::id(),
        RELAY_METADATA_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed) + 1
    );
    let request = RelayEnvelope::ClientMetadataRequest {
        request_id: request_id.clone(),
        auth_token: relay_token,
        query,
    };
    let response = async {
        #[cfg(test)]
        if let Some(trace) = trace.as_deref_mut() {
            trace.record(
                "metadata_write_started",
                socket_trace.as_ref().and_then(|trace| trace.local_addr),
            );
        }
        timeout(
            request_timeout,
            socket.send(Message::Text(
                serde_json::to_string(&request)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "serialize relay metadata request",
                        message: error.to_string(),
                    })?
                    .into(),
            )),
        )
        .await
        .map_err(|_| {
            #[cfg(test)]
            if let Some(trace) = trace.as_deref_mut() {
                trace.record(
                    "metadata_write_timed_out",
                    socket_trace.as_ref().and_then(|trace| trace.local_addr),
                );
            }
            DaemonError::LocalTransport {
                operation: "write relay metadata request",
                message: format!("timed out after {}ms", config.relay_request_timeout_ms),
            }
        })?
        .map_err(|error| {
            #[cfg(test)]
            if let Some(trace) = trace.as_deref_mut() {
                trace.record(
                    "metadata_write_failed",
                    socket_trace.as_ref().and_then(|trace| trace.local_addr),
                );
            }
            DaemonError::LocalTransport {
                operation: "write relay metadata request",
                message: error.to_string(),
            }
        })?;
        #[cfg(test)]
        if let Some(trace) = trace.as_deref_mut() {
            trace.record(
                "metadata_write_succeeded",
                socket_trace.as_ref().and_then(|trace| trace.local_addr),
            );
        }
        match timeout(request_timeout, socket.next()).await.map_err(|_| {
            DaemonError::LocalTransport {
                operation: "read relay metadata response",
                message: format!("timed out after {}ms", config.relay_request_timeout_ms),
            }
        })? {
            Some(Ok(Message::Text(text))) => serde_json::from_str::<RelayEnvelope>(&text)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "decode relay metadata response",
                    message: error.to_string(),
                })
                .and_then(|response| match &response {
                    RelayEnvelope::ClientMetadataResponse {
                        request_id: response_id,
                        ..
                    } if response_id == &request_id => Ok(response),
                    _ => Err(DaemonError::LocalTransport {
                        operation: "decode relay metadata response",
                        message: "relay metadata response does not match request".to_string(),
                    }),
                }),
            Some(Ok(Message::Close(_))) | None => Err(DaemonError::LocalTransport {
                operation: "read relay metadata response",
                message: "relay closed metadata connection".to_string(),
            }),
            Some(Ok(_)) => Err(DaemonError::LocalTransport {
                operation: "read relay metadata response",
                message: "relay returned a non-text metadata frame".to_string(),
            }),
            Some(Err(error)) => Err(DaemonError::LocalTransport {
                operation: "read relay metadata response",
                message: error.to_string(),
            }),
        }
    }
    .await;
    #[cfg(test)]
    if let Some(trace) = trace.as_deref_mut() {
        trace.record(
            if response.is_ok() {
                "metadata_response_received"
            } else {
                "metadata_response_failed"
            },
            socket_trace.as_ref().and_then(|trace| trace.local_addr),
        );
        trace.record(
            "discovery_close_started",
            socket_trace.as_ref().and_then(|trace| trace.local_addr),
        );
    }
    let _ = socket.close(None).await;
    let _ = timeout(RELAY_METADATA_CLOSE_TIMEOUT, async {
        while let Some(message) = socket.next().await {
            match message {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    })
    .await;
    #[cfg(test)]
    if let Some(trace) = trace {
        trace.record(
            "discovery_close_drained",
            socket_trace.as_ref().and_then(|trace| trace.local_addr),
        );
    }
    response
}

fn relay_metadata_error_is_retryable(error: &DaemonError) -> bool {
    match error {
        DaemonError::LocalTransport { operation, message } => {
            matches!(
                *operation,
                "connect relay metadata socket"
                    | "write relay metadata request"
                    | "read relay metadata response"
            ) && (message.contains("timed out")
                || message.contains("Operation timed out")
                || message.contains("Connection reset")
                || message.contains("connection closed")
                || message.contains("relay closed metadata connection"))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    #[tokio::test]
    async fn hosted_metadata_does_not_fall_back_to_runtime_admission() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://127.0.0.1:1".into());
        config.relay_token = Some("synthetic-kernel-runtime-token".into());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            machine_id: Some("paired-machine".into()),
            ..Default::default()
        });
        let error = query_relay_once(&config, RelayMetadataQuery::ListLiveMachines)
            .await
            .expect_err("failed scoped discovery must reject before opening a relay socket");
        assert!(matches!(
            error,
            DaemonError::LocalTransport {
                operation: "issue cloud relay inventory discovery token",
                ..
            }
        ));
        assert_eq!(
            config.relay_token.as_deref(),
            Some("synthetic-kernel-runtime-token")
        );
        assert!(
            config.cloud_relay.is_some(),
            "runtime admission must remain intact"
        );
    }

    #[tokio::test]
    async fn metadata_query_rejects_response_for_another_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let request = match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    serde_json::from_str::<RelayEnvelope>(&text).unwrap()
                }
                other => panic!("expected metadata request, received {other:?}"),
            };
            assert!(matches!(
                request,
                RelayEnvelope::ClientMetadataRequest { .. }
            ));
            socket
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::ClientMetadataResponse {
                        request_id: "another-request".to_string(),
                        machines: Some(Vec::new()),
                        kernels: None,
                        kernel: None,
                        error: None,
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            let _ = timeout(Duration::from_secs(1), socket.next()).await;
        });
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{addr}"));
        config.relay_token = Some("metadata-fixture-token".to_string());
        config.relay_request_timeout_ms = 1_000;
        let response = query_relay_once(&config, RelayMetadataQuery::ListLiveMachines).await;
        server.await.unwrap();
        assert!(
            response.is_err(),
            "uncorrelated metadata must not become inventory evidence"
        );
    }

    #[tokio::test]
    async fn metadata_query_closes_websocket_after_response() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test websocket should bind");
        let addr = listener
            .local_addr()
            .expect("test websocket should have an address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener
                .accept()
                .await
                .expect("metadata client should connect");
            let mut socket = accept_async(stream)
                .await
                .expect("metadata websocket should accept");
            let request = match socket.next().await {
                Some(Ok(Message::Text(text))) => serde_json::from_str::<RelayEnvelope>(&text)
                    .expect("metadata request should decode"),
                other => panic!("expected metadata request, received {other:?}"),
            };
            let RelayEnvelope::ClientMetadataRequest { request_id, .. } = request else {
                panic!("expected metadata request envelope");
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::ClientMetadataResponse {
                        request_id,
                        machines: Some(Vec::new()),
                        kernels: None,
                        kernel: None,
                        error: None,
                    })
                    .expect("metadata response should encode")
                    .into(),
                ))
                .await
                .expect("metadata response should send");

            let close = timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("metadata client should close promptly");
            assert!(
                matches!(close, Some(Ok(Message::Close(_)))),
                "metadata client should complete a websocket close handshake, received {close:?}"
            );
        });

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{addr}"));
        config.relay_token = Some("metadata-test-token".to_string());
        config.relay_request_timeout_ms = 1_000;

        let response = query_relay_once(&config, RelayMetadataQuery::ListLiveMachines)
            .await
            .expect("metadata query should succeed");
        assert!(matches!(
            response,
            RelayEnvelope::ClientMetadataResponse {
                machines: Some(machines),
                ..
            } if machines.is_empty()
        ));
        server
            .await
            .expect("metadata websocket server should finish");
    }
}
