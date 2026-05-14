//! Outbound relay peer requests and pending response correlation.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

use super::*;

static TEMPORARY_PEER_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

#[allow(dead_code)]
#[derive(Debug)]
pub(super) struct RelayPeerResponseEnvelope {
    pub(super) from_daemon_id: String,
    pub(super) encrypted_response: Option<EncryptedRelayPayload>,
    pub(super) error: Option<RelayError>,
}

#[cfg(test)]
pub async fn send_peer_request_to_known_kernel_via_relay(
    config: &crate::config::DaemonConfig,
    state: &Arc<RwLock<RelayClientState>>,
    target: ClientTarget,
    target_public_key: &str,
    request: RelayPeerRequest,
) -> Result<RelayPeerResponse, DaemonError> {
    let _target_ref = target
        .daemon_id
        .as_deref()
        .or(target.daemon_alias.as_deref())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "peer target must include daemon id or alias".to_string(),
        })?;
    let plaintext = serde_json::to_vec(&request).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay peer request",
        message: error.to_string(),
    })?;
    let encrypted_request = relay_crypto::encrypt_payload_for_peer(
        &config.relay_private_key,
        target_public_key,
        &plaintext,
    )?;
    let (request_id, response_rx, outgoing_tx) = {
        let mut guard = state.write().await;
        let Some(outgoing_tx) = guard.outgoing_tx.clone() else {
            return Err(DaemonError::LocalTransport {
                operation: "send relay peer request",
                message: "relay is not connected".to_string(),
            });
        };
        guard.next_peer_request_id += 1;
        let request_id = format!("daemon-peer-{}", guard.next_peer_request_id);
        let (response_tx, response_rx) = oneshot::channel();
        guard
            .pending_peer_requests
            .insert(request_id.clone(), response_tx);
        (request_id, response_rx, outgoing_tx)
    };
    if outgoing_tx
        .send(RelayEnvelope::DaemonPeerRequest {
            request_id: request_id.clone(),
            target,
            encrypted_request,
        })
        .is_err()
    {
        let mut guard = state.write().await;
        guard.pending_peer_requests.remove(&request_id);
        return Err(DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "relay is not connected".to_string(),
        });
    }
    let envelope = match timeout(
        Duration::from_millis(config.relay_request_timeout_ms),
        response_rx,
    )
    .await
    {
        Ok(Ok(envelope)) => envelope,
        Ok(Err(_)) => {
            return Err(DaemonError::LocalTransport {
                operation: "read relay peer response",
                message: "relay peer request was cancelled".to_string(),
            });
        }
        Err(_) => {
            let mut guard = state.write().await;
            guard.pending_peer_requests.remove(&request_id);
            return Err(DaemonError::LocalTransport {
                operation: "read relay peer response",
                message: format!(
                    "timed out waiting for relay peer response after {}ms",
                    config.relay_request_timeout_ms
                ),
            });
        }
    };
    if let Some(error) = envelope.error {
        return Err(DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: error.message,
        });
    }
    let encrypted_response =
        envelope
            .encrypted_response
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "read relay peer response",
                message: format!(
                    "peer `{}` returned no response payload",
                    envelope.from_daemon_id
                ),
            })?;
    let decrypted = relay_crypto::decrypt_payload_for_private_key(
        &config.relay_private_key,
        &encrypted_response,
    )?;
    serde_json::from_slice::<RelayPeerResponse>(&decrypted.plaintext).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "decode relay peer response",
            message: error.to_string(),
        }
    })
}

#[cfg(test)]
pub async fn send_peer_request_via_relay(
    app: &Arc<Mutex<DaemonApp>>,
    state: &Arc<RwLock<RelayClientState>>,
    target: ClientTarget,
    request: RelayPeerRequest,
) -> Result<RelayPeerResponse, DaemonError> {
    let target_ref = target
        .daemon_id
        .as_deref()
        .or(target.daemon_alias.as_deref())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "peer target must include daemon id or alias".to_string(),
        })?;
    let config = {
        let app = app.lock().await;
        app.config().clone()
    };
    let kernel = relay_discovery::get_live_kernel(&config, target_ref).await?;
    send_peer_request_to_known_kernel_via_relay(&config, state, target, &kernel.public_key, request)
        .await
}

pub async fn send_peer_request_via_temporary_connection(
    config: &crate::config::DaemonConfig,
    target: ClientTarget,
    request: RelayPeerRequest,
) -> Result<RelayPeerResponse, DaemonError> {
    let target_ref = target
        .daemon_id
        .as_deref()
        .or(target.daemon_alias.as_deref())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "peer target must include daemon id or alias".to_string(),
        })?;
    let relay_url = config
        .relay_url
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "relay_url is not configured".to_string(),
        })?;
    let relay_token = config
        .relay_token
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "relay_token is not configured".to_string(),
        })?;
    let kernel = relay_discovery::get_live_kernel(config, target_ref).await?;
    let plaintext = serde_json::to_vec(&request).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay peer request",
        message: error.to_string(),
    })?;
    let encrypted_request = relay_crypto::encrypt_payload_for_peer(
        &config.relay_private_key,
        &kernel.public_key,
        &plaintext,
    )?;
    let (mut socket, _) =
        connect_async(&relay_url)
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "connect temporary relay peer socket",
                message: error.to_string(),
            })?;
    let request_id = format!(
        "daemon-peer-tmp-{}-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms(),
        TEMPORARY_PEER_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let register = RelayEnvelope::DaemonRegister {
        registration: arroba_relay::protocol::DaemonRegistration {
            auth_token: relay_token,
            daemon_id: format!("{}:peer-tmp:{}", config.daemon_id, request_id),
            machine_id: config.host_machine_id.clone(),
            machine_alias: config.host_machine_alias.clone(),
            os_name: Some(config.os_name.clone()),
            kernel_started_at_ms: crate::session::unix_epoch_ms(),
            daemon_alias: config.daemon_alias.clone(),
            kernel_alias: config.daemon_alias.clone(),
            public_key: config.relay_public_key.clone(),
            capabilities: vec!["relay_peer_transport".to_string()],
            available_providers: Vec::new(),
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
        },
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&register)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "serialize temporary relay register",
                    message: error.to_string(),
                })?
                .into(),
        ))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "write temporary relay register",
            message: error.to_string(),
        })?;
    socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonPeerRequest {
                request_id: request_id.clone(),
                target,
                encrypted_request,
            })
            .map_err(|error| DaemonError::LocalTransport {
                operation: "serialize temporary relay peer request",
                message: error.to_string(),
            })?
            .into(),
        ))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "write temporary relay peer request",
            message: error.to_string(),
        })?;
    let response = timeout(
        Duration::from_millis(config.relay_request_timeout_ms),
        async {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        let envelope =
                            serde_json::from_str::<RelayEnvelope>(&text).map_err(|error| {
                                DaemonError::LocalTransport {
                                    operation: "decode temporary relay peer response",
                                    message: error.to_string(),
                                }
                            })?;
                        if let RelayEnvelope::DaemonPeerResponse {
                            request_id: response_request_id,
                            from_daemon_id: _,
                            encrypted_response,
                            error,
                        } = envelope
                        {
                            if response_request_id != request_id {
                                continue;
                            }
                            if let Some(error) = error {
                                return Err(DaemonError::LocalTransport {
                                    operation: "read temporary relay peer response",
                                    message: error.message,
                                });
                            }
                            let encrypted_response =
                                encrypted_response.ok_or_else(|| DaemonError::LocalTransport {
                                    operation: "read temporary relay peer response",
                                    message: "peer returned no response payload".to_string(),
                                })?;
                            let decrypted = relay_crypto::decrypt_payload_for_private_key(
                                &config.relay_private_key,
                                &encrypted_response,
                            )?;
                            return serde_json::from_slice::<RelayPeerResponse>(
                                &decrypted.plaintext,
                            )
                            .map_err(|error| {
                                DaemonError::LocalTransport {
                                    operation: "decode temporary relay peer response",
                                    message: error.to_string(),
                                }
                            });
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        return Err(DaemonError::LocalTransport {
                            operation: "read temporary relay peer response",
                            message: "relay closed temporary peer connection".to_string(),
                        });
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        return Err(DaemonError::LocalTransport {
                            operation: "read temporary relay peer response",
                            message: error.to_string(),
                        });
                    }
                }
            }
        },
    )
    .await;
    response.unwrap_or_else(|_| {
        Err(DaemonError::LocalTransport {
            operation: "read temporary relay peer response",
            message: format!(
                "timed out waiting for relay peer response after {}ms",
                config.relay_request_timeout_ms
            ),
        })
    })
}

pub(super) async fn resolve_pending_peer_response(
    state: &Arc<RwLock<RelayClientState>>,
    request_id: String,
    response: RelayPeerResponseEnvelope,
) {
    let sender = state
        .write()
        .await
        .pending_peer_requests
        .remove(&request_id);
    if let Some(sender) = sender {
        let _ = sender.send(response);
    }
}
