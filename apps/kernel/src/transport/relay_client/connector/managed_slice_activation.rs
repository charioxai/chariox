//! MP-08/MP-10/MP-11: owned confirmation on a connected, reauthenticated relay.
use crate::runtime::router::CommandRouter;
use crate::transport::relay_client::RelayClientState;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::sleep;

const MANAGED_SLICE_ACTIVATION_CONFIRMATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const MANAGED_SLICE_ACTIVATION_CONFIRMATION_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct ManagedSliceActivationConfirmation {
    task: Option<JoinHandle<()>>,
}

impl ManagedSliceActivationConfirmation {
    pub(super) async fn resume_pending(
        &mut self,
        router: &Arc<CommandRouter>,
        state: &Arc<RwLock<RelayClientState>>,
    ) {
        if self.task.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        self.task = spawn_pending_managed_slice_activation_confirmation_after_connect(
            Arc::clone(router),
            Arc::clone(state),
        )
        .await;
    }
}

impl Drop for ManagedSliceActivationConfirmation {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub(super) fn socket_has_current_token(current: Option<&str>, socket: &str) -> bool {
    current == Some(socket)
}

pub(super) fn managed_slice_token_confirmation_matches(
    slice_id: &str,
    nonce: &str,
    peer_version: u32,
    expected_slice_id: &str,
    expected_nonce: &str,
) -> bool {
    slice_id == expected_slice_id
        && nonce == expected_nonce
        && peer_version >= crate::transport::relay_peer::MINIMUM_RELAY_PEER_RUNTIME_VERSION
}

fn spawn_managed_slice_activation_confirmation(
    router: Arc<CommandRouter>,
    state: Arc<RwLock<RelayClientState>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Some(pending) = state
                .write()
                .await
                .claim_pending_managed_slice_activation_confirmation()
            else {
                return;
            };
            let config = router.relay_config_snapshot();
            let response =
                crate::transport::relay_client::peer_client::send_peer_request_to_known_kernel_via_relay_with_timeout(
                    &config,
                    &state,
                    chariox_relay::protocol::ClientTarget {
                        daemon_id: Some(pending.owner_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    &pending.owner_public_key,
                    crate::transport::relay_peer::RelayPeerRequest::ConfirmManagedSliceRelayToken {
                        slice_id: pending.slice_id.clone(),
                        owner_kernel_id: pending.owner_kernel_id.clone(),
                        worker_kernel_id: pending.worker_kernel_id.clone(),
                        activation_nonce: pending.activation_nonce.clone(),
                    },
                    MANAGED_SLICE_ACTIVATION_CONFIRMATION_REQUEST_TIMEOUT,
                )
                .await;
            let retryable = match response {
                Ok(
                    crate::transport::relay_peer::RelayPeerResponse::ManagedSliceRelayTokenActivated {
                        slice_id,
                        activation_nonce,
                        relay_peer_protocol_version,
                    },
                ) if managed_slice_token_confirmation_matches(
                    &slice_id, &activation_nonce, relay_peer_protocol_version,
                    &pending.slice_id, &pending.activation_nonce,
                ) =>
                {
                    state
                        .write()
                        .await
                        .finish_managed_slice_activation_confirmation(
                            &pending.slice_id,
                            &pending.activation_nonce,
                        );
                    crate::logging::info_with_fields(
                        "daemon.relay_client",
                        "managed slice relay token activation confirmed",
                        serde_json::json!({
                            "slice_id": pending.slice_id,
                            "owner_kernel_id": pending.owner_kernel_id,
                        }),
                    );
                    false
                }
                Ok(crate::transport::relay_peer::RelayPeerResponse::ManagedSliceRelayTokenFailed {
                    code,
                    retryable,
                }) => {
                    if !retryable {
                        state
                            .write()
                            .await
                            .finish_managed_slice_activation_confirmation(
                                &pending.slice_id,
                                &pending.activation_nonce,
                            );
                    }
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "managed slice relay token activation confirmation was rejected",
                        serde_json::json!({
                            "slice_id": pending.slice_id,
                            "owner_kernel_id": pending.owner_kernel_id,
                            "code": code,
                            "retryable": retryable,
                        }),
                    );
                    retryable
                }
                Ok(response) => {
                    state
                        .write()
                        .await
                        .finish_managed_slice_activation_confirmation(
                            &pending.slice_id,
                            &pending.activation_nonce,
                        );
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "managed slice relay token activation confirmation returned an incompatible response",
                        serde_json::json!({
                            "slice_id": pending.slice_id,
                            "owner_kernel_id": pending.owner_kernel_id,
                            "response": format!("{response:?}"),
                        }),
                    );
                    false
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "managed slice relay token activation confirmation failed",
                        serde_json::json!({
                            "slice_id": pending.slice_id,
                            "owner_kernel_id": pending.owner_kernel_id,
                            "error": error.to_string(),
                        }),
                    );
                    true
                }
            };
            if !retryable || !state.read().await.connected() {
                return;
            }
            sleep(MANAGED_SLICE_ACTIVATION_CONFIRMATION_RETRY_DELAY).await;
            if !state.read().await.connected() {
                return;
            }
        }
    })
}

pub(super) async fn spawn_pending_managed_slice_activation_confirmation_after_connect(
    router: Arc<CommandRouter>,
    state: Arc<RwLock<RelayClientState>>,
) -> Option<JoinHandle<()>> {
    state
        .read()
        .await
        .pending_managed_slice_activation_confirmation()?;
    Some(spawn_managed_slice_activation_confirmation(router, state))
}
