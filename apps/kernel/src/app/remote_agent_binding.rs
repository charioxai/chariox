use arroba_relay::protocol::{ClientTarget, RelayKernelPresence};

use crate::agent::{AgentInstance, CreateAgentRequest, RemoteAgentBinding};
use crate::app::DaemonApp;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_discovery;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

use super::remote_kernel_selection::{
    ensure_kernel_can_host_provider, kernel_presence_matches_ref, select_remote_kernel,
};

impl DaemonApp {
    pub(crate) fn kernel_ref_is_local(&self, kernel_ref: &str) -> bool {
        let kernel_ref = kernel_ref.trim();
        !kernel_ref.is_empty()
            && (self.config.daemon_id == kernel_ref
                || self.config.daemon_alias.as_deref() == Some(kernel_ref))
    }

    pub(crate) fn spawn_worker_agent(
        &mut self,
        mut request: CreateAgentRequest,
        kernel_ref: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let relay_override = self.slice_relay_config_for_kernel_ref(kernel_ref);
        let relay_config = relay_override.as_ref().unwrap_or(&self.config);
        let worker_kernel = self.select_remote_kernel_by_ref_with_config(
            kernel_ref,
            &request.provider,
            relay_config,
        )?;
        request.kernel_ref = None;
        request.worktree_id = None;
        request.worktree_placement = None;
        let session_store = self.session_state_store();
        let agent = {
            let mut sessions = session_store.write();
            self.agents.create_agent(request, &mut sessions)?
        };
        let remote_setup =
            self.bind_remote_agent_to_worker(&agent, &worker_kernel, None, relay_override);
        if remote_setup.is_err() {
            let mut sessions = session_store.write();
            let _ = self.agents.destroy_agent(agent.id(), &mut sessions);
        }
        remote_setup
    }

    fn bind_remote_agent_to_worker(
        &mut self,
        agent: &AgentInstance,
        worker_kernel: &RelayKernelPresence,
        worktree_placement: Option<crate::agent::GitWorktreePlacement>,
        relay_override: Option<DaemonConfig>,
    ) -> Result<AgentInstance, DaemonError> {
        let relay_config = relay_override.as_ref().unwrap_or(&self.config);
        let session = self.sessions().get_session(agent.session_id())?;
        let effective_config =
            crate::session::effective_agent_execution_config(&session, Some(agent));
        let target = ClientTarget {
            daemon_id: Some(worker_kernel.kernel_id.clone()),
            daemon_alias: None,
        };
        let lease = match self.block_on_relay_future(send_peer_request_via_temporary_connection(
            relay_config,
            target.clone(),
            RelayPeerRequest::CreateExecutionLease {
                home_kernel_id: self.config.daemon_id.clone(),
                home_session_id: agent.session_id().to_string(),
                home_agent_id: agent.id().to_string(),
                owner_user_id: agent.owner_user_id().to_string(),
            },
        ))? {
            RelayPeerResponse::ExecutionLeaseCreated { lease } => lease,
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "create remote execution lease",
                    message: format!("unexpected peer response: {other:?}"),
                });
            }
        };
        let leased_agent =
            match self.block_on_relay_future(send_peer_request_via_temporary_connection(
                relay_config,
                target.clone(),
                RelayPeerRequest::SpawnLeasedAgent {
                    lease_id: lease.id.clone(),
                    provider: agent.provider().to_string(),
                    model: agent.model().map(ToOwned::to_owned),
                    effort: agent.effort().map(ToOwned::to_owned),
                    execution_mode: Some(effective_config.mode),
                    permission_level: Some(effective_config.permission_level),
                    worktree_id: agent.worktree_id().map(ToOwned::to_owned),
                    worktree_placement,
                },
            )) {
                Ok(RelayPeerResponse::LeasedAgentSpawned { leased_agent }) => leased_agent,
                Ok(other) => {
                    let _ = self.block_on_relay_future(send_peer_request_via_temporary_connection(
                        relay_config,
                        target,
                        RelayPeerRequest::DestroyExecutionLease {
                            lease_id: lease.id.clone(),
                        },
                    ));
                    return Err(DaemonError::LocalTransport {
                        operation: "spawn remote leased agent",
                        message: format!("unexpected peer response: {other:?}"),
                    });
                }
                Err(error) => {
                    let _ = self.block_on_relay_future(send_peer_request_via_temporary_connection(
                        relay_config,
                        target,
                        RelayPeerRequest::DestroyExecutionLease {
                            lease_id: lease.id.clone(),
                        },
                    ));
                    return Err(error);
                }
            };
        let bound = self.agents.bind_remote_execution(
            agent.id(),
            RemoteAgentBinding {
                worker_kernel_id: worker_kernel.kernel_id.clone(),
                worker_machine_id: worker_kernel.machine_id.clone(),
                execution_lease_id: lease.id,
                leased_agent_id: leased_agent.id,
                relay_url: relay_override
                    .as_ref()
                    .and_then(|config| config.relay_url.clone()),
                relay_token: relay_override
                    .as_ref()
                    .and_then(|config| config.relay_token.clone()),
            },
        )?;
        self.ensure_remote_agent_skill_packages(&bound)?;
        self.ensure_remote_agent_mcp_availability(&bound)?;
        Ok(bound)
    }

    pub(crate) fn refresh_remote_agent_binding(
        &mut self,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self.agents.get_agent(agent_id)?;
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Err(DaemonError::LocalTransport {
                operation: "refresh remote agent binding",
                message: format!("agent `{agent_id}` is not remote-backed"),
            });
        };
        let worker_kernel = self.select_remote_kernel_for_machine(
            &remote_execution.worker_machine_id,
            agent.provider(),
        )?;
        let rebound = self.bind_remote_agent_to_worker(&agent, &worker_kernel, None, None)?;
        self.durable_state_store().append_event(
            "agent.updated",
            Some(rebound.id().to_string()),
            serde_json::json!({
                "agent": &rebound,
                "source": "remote_agent_binding_refreshed",
            }),
        )?;
        if let Ok(session) = self.sessions.get_session(rebound.session_id()) {
            self.update_session_projection(session);
        }
        Ok(rebound)
    }

    pub(crate) fn move_agent_to_remote(
        &mut self,
        session_id: &str,
        agent_ref: &str,
        machine_ref: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .agents
            .get_agent(agent_ref)
            .or_else(|_| self.agents.get_agent_by_ref(agent_ref))?;
        if agent.session_id() != session_id {
            return Err(DaemonError::LocalTransport {
                operation: "move agent to remote",
                message: format!("agent `{agent_ref}` does not belong to session `{session_id}`"),
            });
        }
        if agent.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "move agent to remote",
                message: format!("agent `{agent_ref}` is already remote-backed"),
            });
        }
        if self
            .providers
            .get_run_for_agent(session_id, agent.id())
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "move agent to remote",
                message:
                    "agent has a provider run; stop or destroy the provider run before moving it"
                        .to_string(),
            });
        }
        let worker_kernel = self.select_remote_kernel_for_machine(machine_ref, agent.provider())?;
        self.bind_remote_agent_to_worker(&agent, &worker_kernel, None, None)
    }

    fn ensure_remote_agent_skill_packages(
        &mut self,
        agent: &AgentInstance,
    ) -> Result<(), DaemonError> {
        let Some(remote_execution) = agent.remote_execution() else {
            return Ok(());
        };
        if agent.skill_grants().is_empty() {
            return Ok(());
        }
        let session = self.sessions.get_session(agent.session_id())?;
        let mut roots = vec![crate::skill::ArrobaSkillRegistry::project_root(
            session.workspace_id(),
        )];
        if let Some(user_root) = crate::skill::ArrobaSkillRegistry::user_root() {
            roots.push(user_root);
        }
        let registry = crate::skill::ArrobaSkillRegistry::new(roots);
        let packages = agent
            .skill_grants()
            .iter()
            .map(|grant| {
                registry
                    .package(grant)?
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "ensure remote agent skill packages",
                        message: format!("skill `{grant}` is granted but is not installed"),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if packages.is_empty() {
            return Ok(());
        }
        let relay_config = self.relay_config_for_remote_execution(remote_execution);
        let response = self.block_on_relay_future(send_peer_request_via_temporary_connection(
            &relay_config,
            ClientTarget {
                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::EnsureRemoteSkillPackages {
                context: crate::transport::relay_peer::RemoteSkillSyncContext {
                    home_kernel_id: self.config.daemon_id.clone(),
                    home_session_id: agent.session_id().to_string(),
                    home_agent_id: agent.id().to_string(),
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                },
                packages,
            },
        ))?;
        match response {
            RelayPeerResponse::RemoteSkillPackagesEnsured { .. } => Ok(()),
            other => Err(DaemonError::LocalTransport {
                operation: "ensure remote agent skill packages",
                message: format!("unexpected remote skill sync response: {other:?}"),
            }),
        }
    }

    fn ensure_remote_agent_mcp_availability(
        &mut self,
        agent: &AgentInstance,
    ) -> Result<(), DaemonError> {
        let Some(remote_execution) = agent.remote_execution() else {
            return Ok(());
        };
        if agent.mcp_grants().is_empty() {
            return Ok(());
        }
        let session = self.sessions.get_session(agent.session_id())?;
        let mut roots = vec![crate::mcp::ArrobaMcpRegistry::project_root(
            session.workspace_id(),
        )];
        if let Some(user_root) = crate::mcp::ArrobaMcpRegistry::user_root() {
            roots.push(user_root);
        }
        let registry = crate::mcp::ArrobaMcpRegistry::new(roots);
        let required_mcps = agent
            .mcp_grants()
            .iter()
            .map(|grant| {
                let config = registry
                    .get(grant)?
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "ensure remote agent MCP availability",
                        message: format!("MCP `{grant}` is granted but is not installed"),
                    })?;
                let definition_hash = config.definition_hash()?;
                Ok(crate::transport::relay_peer::RequiredRemoteMcp {
                    config,
                    definition_hash,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if required_mcps.is_empty() {
            return Ok(());
        }
        let relay_config = self.relay_config_for_remote_execution(remote_execution);
        let response = self.block_on_relay_future(send_peer_request_via_temporary_connection(
            &relay_config,
            ClientTarget {
                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::CheckRemoteMcpAvailability {
                context: crate::transport::relay_peer::RemoteMcpCheckContext {
                    home_kernel_id: self.config.daemon_id.clone(),
                    home_session_id: agent.session_id().to_string(),
                    home_agent_id: agent.id().to_string(),
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                },
                required_mcps,
            },
        ))?;
        match response {
            RelayPeerResponse::RemoteMcpAvailabilityChecked { results } => {
                let unavailable = results
                    .iter()
                    .filter(|result| {
                        !matches!(
                            result.status,
                            crate::transport::relay_peer::RemoteMcpAvailabilityStatus::Available
                        )
                    })
                    .collect::<Vec<_>>();
                if unavailable.is_empty() {
                    Ok(())
                } else {
                    Err(DaemonError::LocalTransport {
                        operation: "ensure remote agent MCP availability",
                        message: format!(
                            "remote MCP unavailable on worker. Install the matching MCP definition in the worker project or user registry, then retry: {unavailable:?}"
                        ),
                    })
                }
            }
            other => Err(DaemonError::LocalTransport {
                operation: "ensure remote agent MCP availability",
                message: format!("unexpected remote MCP availability response: {other:?}"),
            }),
        }
    }

    fn select_remote_kernel_for_machine(
        &self,
        machine_ref: &str,
        provider: &str,
    ) -> Result<RelayKernelPresence, DaemonError> {
        let machine_ref = crate::config::DaemonConfig::resolve_registered_machine_ref(machine_ref)
            .unwrap_or_else(|| machine_ref.to_string());
        let (_, projected_kernels) = self.remote_relay_inventory_projection_store().snapshot();
        if let Some(kernel) = select_remote_kernel(projected_kernels, &machine_ref, provider) {
            return Ok(kernel);
        }
        let kernels = self.block_on_relay_future(
            relay_discovery::list_live_kernels_for_machine(&self.config, &machine_ref),
        )?;
        select_remote_kernel(kernels, &machine_ref, provider).ok_or_else(|| {
            DaemonError::NoRemoteKernelAvailable {
                machine_ref,
                provider: provider.to_string(),
            }
        })
    }

    fn select_remote_kernel_by_ref_with_config(
        &self,
        kernel_ref: &str,
        provider: &str,
        relay_config: &DaemonConfig,
    ) -> Result<RelayKernelPresence, DaemonError> {
        let kernel_ref = kernel_ref.trim();
        if relay_config.relay_url == self.config.relay_url
            && relay_config.relay_token == self.config.relay_token
        {
            let (_, projected_kernels) = self.remote_relay_inventory_projection_store().snapshot();
            if let Some(kernel) = projected_kernels
                .into_iter()
                .find(|kernel| kernel_presence_matches_ref(kernel, kernel_ref))
            {
                return ensure_kernel_can_host_provider(kernel, kernel_ref, provider);
            }
        }
        let kernel =
            self.block_on_relay_future(relay_discovery::get_live_kernel(relay_config, kernel_ref))?;
        ensure_kernel_can_host_provider(kernel, kernel_ref, provider)
    }

    fn slice_relay_config_for_kernel_ref(&self, kernel_ref: &str) -> Option<DaemonConfig> {
        let slice = self.slices.resolve_by_worker_kernel_ref(kernel_ref)?;
        let relay = crate::slice::local_docker_private_relay(&slice);
        let mut config = self.config.clone();
        config.relay_url = Some(relay.relay_url);
        config.relay_token = Some(relay.relay_token);
        config.cloud_relay = None;
        Some(config)
    }

    pub(crate) fn relay_config_for_remote_execution(
        &self,
        remote_execution: &RemoteAgentBinding,
    ) -> DaemonConfig {
        let mut config = self.config.clone();
        if let (Some(relay_url), Some(relay_token)) = (
            remote_execution.relay_url.clone(),
            remote_execution.relay_token.clone(),
        ) {
            config.relay_url = Some(relay_url);
            config.relay_token = Some(relay_token);
            config.cloud_relay = None;
        }
        config
    }
}

#[cfg(test)]
mod tests {
    use crate::app::DaemonApp;
    use crate::config::DaemonConfig;

    #[test]
    fn slice_worker_kernel_refs_resolve_to_private_relay_config() {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let slice = app
            .slices()
            .create(
                &app.config().daemon_id,
                &app.config().host_machine_id,
                crate::slice::CreateSliceInput {
                    name: "linux-dev".to_string(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".to_string(),
                    workspace_mount: Some("/repo".to_string()),
                    worker_kernel_ref: None,
                    display_url: Some("http://127.0.0.1:6080".to_string()),
                    now_ms: 42,
                },
            )
            .expect("slice should create");

        let relay_config = app
            .slice_relay_config_for_kernel_ref(&slice.worker_kernel_ref)
            .expect("slice worker ref should have relay config");

        assert_eq!(
            relay_config.relay_url.as_deref(),
            Some("ws://127.0.0.1:43130")
        );
        assert_eq!(
            relay_config.relay_token.as_deref(),
            Some("slice-local-daemon-test-slice-1")
        );
        assert!(relay_config.cloud_relay.is_none());
    }
}
