//! MP-08 / MP-10 / MP-11: leased projection reads the kernel-owned provider stream.
use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::transport::relay_peer::RelayPeerEvent;

impl KernelRuntimeState {
    pub(crate) async fn acknowledge_relay_leased_runtime_projection(
        &self,
        leased_agent_id: String,
        provider_run_id: String,
        delivery: crate::transport::relay_peer::RelayProjectionDelivery,
    ) -> Result<bool, DaemonError> {
        self.with_app_side_effect(move |app| {
            let mut runtime = RemoteLeaseRuntime::new(app);
            runtime.consume_leased_agent_authorization(&leased_agent_id)?;
            runtime.acknowledge_leased_runtime_projection(
                &leased_agent_id,
                &provider_run_id,
                &delivery,
            )
        })
        .await
    }

    pub(super) async fn acknowledge_remote_runtime_projection(
        &self,
        acknowledgement: crate::app::RemoteProjectionAcknowledgement,
    ) -> Result<(), DaemonError> {
        let binding = acknowledgement.binding;
        let config = self
            .with_app_side_effect(|app| app.relay_config_for_remote_execution(&binding))
            .await;
        let request = RelayPeerRequest::AcknowledgeLeasedRuntimeProjection {
            leased_agent_id: binding.leased_agent_id,
            provider_run_id: acknowledgement.provider_run_id,
            delivery: acknowledgement.delivery,
        };
        let target = ClientTarget {
            daemon_id: Some(binding.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let timeout = std::time::Duration::from_secs(5);
        let (response, peer) = match self.connected_relay_state_for_config(&config).await {
            Some(state) => crate::transport::relay_client::send_peer_request_via_connected_relay_authenticated(
                &config, &state, target, request, timeout,
            ).await?,
            None => crate::transport::relay_client::send_peer_request_via_temporary_connection_authenticated(
                &config, target, request, timeout,
            ).await?,
        };
        // The reply must still be from the worker to which the binding belongs.
        peer.authorize_worker(&binding.worker_kernel_id)?;
        match response {
            RelayPeerResponse::LeasedRuntimeProjectionAcknowledged { accepted: true } => Ok(()),
            _ => Err(DaemonError::LocalTransport {
                operation: "acknowledge remote projection",
                message: "worker rejected exact projection receipt".into(),
            }),
        }
    }

    pub(crate) async fn drain_relay_leased_runtime_projection(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
        replay_settled_completion: bool,
        require_authorization: bool,
    ) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
        let leased_agent_id = leased_agent_id.to_string();
        let provider_run_id = provider_run_id.to_string();
        if pump_output {
            let session_id = self
                .with_app_side_effect(|app| {
                    let mut runtime = RemoteLeaseRuntime::new(app);
                    if require_authorization {
                        runtime.consume_leased_agent_authorization(&leased_agent_id)?;
                    }
                    runtime.leased_provider_output_session(&leased_agent_id, &provider_run_id)
                })
                .await?;
            let _permit = self.provider_runtime_lanes.acquire(&provider_run_id).await;
            let recipients = self
                .owned
                .attachment_store
                .list_session_attachment_ids(&session_id);
            Box::pin(self.pump_owned_provider_output(
                &session_id,
                &provider_run_id,
                recipients,
                false,
            ))
            .await?;
        }
        self.with_app_side_effect(move |app| {
            let mut runtime = RemoteLeaseRuntime::new(app);
            if require_authorization && !pump_output {
                runtime.consume_leased_agent_authorization(&leased_agent_id)?;
            }
            // Completion records and exact settlement are committed by the same
            // runtime pump before the home sees them. The compatibility pump can
            // consume a provider result before its owned acknowledgement settles.
            runtime.drain_retained_leased_runtime_projection(
                &leased_agent_id,
                &provider_run_id,
                false,
                replay_settled_completion,
            )
        })
        .await
    }
}
