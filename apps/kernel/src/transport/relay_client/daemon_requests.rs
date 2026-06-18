//! Inbound browser/client relay request dispatch to the kernel command router.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use arroba_relay::protocol::{EncryptedRelayPayload, RelayCallerIdentity, RelayError};
use serde::Deserialize;
use serde_json::Value;

use crate::local::LocalDaemonRequest;
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::router::CommandRouter;
use crate::runtime_transport::command_cache::{
    CommandFingerprint, CommandReservation, CommandResultCache,
};
use crate::transport::kernel_protocol::{
    map_kernel_error, KernelOutgoingFrame, KernelTransportError,
};
use crate::transport::relay_crypto;

use super::request_errors::{relay_error, relay_request_kind};

#[derive(Debug, Clone)]
pub(super) struct RelayRequestOutcome {
    pub(super) encrypted_response: Option<EncryptedRelayPayload>,
    pub(super) error: Option<RelayError>,
}

pub(super) async fn handle_daemon_request(
    router: &CommandRouter,
    command_sequence: &AtomicU64,
    caller_identity: Option<RelayCallerIdentity>,
    encrypted_request: EncryptedRelayPayload,
    command_result_cache: &Arc<CommandResultCache>,
) -> RelayRequestOutcome {
    let (request, command_id, client_public_key, daemon_private_key) = {
        let daemon_private_key = router.relay_private_key();
        let decrypted = match relay_crypto::decrypt_payload_for_private_key(
            &daemon_private_key,
            &encrypted_request,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        let request = match parse_relay_client_request(&decrypted.plaintext) {
            Ok(request) => request,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        (
            request.request,
            request.command_id,
            decrypted.sender_public_key,
            daemon_private_key,
        )
    };
    let request_kind = relay_request_kind(&request);
    crate::logging::info_with_fields(
        "daemon.relay_client",
        "relay daemon request dispatching",
        serde_json::json!({
            "request_kind": request_kind,
            "command_id": command_id,
        }),
    );
    let result = dispatch_relay_client_request(
        router,
        command_sequence,
        caller_identity,
        request,
        command_id,
        command_result_cache,
    )
    .await;
    match result {
        RelayDispatchOutcome::Response(response) => {
            crate::logging::info_with_fields(
                "daemon.relay_client",
                "relay daemon request dispatched",
                serde_json::json!({
                    "request_kind": request_kind,
                }),
            );
            let plaintext = match serde_json::to_vec(&response) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(relay_error(
                            "relay_request_failed",
                            &format!("failed to serialize relay response: {error}"),
                            false,
                        )),
                    };
                }
            };
            crate::logging::info_with_fields(
                "daemon.relay_client",
                "relay daemon response serialized",
                serde_json::json!({
                    "request_kind": request_kind,
                    "byte_len": plaintext.len(),
                }),
            );
            match relay_crypto::encrypt_payload_for_peer(
                &daemon_private_key,
                &client_public_key,
                &plaintext,
            ) {
                Ok(encrypted_response) => {
                    crate::logging::info_with_fields(
                        "daemon.relay_client",
                        "relay daemon response encrypted",
                        serde_json::json!({
                            "request_kind": request_kind,
                            "byte_len": plaintext.len(),
                        }),
                    );
                    RelayRequestOutcome {
                        encrypted_response: Some(encrypted_response),
                        error: None,
                    }
                }
                Err(error) => RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "relay_request_failed",
                        &format!("failed to encrypt relay response: {error}"),
                        false,
                    )),
                },
            }
        }
        RelayDispatchOutcome::RelayError(error) => RelayRequestOutcome {
            encrypted_response: None,
            error: Some(error),
        },
    }
}

#[derive(Debug)]
struct ParsedRelayClientRequest {
    command_id: Option<String>,
    request: LocalDaemonRequest,
}

#[derive(Debug, Deserialize)]
struct RelayClientRequestEnvelope {
    #[serde(default)]
    command_id: Option<String>,
    request: LocalDaemonRequest,
}

fn parse_relay_client_request(bytes: &[u8]) -> Result<ParsedRelayClientRequest, serde_json::Error> {
    let value = serde_json::from_slice::<Value>(bytes)?;
    if value.get("request").is_some() {
        let envelope = serde_json::from_value::<RelayClientRequestEnvelope>(value)?;
        return Ok(ParsedRelayClientRequest {
            command_id: envelope
                .command_id
                .filter(|command_id| !command_id.trim().is_empty()),
            request: envelope.request,
        });
    }
    Ok(ParsedRelayClientRequest {
        command_id: None,
        request: serde_json::from_value(value)?,
    })
}

enum RelayDispatchOutcome {
    Response(Value),
    RelayError(RelayError),
}

async fn dispatch_relay_client_request(
    router: &CommandRouter,
    command_sequence: &AtomicU64,
    caller_identity: Option<RelayCallerIdentity>,
    request: LocalDaemonRequest,
    command_id: Option<String>,
    command_result_cache: &CommandResultCache,
) -> RelayDispatchOutcome {
    let sequence = command_sequence.fetch_add(1, Ordering::Relaxed);
    let command_id = command_id.unwrap_or_else(|| {
        format!(
            "relay-client-{}-{sequence}",
            crate::session::unix_epoch_ms()
        )
    });
    let command = KernelCommand::from_local_request_with_caller(
        command_id.clone(),
        KernelCommandSource::RelayClient,
        caller_identity
            .map(KernelCaller::from_relay_identity)
            .unwrap_or_else(|| KernelCaller::for_source(&KernelCommandSource::RelayClient)),
        None,
        None,
        &request,
    );
    let fingerprint = CommandFingerprint::from_command_and_request(&command, &request);
    match command_result_cache
        .reserve(&command.command_id, &fingerprint)
        .await
    {
        CommandReservation::Wait(wait_rx) => {
            return match wait_rx.await {
                Ok(cached) => cached_relay_dispatch_outcome(cached.response, cached.error),
                Err(_) => RelayDispatchOutcome::RelayError(relay_error(
                    "duplicate_command_unavailable",
                    "original duplicate command result was unavailable",
                    true,
                )),
            };
        }
        CommandReservation::Conflict => {
            return RelayDispatchOutcome::RelayError(relay_error(
                "duplicate_command_conflict",
                &format!(
                    "command_id `{}` was already used for a different request",
                    command.command_id
                ),
                false,
            ));
        }
        CommandReservation::Dispatch => {}
    }
    let command_id = command.command_id.clone();
    let result = router.dispatch(command, request).await;
    let outgoing = match result {
        Ok(response) => KernelOutgoingFrame::Response {
            request_id: command_id.clone(),
            response: Box::new(Some(serde_json::to_value(response).unwrap_or(Value::Null))),
            error: None,
        },
        Err(error) => KernelOutgoingFrame::Response {
            request_id: command_id.clone(),
            response: Box::new(None),
            error: Some(map_kernel_error(&error)),
        },
    };
    command_result_cache
        .complete(command_id, fingerprint, &outgoing)
        .await;
    let KernelOutgoingFrame::Response {
        response, error, ..
    } = outgoing
    else {
        unreachable!("relay command dispatch only builds response frames")
    };
    cached_relay_dispatch_outcome(response, error)
}

fn cached_relay_dispatch_outcome(
    response: Box<Option<Value>>,
    error: Option<KernelTransportError>,
) -> RelayDispatchOutcome {
    if let Some(error) = error {
        return RelayDispatchOutcome::RelayError(relay_error(
            &error.code,
            &error.message,
            error.retryable,
        ));
    }
    RelayDispatchOutcome::Response((*response).unwrap_or(Value::Null))
}
