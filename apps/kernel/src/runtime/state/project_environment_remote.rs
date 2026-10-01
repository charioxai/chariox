//! MP-08 / MP-10: Slice placement imports once through the ordinary leased-worker authority.
use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse, RemoteSkillSyncContext};

impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn ensure_slice_project_environment(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        if !self.remote_agent_is_home_managed_slice(agent) {
            return Ok(());
        }
        let active = self
            .owned
            .agent_store
            .list_agents()
            .into_iter()
            .filter_map(|agent| {
                agent.remote_execution().map(|remote| {
                    format!("{}:{}", remote.worker_kernel_id, remote.execution_lease_id)
                })
            })
            .collect();
        self.owned.project_environment_placements.retain(&active);
        let remote = agent.remote_execution().expect("slice remote binding");
        let key = format!("{}:{}", remote.worker_kernel_id, remote.execution_lease_id);
        self.owned
            .project_environment_placements
            .ensure(&key, || self.install_slice_project_environment(agent))
            .await
    }

    async fn install_slice_project_environment(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let remote = agent.remote_execution().expect("slice remote binding");
        let slice = self
            .owned
            .slice_store
            .list()
            .into_iter()
            .find(|slice| slice.worker_kernel_id.as_deref() == Some(&remote.worker_kernel_id))
            .ok_or_else(|| {
                remote_environment_error("slice environment has no owned slice record")
            })?;
        let Some(
            crate::managed_context::package::ManagedContextDevelopmentSelection::SourceProject {
                project_id,
                repositories,
            },
        ) = &slice.development
        else {
            return Ok(());
        };
        let publication = slice
            .development_publication
            .as_ref()
            .ok_or_else(|| remote_environment_error("slice development publication is missing"))?;
        if publication.repository_paths.len() != repositories.len() {
            return Err(remote_environment_error(
                "slice repository mapping is incomplete",
            ));
        }
        let selections = repositories
            .iter()
            .map(crate::managed_context::outbound_service::resolve_repository_selection)
            .collect::<Result<Vec<_>, _>>()?;
        let public_key = self
            .owned
            .relay_state
            .read()
            .await
            .peer_public_key(&remote.worker_kernel_id)
            .ok_or_else(|| {
                remote_environment_error("slice worker public identity is unavailable")
            })?;
        let layer = self
            .prepare_project_environment_layer(
                project_id,
                &selections,
                &remote.execution_lease_id,
                &remote.worker_kernel_id,
                &public_key,
                false,
            )
            .await?;
        let workspace_directories = repositories
            .iter()
            .zip(&publication.repository_paths)
            .map(|(source, path)| {
                let basename = Path::new(path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| {
                        remote_environment_error("slice repository basename is invalid")
                    })?;
                Ok((source.workspace_id.clone(), basename.to_string()))
            })
            .collect::<Result<_, DaemonError>>()?;
        let request = RelayPeerRequest::InstallLeasedProjectEnvironment {
            context: RemoteSkillSyncContext {
                home_kernel_id: self.owned.config_projection.snapshot().daemon_id,
                home_session_id: agent.session_id().into(),
                home_agent_id: agent.id().into(),
                leased_agent_id: remote.leased_agent_id.clone(),
            },
            layer,
            workspace_directories,
        };
        let mut config = self.owned.config_projection.snapshot();
        if let (Some(url), Some(token)) = (&remote.relay_url, &remote.relay_token) {
            config.apply_remote_relay_override(url.clone(), token.clone());
        }
        let target = chariox_relay::protocol::ClientTarget {
            daemon_id: Some(remote.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let response = match self.connected_relay_state_for_config(&config).await {
            Some(state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay(
                    &config, &state, target, request,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &config, target, request,
                )
                .await
            }
        }?;
        match response {
            RelayPeerResponse::LeasedProjectEnvironmentInstalled { .. } => Ok(()),
            _ => Err(remote_environment_error(
                "unexpected Project environment installation response",
            )),
        }
    }

    pub(crate) async fn install_leased_project_environment(
        &self,
        context: RemoteSkillSyncContext,
        layer: crate::managed_context::development::DevelopmentProjectEnvironment,
        directories: BTreeMap<String, String>,
    ) -> Result<String, DaemonError> {
        self.with_app_side_effect(move |app| {
            let mut runtime = RemoteLeaseRuntime::new(app);
            runtime.install_project_environment(context, layer, directories)
        })
        .await
    }
}
fn remote_environment_error(message: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "Project environment placement",
        message: message.into(),
    }
}
