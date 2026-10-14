//! MP-08 / MP-10 / MP-11: leased projection reads the kernel-owned provider stream.
use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::transport::relay_peer::RelayPeerEvent;

impl KernelRuntimeState {
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
            runtime.drain_leased_runtime_projection_with_recovery(
                &leased_agent_id,
                &provider_run_id,
                false,
                replay_settled_completion,
            )
        })
        .await
    }
}
