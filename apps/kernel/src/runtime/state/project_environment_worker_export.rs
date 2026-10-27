//! MP-08 / MP-10 / MP-11: The exporting worker refreshes its own manifest and seals its own values.
use super::project_environment_export::environment_failure;
use super::project_environment_workspaces::project_environment_repository_selections;
use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::transport::relay_peer::{ProjectEnvironmentExportTarget, RemoteSkillSyncContext};
impl KernelRuntimeState {
    pub(super) async fn export_remote_project_environment(
        &self,
        agent: &crate::agent::AgentInstance,
        interactive: bool,
        target_name: &str,
        target: Option<ProjectEnvironmentExportTarget>,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let remote = agent
            .remote_execution()
            .ok_or_else(|| environment_failure("source worker binding unavailable"))?;
        if remote
            .relay_peer_protocol_version
            .is_none_or(|version| version < 66)
        {
            return Err(environment_failure(
                "source worker requires relay peer protocol 66",
            ));
        }
        let mut config = self.owned.config_projection.snapshot();
        if let (Some(url), Some(token)) = (&remote.relay_url, &remote.relay_token) {
            config.apply_remote_relay_override(url.clone(), token.clone());
        }
        let timeout = if interactive {
            Duration::from_secs(1800)
        } else {
            Duration::from_secs(240)
        };
        crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &config,
            ClientTarget {
                daemon_id: Some(remote.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::ExportLeasedProjectEnvironment {
                context: RemoteSkillSyncContext {
                    home_kernel_id: config.daemon_id.clone(),
                    home_session_id: agent.session_id().into(),
                    home_agent_id: agent.id().into(),
                    leased_agent_id: remote.leased_agent_id.clone(),
                },
                interactive,
                target_name: target_name.into(),
                target,
            },
            timeout,
        )
        .await
    }
    pub(crate) async fn export_leased_project_environment(
        &self,
        context: RemoteSkillSyncContext,
        interactive: bool,
        target_name: String,
        target: Option<ProjectEnvironmentExportTarget>,
    ) -> Result<RelayPeerResponse, DaemonError> {
        if target_name.len() > 256 {
            return Err(environment_failure(
                "source export target name exceeds bounds",
            ));
        }
        let bound = self
            .with_app_side_effect(move |app| {
                let mut runtime = RemoteLeaseRuntime::new(app);
                runtime.consume_leased_agent_authorization(&context.leased_agent_id)?;
                runtime.leased_project_target(
                    &context.leased_agent_id,
                    &context.home_session_id,
                    &context.home_agent_id,
                    None,
                )
            })
            .await?;
        let session = self
            .owned
            .session_store
            .get_session(&bound.backing_session_id)?;
        let project = self.owned.session_store.get_project(session.project_id())?;
        let repositories = project_environment_repository_selections(&project, &session);
        let prepared = self
            .refresh_project_environment_state(
                project.id(),
                &repositories,
                interactive,
                &target_name,
            )
            .await?;
        let layer = target
            .map(|target| {
                let config = self.owned.config_projection.snapshot();
                Ok::<_, DaemonError>(
                    crate::managed_context::development::DevelopmentProjectEnvironment {
                        sealed: crate::project_environment::seal_project_environment(
                            &target.context_id,
                            &config.daemon_id,
                            &target.kernel_id,
                            &config.relay_private_key,
                            &target.public_key,
                            &prepared.state.manifest,
                            &prepared.resolved,
                        )?,
                        evidence: prepared.state.evidence.clone(),
                        repository_workspaces: BTreeMap::new(),
                    },
                )
            })
            .transpose()?;
        Ok(RelayPeerResponse::LeasedProjectEnvironmentExport {
            manifest: prepared.state.manifest.clone(),
            evidence: prepared.state.evidence.clone(),
            layer,
        })
    }
}
