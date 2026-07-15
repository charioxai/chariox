//! Inbound browser/client relay request dispatch to the kernel command router.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use arroba_relay::auth::RelaySubjectKind;
use arroba_relay::protocol::{EncryptedRelayPayload, RelayCallerIdentity, RelayError};
use serde::Deserialize;
use serde_json::Value;

use crate::local::LocalDaemonRequest;
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::router::CommandRouter;
use crate::runtime::terminal_pairings::public_key_thumbprint;
use crate::runtime_transport::command_cache::{
    request_is_cacheable, CommandFingerprint, CommandReservation, CommandResultCache,
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
    if let Err(error) = validate_bound_service_sender(caller_identity.as_ref(), &encrypted_request)
    {
        return RelayRequestOutcome {
            encrypted_response: None,
            error: Some(error),
        };
    }
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
    let quiet_success_request =
        crate::runtime::command_latency::is_quiet_success_command_type(request_kind);
    if !quiet_success_request {
        crate::logging::info_with_fields(
            "daemon.relay_client",
            "relay daemon request dispatching",
            serde_json::json!({
                "request_kind": request_kind,
                "command_id": command_id,
            }),
        );
    }
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
            if !quiet_success_request {
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "relay daemon request dispatched",
                    serde_json::json!({
                        "request_kind": request_kind,
                    }),
                );
            }
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
            if !quiet_success_request {
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "relay daemon response serialized",
                    serde_json::json!({
                        "request_kind": request_kind,
                        "byte_len": plaintext.len(),
                    }),
                );
            }
            match relay_crypto::encrypt_payload_for_peer(
                &daemon_private_key,
                &client_public_key,
                &plaintext,
            ) {
                Ok(encrypted_response) => {
                    if !quiet_success_request {
                        crate::logging::info_with_fields(
                            "daemon.relay_client",
                            "relay daemon response encrypted",
                            serde_json::json!({
                                "request_kind": request_kind,
                                "byte_len": plaintext.len(),
                            }),
                        );
                    }
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

fn validate_bound_service_sender(
    caller_identity: Option<&RelayCallerIdentity>,
    encrypted_request: &EncryptedRelayPayload,
) -> Result<(), RelayError> {
    let Some(expected_thumbprint) = caller_identity
        .filter(|identity| identity.subject_kind == RelaySubjectKind::Service)
        .and_then(|identity| identity.public_key_thumbprint.as_deref())
    else {
        return Ok(());
    };
    let actual_thumbprint = public_key_thumbprint(&encrypted_request.sender_public_key);
    if actual_thumbprint != expected_thumbprint {
        return Err(relay_error(
            "unauthorized",
            "relay service sender key does not match its authenticated identity",
            false,
        ));
    }
    Ok(())
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
    let fingerprint = request_is_cacheable(&request)
        .then(|| CommandFingerprint::from_command_and_request(&command, &request));
    if let Some(fingerprint) = fingerprint.as_ref() {
        match command_result_cache
            .reserve(&command.command_id, fingerprint)
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
    if let Some(fingerprint) = fingerprint {
        command_result_cache
            .complete(command_id, fingerprint, &outgoing)
            .await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn caller_identity(
        subject_kind: RelaySubjectKind,
        public_key_thumbprint: Option<String>,
    ) -> RelayCallerIdentity {
        RelayCallerIdentity {
            realm_id: "realm-1".to_string(),
            subject: "caller-1".to_string(),
            subject_kind,
            expires_at_ms: u64::MAX,
            token_id: Some("token-1".to_string()),
            user_id: Some("user-1".to_string()),
            public_key_thumbprint,
        }
    }

    fn encrypted_request(sender_public_key: &str) -> EncryptedRelayPayload {
        EncryptedRelayPayload {
            sender_public_key: sender_public_key.to_string(),
            nonce: "nonce".to_string(),
            ciphertext: "ciphertext".to_string(),
        }
    }

    #[test]
    fn service_sender_key_must_match_bound_thumbprint() {
        let request = encrypted_request("ephemeral-service-public-key");
        let identity = caller_identity(
            RelaySubjectKind::Service,
            Some(public_key_thumbprint(&request.sender_public_key)),
        );

        assert!(validate_bound_service_sender(Some(&identity), &request).is_ok());
    }

    #[test]
    fn mismatched_service_sender_key_is_rejected_before_dispatch() {
        let identity = caller_identity(
            RelaySubjectKind::Service,
            Some(public_key_thumbprint("different-public-key")),
        );
        let error = validate_bound_service_sender(
            Some(&identity),
            &encrypted_request("ephemeral-service-public-key"),
        )
        .expect_err("a stolen service token must not act as an unbound bearer token");

        assert_eq!(error.code, "unauthorized");
        assert!(!error.retryable);
    }

    #[test]
    fn ordinary_client_identity_is_not_subject_to_service_key_binding() {
        let identity = caller_identity(
            RelaySubjectKind::Client,
            Some(public_key_thumbprint("paired-client-public-key")),
        );

        assert!(validate_bound_service_sender(
            Some(&identity),
            &encrypted_request("per-request-client-public-key"),
        )
        .is_ok());
    }
}
