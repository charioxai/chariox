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
        if self
            .owned
            .session_store
            .get_session(agent.session_id())?
            .project_id()
            != project_id
        {
            return Err(remote_environment_error(
                "slice Project does not match the selected session",
            ));
        }
        if remote
            .relay_peer_protocol_version
            .is_none_or(|version| version < 66)
        {
            return Err(remote_environment_error(
                "slice worker requires relay peer protocol 66",
            ));
        }
        let mut reuse_config = self.owned.config_projection.snapshot();
        if let (Some(url), Some(token)) = (&remote.relay_url, &remote.relay_token) {
            reuse_config.apply_remote_relay_override(url.clone(), token.clone());
        }
        let reuse = crate::transport::relay_client::send_peer_request_via_temporary_connection(
            &reuse_config,
            chariox_relay::protocol::ClientTarget {
                daemon_id: Some(remote.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::UseLeasedProjectEnvironment {
                context: RemoteSkillSyncContext {
                    home_kernel_id: reuse_config.daemon_id.clone(),
                    home_session_id: agent.session_id().into(),
                    home_agent_id: agent.id().into(),
                    leased_agent_id: remote.leased_agent_id.clone(),
                },
                project_id: project_id.clone(),
            },
        )
        .await?;
        match reuse {
            RelayPeerResponse::LeasedProjectEnvironmentUsed { exists: true } => return Ok(()),
            RelayPeerResponse::LeasedProjectEnvironmentUsed { exists: false } => {}
            _ => return Err(remote_environment_error("unexpected target reuse response")),
        }
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
        let layer = if let Some(source_ref) = &slice.source_slice_ref {
            let source_agent = self.source_slice_agent(source_ref)?;
            let response = self
                .export_remote_project_environment(
                    &source_agent,
                    false,
                    &slice.name,
                    Some(
                        crate::transport::relay_peer::ProjectEnvironmentExportTarget {
                            context_id: remote.execution_lease_id.clone(),
                            kernel_id: remote.worker_kernel_id.clone(),
                            public_key: public_key.clone(),
                        },
                    ),
                )
                .await?;
            let RelayPeerResponse::LeasedProjectEnvironmentExport {
                layer: Some(layer), ..
            } = response
            else {
                return Err(remote_environment_error(
                    "source worker did not seal its Project environment",
                ));
            };
            let expected_source_key = self
                .owned
                .relay_state
                .read()
                .await
                .peer_public_key(&layer.sealed.binding.source_kernel_id)
                .ok_or_else(|| {
                    remote_environment_error("source worker public identity unavailable")
                })?;
            if layer.sealed.values.sender_public_key != expected_source_key {
                return Err(remote_environment_error(
                    "source worker sealed identity mismatch",
                ));
            }
            layer
        } else {
            self.prepare_project_environment_layer(
                project_id,
                &selections,
                &remote.execution_lease_id,
                &remote.worker_kernel_id,
                &public_key,
                false,
            )
            .await?
        };
        let source_kernel_id = slice
            .source_slice_ref
            .as_ref()
            .map(|_| layer.sealed.binding.source_kernel_id.clone());
        let source_workspaces: BTreeSet<_> = layer
            .sealed
            .manifest
            .entries
            .iter()
            .map(|entry| entry.workspace_id.clone())
            .chain(
                layer
                    .sealed
                    .manifest
                    .private_files
                    .iter()
                    .map(|file| file.workspace_id.clone()),
            )
            .chain(layer.evidence.files.keys().cloned())
            .collect();
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
                let workspace = if source_kernel_id.is_some() {
                    source_workspaces
                        .iter()
                        .find(|workspace| {
                            Path::new(workspace).file_name() == Some(std::ffi::OsStr::new(basename))
                        })
                        .cloned()
                        .ok_or_else(|| {
                            remote_environment_error("source worker workspace mapping missing")
                        })?
                } else {
                    source.workspace_id.clone()
                };
                Ok((workspace, basename.to_string()))
            })
            .collect::<Result<_, DaemonError>>()?;
        let request = RelayPeerRequest::InstallLeasedProjectEnvironment {
            source_kernel_id,
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

    pub(crate) async fn use_leased_project_environment(
        &self,
        context: RemoteSkillSyncContext,
        project_id: String,
    ) -> Result<bool, DaemonError> {
        self.with_app_side_effect(move |app| {
            RemoteLeaseRuntime::new(app).use_project_environment(context, project_id)
        })
        .await
    }
    pub(crate) async fn install_leased_project_environment(
        &self,
        context: RemoteSkillSyncContext,
        source_kernel_id: Option<String>,
        layer: crate::managed_context::development::DevelopmentProjectEnvironment,
        directories: BTreeMap<String, String>,
    ) -> Result<String, DaemonError> {
        self.with_app_side_effect(move |app| {
            let mut runtime = RemoteLeaseRuntime::new(app);
            runtime.install_project_environment(context, source_kernel_id, layer, directories)
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

// MP-08 / MP-10 / MP-11: Home authenticates the selected agent; worker owns its copy.
impl KernelRuntimeState {
    pub(super) async fn remote_project_environment(
        &self,
        agent: &crate::agent::AgentInstance,
        adjust: bool,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let remote = agent
            .remote_execution()
            .ok_or_else(|| remote_environment_error("worker binding missing"))?;
        if remote
            .relay_peer_protocol_version
            .is_none_or(|version| version < 66)
        {
            return Err(remote_environment_error(
                "worker needs relay peer protocol 66 for Environment",
            ));
        }
        let mut config = self.owned.config_projection.snapshot();
        if let (Some(url), Some(token)) = (&remote.relay_url, &remote.relay_token) {
            config.apply_remote_relay_override(url.clone(), token.clone());
        }
        let request = RelayPeerRequest::ReadLeasedProjectEnvironment {
            context: RemoteSkillSyncContext {
                home_kernel_id: config.daemon_id.clone(),
                home_session_id: agent.session_id().into(),
                home_agent_id: agent.id().into(),
                leased_agent_id: remote.leased_agent_id.clone(),
            },
            adjust,
        };
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
            RelayPeerResponse::LeasedProjectEnvironment {
                manifest,
                adjustment_started,
            } if adjustment_started == adjust => Ok(if adjust {
                LocalDaemonResponse::ProjectEnvironmentAdjustmentStarted {
                    session_id: agent.session_id().into(),
                    agent_id: agent.id().into(),
                }
            } else {
                LocalDaemonResponse::ProjectEnvironmentManifest { manifest }
            }),
            _ => Err(remote_environment_error(
                "unexpected worker Environment response",
            )),
        }
    }
    pub(crate) async fn read_leased_project_environment(
        &self,
        context: RemoteSkillSyncContext,
        adjust: bool,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let target = self
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
            .get_session(&target.backing_session_id)?;
        if adjust {
            Box::pin(self.start_project_environment_adjustment(
                crate::local::AdjustProjectEnvironmentRequest {
                    session_id: target.backing_session_id,
                    agent_id: target.backing_agent_id,
                },
                &target.owner_user_id,
            ))
            .await?;
        }
        let response = Box::pin(self.get_project_environment_manifest(
            crate::local::GetProjectEnvironmentManifestRequest {
                project_id: session.project_id().into(),
                agent_id: None,
            },
            &target.owner_user_id,
        ))
        .await?;
        let LocalDaemonResponse::ProjectEnvironmentManifest { manifest } = response else {
            unreachable!()
        };
        Ok(RelayPeerResponse::LeasedProjectEnvironment {
            manifest,
            adjustment_started: adjust,
        })
    }
}

impl KernelRuntimeState {
    pub(crate) fn authorize_forwarded_interaction(
        &self,
        worker_id: &str,
        context: &crate::transport::relay_peer::RemoteNativeInteractionContext,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(&context.home_agent_id)?;
        let remote = agent
            .remote_execution()
            .ok_or_else(|| remote_environment_error("interaction has no selected worker"))?;
        if agent.session_id() != context.home_session_id
            || remote.worker_kernel_id != worker_id
            || remote.leased_agent_id != context.leased_agent_id
        {
            return Err(remote_environment_error(
                "interaction does not match the selected worker lease",
            ));
        }
        Ok(())
    }
}
