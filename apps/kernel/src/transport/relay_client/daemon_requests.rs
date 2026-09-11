//! Inbound browser/client relay request dispatch to the kernel command router.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chariox_relay::protocol::{EncryptedRelayPayload, RelayCallerIdentity, RelayError};
use serde::Deserialize;
use serde_json::Value;

use crate::local::LocalDaemonRequest;
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::router::CommandRouter;
use crate::runtime_transport::command_cache::{
    request_is_cacheable, CommandFingerprint, CommandReservation, CommandResultCache,
};
use crate::transport::kernel_protocol::{
    map_kernel_error, BrowserImportRelayResponse, KernelOutgoingFrame, KernelTransportError,
};
use crate::transport::relay_crypto;

use super::request_errors::{relay_error, relay_request_kind};
use super::sender_identity::{
    is_browser_import_request, require_browser_import_sender, validate_bound_service_sender,
    validate_browser_import_sender,
};

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
    let (message, client_public_key, daemon_private_key) = {
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
        (request, decrypted.sender_public_key, daemon_private_key)
    };
    let (request_kind, command_id, bind_import_response, result) = match message {
        ParsedRelayClientMessage::Request(request) => {
            if let Err(error) = validate_browser_import_sender(
                &request.request,
                caller_identity.as_ref(),
                &encrypted_request,
            ) {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(error),
                };
            }
            let request_kind = relay_request_kind(&request.request);
            let bind_import_response = is_browser_import_request(&request.request);
            let result = dispatch_relay_client_request(
                router,
                command_sequence,
                caller_identity,
                request.request,
                request.command_id.clone(),
                command_result_cache,
            )
            .await;
            (
                request_kind,
                request.command_id,
                bind_import_response,
                result,
            )
        }
        ParsedRelayClientMessage::BrowserImportDelivery(request) => {
            let identity =
                match require_browser_import_sender(caller_identity.as_ref(), &encrypted_request) {
                    Ok(identity) => identity.clone(),
                    Err(error) => {
                        return RelayRequestOutcome {
                            encrypted_response: None,
                            error: Some(error),
                        }
                    }
                };
            let command_id = format!(
                "browser-import-delivery-{}-{}",
                crate::session::unix_epoch_ms(),
                command_sequence.fetch_add(1, Ordering::Relaxed)
            );
            let selection = request.selection.clone();
            let metadata_request = LocalDaemonRequest::AuthorizeBrowserImportSource(
                crate::local::BrowserImportSourceRequest {
                    request_id: request.request_id.clone(),
                    selection,
                },
            );
            let command = KernelCommand::from_local_request_with_caller(
                command_id.clone(),
                KernelCommandSource::RelayClient,
                KernelCaller::from_relay_identity(identity),
                None,
                None,
                &metadata_request,
            );
            let result = router
                .runtime_state()
                .execute_browser_import_delivery(&command, request)
                .await
                .map(|cookie_count| {
                    RelayDispatchOutcome::Response(serde_json::json!({
                        "BrowserImportDelivered": {"cookie_count": cookie_count}
                    }))
                })
                .unwrap_or_else(|_| {
                    RelayDispatchOutcome::RelayError(relay_error(
                        "browser_import_failed",
                        "browser import failed or requires recovery",
                        false,
                    ))
                });
            ("browser_import_delivery", Some(command_id), true, result)
        }
    };
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
            let serialized = if bind_import_response {
                serde_json::to_vec(&BrowserImportRelayResponse {
                    request_nonce: encrypted_request.nonce.clone(),
                    response,
                })
            } else {
                serde_json::to_vec(&response)
            };
            let plaintext = match serialized {
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

#[derive(Debug)]
enum ParsedRelayClientMessage {
    Request(ParsedRelayClientRequest),
    BrowserImportDelivery(crate::runtime::browser_import_payload::BrowserImportDeliveryRequest),
}

#[derive(Debug)]
struct ParsedRelayClientRequest {
    command_id: Option<String>,
    request: LocalDaemonRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelayClientRequestEnvelope {
    #[serde(default)]
    command_id: Option<String>,
    request: LocalDaemonRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserImportDeliveryEnvelope {
    browser_import_delivery: crate::runtime::browser_import_payload::BrowserImportDeliveryRequest,
}

fn parse_relay_client_request(bytes: &[u8]) -> Result<ParsedRelayClientMessage, serde_json::Error> {
    if let Ok(envelope) = serde_json::from_slice::<BrowserImportDeliveryEnvelope>(bytes) {
        return Ok(ParsedRelayClientMessage::BrowserImportDelivery(
            envelope.browser_import_delivery,
        ));
    }
    if let Ok(envelope) = serde_json::from_slice::<RelayClientRequestEnvelope>(bytes) {
        return Ok(ParsedRelayClientMessage::Request(
            ParsedRelayClientRequest {
                command_id: envelope
                    .command_id
                    .filter(|command_id| !command_id.trim().is_empty()),
                request: envelope.request,
            },
        ));
    }
    Ok(ParsedRelayClientMessage::Request(
        ParsedRelayClientRequest {
            command_id: None,
            request: serde_json::from_slice(bytes)?,
        },
    ))
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
        Ok(response) => {
            let mut response = serde_json::to_value(response).unwrap_or(Value::Null);
            crate::local::redact_client_response_value(&mut response);
            KernelOutgoingFrame::Response {
                request_id: command_id.clone(),
                response: Box::new(Some(response)),
                error: None,
            }
        }
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
#[path = "daemon_requests_browser_import_tests.rs"]
mod browser_import_tests;

#[cfg(test)]
mod tests {
    use crate::agent::{AgentInstance, GridPosition, RemoteAgentBinding};
    use crate::local::LocalDaemonResponse;
    use base64::Engine;

    #[test]
    fn encrypted_delivery_parser_keeps_cookie_payload_out_of_daemon_requests_and_debug() {
        let generated_value = format!("generated-{}", crate::session::unix_epoch_ms());
        let payload = serde_json::json!([{
            "name":"session", "value":generated_value, "domain":"example.test", "path":"/"
        }])
        .to_string();
        let envelope = serde_json::json!({
            "browser_import_delivery": {
                "request_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "selection": {
                    "session_id":"room-1", "attachment_id":"attachment-1",
                    "environment_id":"environment-1", "runtime_generation":1,
                    "tab_id":"tab-1", "document_revision":1, "source_store_id":"normal",
                    "domains":["example.test"], "partition_sites":[], "overwrite":false
                },
                "payload_base64":base64::engine::general_purpose::STANDARD.encode(payload.as_bytes())
            }
        });
        let parsed = super::parse_relay_client_request(envelope.to_string().as_bytes()).unwrap();
        let super::ParsedRelayClientMessage::BrowserImportDelivery(delivery) = parsed else {
            panic!("delivery must not parse as a local daemon request")
        };
        assert!(!format!("{delivery:?}").contains(&generated_value));
        let (_, _, decoded) = delivery.into_parts();
        assert!(decoded.as_str().contains(&generated_value));

        let mut public_request = envelope["browser_import_delivery"].clone();
        public_request["selection"]["cookies"] = serde_json::json!([{}]);
        assert!(super::parse_relay_client_request(
            serde_json::json!({"request":public_request})
                .to_string()
                .as_bytes()
        )
        .is_err());
    }

    #[test]
    fn relay_client_response_projection_redacts_remote_relay_token() {
        let mut agent = AgentInstance::new(
            "agent-1",
            "agent-ref-1",
            "session-1",
            Some("reviewer".to_string()),
            "codex",
            Some("gpt-5.6-sol".to_string()),
            Some("high".to_string()),
            None,
            GridPosition {
                row: 0,
                col: 0,
                row_span: 1,
                col_span: 1,
            },
        );
        agent.set_remote_execution(Some(RemoteAgentBinding {
            worker_kernel_id: "worker-1".to_string(),
            worker_machine_id: "machine-1".to_string(),
            execution_lease_id: "lease-1".to_string(),
            leased_agent_id: "agent-1".to_string(),
            active_worker_provider_run_id: None,
            relay_url: Some("wss://relay.example".to_string()),
            relay_token: Some("secret-token".to_string()),
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        }));

        let mut response = serde_json::to_value(LocalDaemonResponse::AgentMovedToRemote { agent })
            .expect("response should serialize");
        crate::local::redact_client_response_value(&mut response);

        assert!(response
            .pointer("/AgentMovedToRemote/agent/remote_execution/relay_token")
            .is_none());
        assert_eq!(
            response
                .pointer("/AgentMovedToRemote/agent/remote_execution/relay_url")
                .and_then(serde_json::Value::as_str),
            Some("wss://relay.example")
        );
        assert_eq!(
            response
                .pointer("/AgentMovedToRemote/agent/remote_execution/relay_peer_protocol_version")
                .and_then(serde_json::Value::as_u64),
            Some(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION as u64)
        );
    }

    #[test]
    fn relay_client_response_projection_preserves_issued_client_tokens() {
        let cloud_token = LocalDaemonResponse::CloudRelayClientTokenIssued {
            profile: crate::local::CloudRelayProfile {
                api_url: "https://cloud.example".to_string(),
                email: "user@example.com".to_string(),
                account_id: "account-1".to_string(),
                user_id: "user-1".to_string(),
                account_slug: "account".to_string(),
                realm_id: "realm-1".to_string(),
                relay_url: "wss://relay.example".to_string(),
                issuer_id: "issuer-1".to_string(),
                client_id: None,
                client_alias: None,
                machine_id: None,
                machine_alias: None,
                machine_credential: None,
                cloud_session_token: None,
                cloud_session_expires_at_ms: None,
                token_expires_at_ms: None,
            },
            token: crate::local::CloudRelayRuntimeToken {
                relay_url: "wss://relay.example".to_string(),
                relay_token: "issued-runtime-token".to_string(),
                token_expires_at: "2099-01-01T00:00:00Z".to_string(),
            },
        };
        let connection = LocalDaemonResponse::KernelClientConnectionResolved {
            connection: crate::local::KernelClientConnection {
                relay_url: "wss://relay.example".to_string(),
                relay_token: "issued-client-token".to_string(),
                target_daemon_id: None,
                target_daemon_alias: None,
                token_expires_at: None,
                machine_id: None,
                kernel_id: None,
            },
        };

        for mut response in [
            serde_json::to_value(cloud_token).expect("cloud token should serialize"),
            serde_json::to_value(connection).expect("client connection should serialize"),
        ] {
            crate::local::redact_client_response_value(&mut response);
            assert!(
                response.to_string().contains("issued-runtime-token")
                    || response.to_string().contains("issued-client-token")
            );
        }
    }
}
