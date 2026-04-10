use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use arroba_relay::protocol::{
    RelayEnvelope, RelayKernelPresence, RelayMachinePresence, RelayMetadataQuery,
};

use crate::config::DaemonConfig;
use crate::error::DaemonError;

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
    let (mut socket, _) =
        connect_async(&relay_url)
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "connect relay metadata socket",
                message: error.to_string(),
            })?;
    let request = RelayEnvelope::ClientMetadataRequest {
        request_id: format!("relay-meta-{}", std::process::id()),
        auth_token: relay_token,
        query,
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&request)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "serialize relay metadata request",
                    message: error.to_string(),
                })?
                .into(),
        ))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "write relay metadata request",
            message: error.to_string(),
        })?;
    match socket.next().await {
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
