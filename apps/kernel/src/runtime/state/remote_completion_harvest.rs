//! MP-08/MP-09/MP-10/MP-11 A04: git observations a worker holds after a
//! leased turn completes. The request goes over the one home-to-worker route
//! and is awaited with the app lock released, so a private relay connector
//! that projects the completion on its own single-threaded runtime keeps
//! polling the relay while the home waits for the answer.
use super::*;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

const REMOTE_COMPLETION_HARVEST_RESPONSE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(60);

impl KernelRuntimeState {
    /// A harvest failure is logged, never raised: the completion itself is
    /// already projected.
    pub(super) async fn harvest_remote_completion_observations(
        &self,
        harvest: crate::app::RemoteCompletionHarvest,
    ) {
        let crate::app::RemoteCompletionHarvest {
            remote_execution,
            provider_run_id,
        } = harvest;
        match self
            .request_remote_git_observations(&remote_execution, &provider_run_id)
            .await
        {
            Ok(RelayPeerResponse::LeasedGitObserved {
                git_observations,
                workspace_live_sync_change,
                ..
            }) => {
                let worker_kernel_id = remote_execution.worker_kernel_id.clone();
                self.with_app_side_effect(move |app| {
                    if let Err(error) = crate::git_observer::append_observations(
                        &app.operational_history_store(),
                        git_observations,
                    ) {
                        crate::logging::warn_with_fields(
                            "daemon.git_observer",
                            "failed to append projected remote git observations",
                            serde_json::json!({
                                "worker_kernel_id": worker_kernel_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    if let Some(change) = workspace_live_sync_change {
                        app.fanout_remote_workspace_live_sync_change(
                            change,
                            Some(&worker_kernel_id),
                        );
                    }
                })
                .await;
            }
            Ok(other) => crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "unexpected projected remote completion harvest response",
                serde_json::json!({
                    "worker_kernel_id": remote_execution.worker_kernel_id,
                    "leased_agent_id": remote_execution.leased_agent_id,
                    "response": format!("{other:?}"),
                }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "failed to harvest projected remote completion observations",
                serde_json::json!({
                    "worker_kernel_id": remote_execution.worker_kernel_id,
                    "leased_agent_id": remote_execution.leased_agent_id,
                    "error": error.to_string(),
                }),
            ),
        }
    }

    async fn request_remote_git_observations(
        &self,
        remote_execution: &crate::agent::RemoteAgentBinding,
        provider_run_id: &str,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let binding = remote_execution.clone();
        let config = self
            .with_app_side_effect(move |app| app.relay_config_for_remote_execution(&binding))
            .await;
        self.enqueue_leased_worker_request(
            &config,
            ClientTarget {
                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::ObserveLeasedGitAfter {
                leased_agent_id: remote_execution.leased_agent_id.clone(),
                provider_run_id: provider_run_id.to_string(),
            },
            REMOTE_COMPLETION_HARVEST_RESPONSE_TIMEOUT,
        )
        .await?
        .wait()
        .await
    }
}
