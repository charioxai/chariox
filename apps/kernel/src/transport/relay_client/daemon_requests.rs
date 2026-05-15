//! Inbound browser/client relay request dispatch to the kernel command router.

use std::sync::atomic::{AtomicU64, Ordering};

use arroba_relay::protocol::{EncryptedRelayPayload, RelayCallerIdentity, RelayError};

use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::router::CommandRouter;
use crate::transport::relay_crypto;

use super::request_errors::{map_relay_error, relay_error, relay_request_kind};

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
) -> RelayRequestOutcome {
    let (request, client_public_key, daemon_private_key) = {
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
        let request = match serde_json::from_slice::<LocalDaemonRequest>(&decrypted.plaintext) {
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
    let request_kind = relay_request_kind(&request);
    crate::logging::info_with_fields(
        "daemon.relay_client",
        "relay daemon request dispatching",
        serde_json::json!({
            "request_kind": request_kind,
        }),
    );
    let result =
        dispatch_relay_client_request(router, command_sequence, caller_identity, request).await;
    match result {
        Ok(response) => {
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
        Err(error) => RelayRequestOutcome {
            encrypted_response: None,
            error: Some(map_relay_error(&error)),
        },
    }
}

async fn dispatch_relay_client_request(
    router: &CommandRouter,
    command_sequence: &AtomicU64,
    caller_identity: Option<RelayCallerIdentity>,
    request: LocalDaemonRequest,
) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
    let sequence = command_sequence.fetch_add(1, Ordering::Relaxed);
    let command_id = format!(
        "relay-client-{}-{sequence}",
        crate::session::unix_epoch_ms()
    );
    let command = KernelCommand::from_local_request_with_caller(
        command_id,
        KernelCommandSource::RelayClient,
        caller_identity
            .map(KernelCaller::from_relay_identity)
            .unwrap_or_else(|| KernelCaller::for_source(&KernelCommandSource::RelayClient)),
        None,
        None,
        &request,
    );
    router.dispatch(command, request).await
}
