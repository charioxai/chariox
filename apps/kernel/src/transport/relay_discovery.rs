use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use arroba_relay::protocol::{
    RelayEnvelope, RelayKernelPresence, RelayMachinePresence, RelayMetadataQuery,
};

use crate::config::DaemonConfig;
use crate::error::DaemonError;

static RELAY_METADATA_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);
const RELAY_METADATA_ATTEMPTS: usize = 3;
const RELAY_METADATA_RETRY_BASE_DELAY_MS: u64 = 250;

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

#[allow(dead_code)]
pub async fn get_live_kernel(
    config: &DaemonConfig,
    kernel_ref: &str,
) -> Result<RelayKernelPresence, DaemonError> {
    let response = query_relay(
        config,
        RelayMetadataQuery::GetLiveKernel {
            kernel_ref: kernel_ref.to_string(),
        },
    )
    .await?;
    match response {
        RelayEnvelope::ClientMetadataResponse {
            kernel: Some(kernel),
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
    let (mut socket, _) = timeout(request_timeout, connect_async(&relay_url))
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "connect relay metadata socket",
            message: format!("timed out after {}ms", config.relay_request_timeout_ms),
        })?
        .map_err(|error| DaemonError::LocalTransport {
            operation: "connect relay metadata socket",
            message: error.to_string(),
        })?;
    let request = RelayEnvelope::ClientMetadataRequest {
        request_id: format!(
            "relay-meta-{}-{}",
            std::process::id(),
            RELAY_METADATA_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed) + 1
        ),
        auth_token: relay_token,
        query,
    };
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
    .map_err(|_| DaemonError::LocalTransport {
        operation: "write relay metadata request",
        message: format!("timed out after {}ms", config.relay_request_timeout_ms),
    })?
    .map_err(|error| DaemonError::LocalTransport {
        operation: "write relay metadata request",
        message: error.to_string(),
    })?;
    match timeout(request_timeout, socket.next()).await.map_err(|_| {
        DaemonError::LocalTransport {
            operation: "read relay metadata response",
            message: format!("timed out after {}ms", config.relay_request_timeout_ms),
        }
    })? {
        Some(Ok(Message::Text(text))) => {
            serde_json::from_str::<RelayEnvelope>(&text).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "decode relay metadata response",
                    message: error.to_string(),
                }
            })
        }
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
