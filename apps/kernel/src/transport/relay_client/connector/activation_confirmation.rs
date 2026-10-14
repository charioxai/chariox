//! MP-08/MP-10/MP-11: activation confirmation on an admitted relay socket.
use super::*;

/// One sender per connection. Repeated heartbeats must not spend the bounded
/// retry budget concurrently, and a disconnected socket must not keep a sender.
#[derive(Default)]
pub(super) struct ActivationConfirmationSender {
    task: Option<JoinHandle<()>>,
}

impl ActivationConfirmationSender {
    pub(super) async fn dispatch_pending(
        &mut self,
        router: &Arc<CommandRouter>,
        state: &Arc<RwLock<RelayClientState>>,
    ) {
        if self.task.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        self.task = spawn_pending_managed_slice_activation_confirmation_on_connected_socket(
            Arc::clone(router),
            Arc::clone(state),
        )
        .await;
    }
}

impl Drop for ActivationConfirmationSender {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

// MP-08/MP-10/MP-11: reject an obsolete home before settling token confirmation.
pub(super) fn managed_slice_token_confirmation_matches(
    slice_id: &str,
    nonce: &str,
    peer_version: u32,
    expected_slice_id: &str,
    expected_nonce: &str,
) -> bool {
    slice_id == expected_slice_id
        && nonce == expected_nonce
        && peer_version >= crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
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
                super::super::peer_client::send_peer_request_to_known_kernel_via_relay_with_timeout(
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

pub(super) async fn spawn_pending_managed_slice_activation_confirmation_on_connected_socket(
    router: Arc<CommandRouter>,
    state: Arc<RwLock<RelayClientState>>,
) -> Option<JoinHandle<()>> {
    let guard = state.read().await;
    if !guard.connected() {
        return None;
    }
    guard.pending_managed_slice_activation_confirmation()?;
    drop(guard);
    Some(spawn_managed_slice_activation_confirmation(router, state))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sender_coalesces_ticks_and_cancels_inflight_confirmation_on_drop() {
        let config = crate::config::DaemonConfig::for_tests();
        let owner_key = config.relay_public_key.clone();
        let router = Arc::new(CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap())),
            1,
        ));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (outgoing, mut requests, _events) = RelayOutgoingSender::channel(4);
        state
            .write()
            .await
            .stage_managed_slice_activation_confirmation(
            super::super::super::connection_state::PendingManagedSliceActivationConfirmation::new(
                "slice-1".into(),
                "owner-1".into(),
                owner_key,
                "worker-1".into(),
                "activation-1".into(),
            ),
        );
        let mut sender = ActivationConfirmationSender::default();
        sender.dispatch_pending(&router, &state).await;
        assert!(
            sender.task.is_none(),
            "a disconnected socket must not dispatch"
        );
        set_connected(&state, outgoing, "wss://relay.example.test".into())
            .await
            .unwrap();
        sender.dispatch_pending(&router, &state).await;
        let request = timeout(Duration::from_secs(1), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(request, RelayEnvelope::DaemonPeerRequest { .. }));
        for _ in 0..20 {
            sender.dispatch_pending(&router, &state).await;
        }
        assert!(
            timeout(Duration::from_millis(50), requests.recv())
                .await
                .is_err(),
            "timer ticks must not launch concurrent retries"
        );
        let task = sender.task.as_ref().unwrap().abort_handle();
        drop(sender);
        timeout(Duration::from_secs(1), async {
            while !task.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(state
            .read()
            .await
            .pending_peer_requests
            .values()
            .all(|response| response.is_closed()));
    }
}
