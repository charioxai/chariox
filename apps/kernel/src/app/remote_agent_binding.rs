use std::{path::PathBuf, time::Duration};

use chariox_relay::protocol::{ClientTarget, RelayKernelPresence};

use crate::agent::{AgentInstance, CreateAgentRequest, RemoteAgentBinding};
use crate::app::DaemonApp;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::transport::relay_client::{
    send_peer_request_via_connected_relay, send_peer_request_via_temporary_connection,
};
use crate::transport::relay_discovery;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

use super::remote_kernel_selection::{
    ensure_kernel_can_host_provider, kernel_presence_matches_ref,
    no_remote_kernel_available_message, select_remote_kernel,
};

const REMOTE_KERNEL_REF_DISCOVERY_ATTEMPTS: usize = 20;
const REMOTE_KERNEL_REF_DISCOVERY_RETRY_DELAY_MS: u64 = 250;

impl DaemonApp {
    pub(crate) fn remote_extension_manifest_for_agent(
        &self,
        agent: &AgentInstance,
    ) -> Result<crate::extension::RemoteExtensionManifest, DaemonError> {
        if agent.remote_execution().is_none() {
            return Ok(crate::extension::RemoteExtensionManifest::default());
        }
        let session = self.sessions.get_session(agent.session_id())?;
        let mut tools = Vec::new();

        let mcp_roots = app_mcp_registry_roots(session.workspace_id());
        let mcp_registry = crate::mcp::CharioxMcpRegistry::new(mcp_roots);
        for name in agent.mcp_grants() {
            let Some(config) = mcp_registry.get(&name)? else {
                return Err(DaemonError::LocalTransport {
                    operation: "remote extension manifest",
                    message: format!("MCP `{name}` is granted but is not installed on home"),
                });
            };
            tools.push(crate::extension::RemoteExtensionTool {
                kind: crate::extension::ExtensionKind::Mcp,
                name: name.clone(),
                tool_name: name,
                description: "Home-proxied MCP server".to_string(),
                input_schema: serde_json::json!({}),
                authority: crate::extension::ExtensionAuthority::Home,
                definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
                execution_location: crate::extension::ExtensionExecutionLocation::Home,
                safety: None,
                timeout_sec: config.tool_timeout_sec,
                version_hash: Some(config.definition_hash()?),
            });
        }

        let script_roots = app_script_registry_roots(session.workspace_id());
        let script_registry = crate::script::CharioxScriptRegistry::new(script_roots);
        for grant in agent.script_grants() {
            let Some(script) = script_registry.get(&grant.name)? else {
                return Err(DaemonError::LocalTransport {
                    operation: "remote extension manifest",
                    message: format!(
                        "script `{}` is granted but is not registered on home",
                        grant.name
                    ),
                });
            };
            tools.push(crate::extension::RemoteExtensionTool {
                kind: crate::extension::ExtensionKind::Script,
                name: grant.name,
                tool_name: script.name,
                description: script.description,
                input_schema: script.input_schema,
                authority: crate::extension::ExtensionAuthority::Home,
                definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
                execution_location: crate::extension::ExtensionExecutionLocation::Home,
                safety: None,
                timeout_sec: Some(
                    script
                        .timeout_sec
                        .unwrap_or(crate::script::DEFAULT_SCRIPT_EXECUTION_TIMEOUT_SEC),
                ),
                version_hash: Some(script.definition_hash),
            });
        }

        let connector_registry = crate::connector::CharioxConnectorRegistry::user()?;
        for grant in agent.connector_grants() {
            let Some(connector) = connector_registry.get(&grant.name)? else {
                return Err(DaemonError::LocalTransport {
                    operation: "remote extension manifest",
                    message: format!(
                        "connector `{}` is granted but is not registered on home",
                        grant.name
                    ),
                });
            };
            let max_safety = crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())?;
            let definition_hash = connector.definition_hash()?;
            for operation in connector.operations {
                if operation.safety > max_safety {
                    continue;
                }
                tools.push(crate::extension::RemoteExtensionTool {
                    kind: crate::extension::ExtensionKind::Connector,
                    name: connector.name.clone(),
                    tool_name: crate::connector::connector_tool_name(
                        &connector.name,
                        &operation.name,
                    ),
                    description: operation.description,
                    input_schema: operation.input_schema,
                    authority: crate::extension::ExtensionAuthority::Home,
                    definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
                    execution_location: crate::extension::ExtensionExecutionLocation::Home,
                    safety: Some(operation.safety.as_str().to_string()),
                    timeout_sec: Some(connector.timeout_ms / 1000),
                    version_hash: Some(definition_hash.clone()),
                });
            }
        }
        Ok(crate::extension::RemoteExtensionManifest { tools })
    }

    pub(crate) fn required_remote_mcps_for_native_provider_launch(
        &self,
        agent: &AgentInstance,
    ) -> Result<Vec<crate::transport::relay_peer::RequiredRemoteMcp>, DaemonError> {
        let mcp_grants = agent.mcp_grants();
        if mcp_grants.is_empty() {
            return Ok(Vec::new());
        }
        let session = self.sessions.get_session(agent.session_id())?;
        let registry =
            crate::mcp::CharioxMcpRegistry::new(app_mcp_registry_roots(session.workspace_id()));
        mcp_grants
            .iter()
            .map(|grant| {
                let config = registry
                    .get(grant)?
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "remote native provider capabilities",
                        message: format!("MCP `{grant}` is granted but is not installed"),
                    })?;
                Ok(crate::transport::relay_peer::RequiredRemoteMcp {
                    definition_hash: config.definition_hash()?,
                    config,
                })
            })
            .collect()
    }

    pub(crate) fn required_remote_skills_for_native_provider_launch(
        &self,
        agent: &AgentInstance,
    ) -> Result<Vec<crate::transport::relay_peer::RequiredRemoteSkill>, DaemonError> {
        let skill_grants = agent.skill_grants();
        if skill_grants.is_empty() {
            return Ok(Vec::new());
        }
        let session = self.sessions.get_session(agent.session_id())?;
        let registry = crate::skill::CharioxSkillRegistry::new(app_skill_registry_roots(
            session.workspace_id(),
        ));
        skill_grants
            .iter()
            .map(|grant| {
                let package =
                    registry
                        .package(grant)?
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "remote native provider capabilities",
                            message: format!("skill `{grant}` is granted but is not installed"),
                        })?;
                Ok(crate::transport::relay_peer::RequiredRemoteSkill {
                    name: package.metadata.name,
                    version_hash: package.version_hash,
                })
            })
            .collect()
    }

    pub(crate) fn remote_prompt_capabilities_for_agent(
        &self,
        agent: &AgentInstance,
    ) -> Result<
        (
            Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
            Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
            crate::extension::RemoteExtensionManifest,
        ),
        DaemonError,
    > {
        let manifest = self.remote_extension_manifest_for_agent(agent)?;
        let native_provider_run = self
            .provider_run_projection
            .get_for_agent(agent.session_id(), agent.id())
            .is_some_and(|run| !run.client_interface().is_chariox());
        if native_provider_run {
            return Ok((
                self.required_remote_mcps_for_native_provider_launch(agent)?,
                Some(self.required_remote_skills_for_native_provider_launch(agent)?),
                manifest.without_mcp_tools(),
            ));
        }
        Ok((Vec::new(), None, manifest))
    }

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
        let relay_config = relay_override
            .clone()
            .unwrap_or_else(|| self.config.clone());
        let discovery_config =
            self.remote_worker_discovery_config(kernel_ref, relay_config.clone())?;
        let worker_kernel = self.select_remote_kernel_by_ref_with_config(
            kernel_ref,
            &request.provider,
            &discovery_config,
        )?;
        let worker_worktree_id =
            self.worker_worktree_id_for_kernel_ref(kernel_ref, request.worktree_id.clone());
        let worktree_placement = request.worktree_placement.clone();
        request.kernel_ref = None;
        request.worktree_id = None;
        request.worktree_placement = None;
        let session_store = self.session_state_store();
        let agent = {
            let mut sessions = session_store.write();
            self.agents.create_agent(request, &mut sessions)?
        };
        let remote_setup = self.bind_remote_agent_to_worker(
            &agent,
            &worker_kernel,
            worker_worktree_id,
            worktree_placement,
            relay_override,
        );
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
        worker_worktree_id: Option<String>,
        worktree_placement: Option<crate::agent::GitWorktreePlacement>,
        relay_override: Option<DaemonConfig>,
    ) -> Result<AgentInstance, DaemonError> {
        let relay_config = relay_override
            .clone()
            .unwrap_or_else(|| self.config.clone());
        let session = self.sessions().get_session(agent.session_id())?;
        let effective_config =
            crate::session::effective_agent_execution_config(&session, Some(agent));
        let workspace_live_sync_mode =
            crate::provider::provider_workspace_live_sync_mode_for_session(
                agent.provider(),
                &self.config,
                Some(&session),
            );
        let target = ClientTarget {
            daemon_id: Some(worker_kernel.kernel_id.clone()),
            daemon_alias: None,
        };
        self.remember_remote_worker_public_key(&relay_config, worker_kernel);
        let use_connected_relay =
            self.hosted_shared_slice_uses_connected_relay(&worker_kernel.kernel_id);
        let (lease, relay_peer_protocol_version) = match self.send_remote_binding_request(
            &relay_config,
            target.clone(),
            RelayPeerRequest::CreateExecutionLease {
                home_kernel_id: self.config.daemon_id.clone(),
                home_session_id: agent.session_id().to_string(),
                home_agent_id: agent.id().to_string(),
                home_agent_metaagent: agent.is_metaagent(),
                owner_user_id: agent.owner_user_id().to_string(),
            },
            use_connected_relay,
        )? {
            RelayPeerResponse::ExecutionLeaseCreated {
                lease,
                relay_peer_protocol_version,
            } => (lease, relay_peer_protocol_version),
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "create remote execution lease",
                    message: format!("unexpected peer response: {other:?}"),
                });
            }
        };
        let cleanup_remote_setup =
            |app: &mut DaemonApp,
             relay_config: &DaemonConfig,
             target: &ClientTarget,
             lease_id: &str,
             leased_agent_id: Option<&str>| {
                if let Some(leased_agent_id) = leased_agent_id {
                    let _ = app.send_remote_binding_request(
                        relay_config,
                        target.clone(),
                        RelayPeerRequest::DestroyLeasedAgent {
                            leased_agent_id: leased_agent_id.to_string(),
                        },
                        use_connected_relay,
                    );
                }
                let _ = app.send_remote_binding_request(
                    relay_config,
                    target.clone(),
                    RelayPeerRequest::DestroyExecutionLease {
                        lease_id: lease_id.to_string(),
                    },
                    use_connected_relay,
                );
            };
        if relay_peer_protocol_version < RELAY_PEER_PROTOCOL_VERSION {
            cleanup_remote_setup(self, &relay_config, &target, &lease.id, None);
            return Err(DaemonError::LocalTransport {
                operation: "create remote execution lease",
                message: format!(
                    "remote worker `{}` uses relay peer protocol {}, but this home kernel requires {}. Upgrade and restart the worker kernel, then retry the remote agent.",
                    worker_kernel.kernel_id,
                    relay_peer_protocol_version,
                    RELAY_PEER_PROTOCOL_VERSION
                ),
            });
        }
        let mut materialized_account = None;
        if crate::provider::canonical_provider_family(agent.provider())
            .is_some_and(|provider| matches!(provider, "codex" | "claude" | "opencode"))
        {
            let materialization_target_kind =
                self.remote_account_materialization_target_kind(&worker_kernel.kernel_id);
            let materialization_target_ref = worker_kernel.kernel_id.clone();
            let account_owner_user_id =
                crate::account_profile::provider_account_authority_owner_user_id(
                    &self.config,
                    agent.owner_user_id(),
                );
            let mut account_materialization =
                match self.provider_account_profiles.export_materialization(
                    &account_owner_user_id,
                    agent.provider(),
                    agent.provider_account_profile(),
                ) {
                    Ok(materialization) => materialization,
                    Err(error) => {
                        let _ = self
                        .provider_account_profiles
                        .update_materialization_status(
                        &account_owner_user_id,
                        agent.provider(),
                        agent.provider_account_profile(),
                        crate::account_profile::ProviderAccountMaterializationStatus {
                            target_kind: materialization_target_kind,
                            target_ref: materialization_target_ref.clone(),
                            state:
                                crate::account_profile::ProviderAccountMaterializationState::Error,
                            observed_at_ms: crate::session::unix_epoch_ms(),
                            last_error: Some("account materialization export failed".to_string()),
                        },
                    );
                        cleanup_remote_setup(self, &relay_config, &target, &lease.id, None);
                        return Err(error);
                    }
                };
            // The home registry aliases the configured Cloud owner to `local`.
            // The worker lease remains scoped to the runtime owner identity, so
            // stamp that identity on the encrypted replica envelope.
            account_materialization.profile.owner_user_id = agent.owner_user_id().to_string();
            let expected_account = account_materialization.profile.clone();
            match self.send_remote_binding_request(
                &relay_config,
                target.clone(),
                RelayPeerRequest::EnsureRemoteProviderAccount {
                    context: crate::transport::relay_peer::RemoteProviderAccountSyncContext {
                        home_kernel_id: self.config.daemon_id.clone(),
                        home_session_id: agent.session_id().to_string(),
                        home_agent_id: agent.id().to_string(),
                        execution_lease_id: lease.id.clone(),
                    },
                    materialization: account_materialization,
                },
                use_connected_relay,
            ) {
                Ok(response)
                    if remote_provider_account_response_matches(&response, &expected_account) =>
                {
                    if let Err(error) =
                        self.provider_account_profiles.update_materialization_status(
                            &account_owner_user_id,
                            agent.provider(),
                            agent.provider_account_profile(),
                            crate::account_profile::ProviderAccountMaterializationStatus {
                                target_kind: materialization_target_kind,
                                target_ref: materialization_target_ref.clone(),
                                state: crate::account_profile::ProviderAccountMaterializationState::Materialized,
                                observed_at_ms: crate::session::unix_epoch_ms(),
                                last_error: None,
                            },
                        )
                    {
                        cleanup_remote_setup(self, &relay_config, &target, &lease.id, None);
                        return Err(error);
                    }
                    materialized_account = Some(expected_account);
                }
                Ok(other) => {
                    let _ = self
                        .provider_account_profiles
                        .update_materialization_status(
                        &account_owner_user_id,
                        agent.provider(),
                        agent.provider_account_profile(),
                        crate::account_profile::ProviderAccountMaterializationStatus {
                            target_kind: materialization_target_kind,
                            target_ref: materialization_target_ref.clone(),
                            state:
                                crate::account_profile::ProviderAccountMaterializationState::Error,
                            observed_at_ms: crate::session::unix_epoch_ms(),
                            last_error: Some("worker rejected account materialization".to_string()),
                        },
                    );
                    cleanup_remote_setup(self, &relay_config, &target, &lease.id, None);
                    return Err(DaemonError::LocalTransport {
                        operation: "materialize remote provider account",
                        message: format!("unexpected peer response: {other:?}"),
                    });
                }
                Err(error) => {
                    let _ = self
                        .provider_account_profiles
                        .update_materialization_status(
                        &account_owner_user_id,
                        agent.provider(),
                        agent.provider_account_profile(),
                        crate::account_profile::ProviderAccountMaterializationStatus {
                            target_kind: materialization_target_kind,
                            target_ref: materialization_target_ref,
                            state:
                                crate::account_profile::ProviderAccountMaterializationState::Error,
                            observed_at_ms: crate::session::unix_epoch_ms(),
                            last_error: Some("account materialization failed".to_string()),
                        },
                    );
                    cleanup_remote_setup(self, &relay_config, &target, &lease.id, None);
                    return Err(error);
                }
            }
        }
        let leased_agent = match self.send_remote_binding_request(
            &relay_config,
            target.clone(),
            RelayPeerRequest::SpawnLeasedAgent {
                lease_id: lease.id.clone(),
                provider: agent.provider().to_string(),
                account_profile: worker_account_profile_for_spawn(
                    agent.provider_account_profile(),
                    materialized_account.as_ref(),
                ),
                model: agent.model().map(ToOwned::to_owned),
                effort: agent.effort().map(ToOwned::to_owned),
                execution_mode: Some(effective_config.mode),
                permission_level: Some(effective_config.permission_level),
                workspace_live_sync_mode: Some(workspace_live_sync_mode),
                worktree_id: worker_worktree_id
                    .or_else(|| agent.worktree_id().map(ToOwned::to_owned)),
                worktree_placement,
            },
            use_connected_relay,
        ) {
            Ok(RelayPeerResponse::LeasedAgentSpawned { leased_agent }) => leased_agent,
            Ok(other) => {
                cleanup_remote_setup(self, &relay_config, &target, &lease.id, None);
                return Err(DaemonError::LocalTransport {
                    operation: "spawn remote leased agent",
                    message: format!("unexpected peer response: {other:?}"),
                });
            }
            Err(error) => {
                cleanup_remote_setup(self, &relay_config, &target, &lease.id, None);
                return Err(error);
            }
        };
        let leased_agent_id = leased_agent.id.clone();
        let lease_id = lease.id.clone();
        let bound = match self.agents.bind_remote_execution(
            agent.id(),
            RemoteAgentBinding {
                worker_kernel_id: worker_kernel.kernel_id.clone(),
                worker_machine_id: worker_kernel.machine_id.clone(),
                execution_lease_id: lease.id,
                leased_agent_id: leased_agent.id,
                active_worker_provider_run_id: None,
                relay_url: relay_override
                    .as_ref()
                    .and_then(|config| config.relay_url.clone()),
                relay_token: relay_override
                    .as_ref()
                    .and_then(|config| config.relay_token.clone()),
                relay_peer_protocol_version: Some(relay_peer_protocol_version),
            },
        ) {
            Ok(bound) => bound,
            Err(error) => {
                cleanup_remote_setup(
                    self,
                    &relay_config,
                    &target,
                    &lease_id,
                    Some(&leased_agent_id),
                );
                return Err(error);
            }
        };
        if let Err(error) = self.ensure_remote_agent_skill_packages(&bound) {
            cleanup_remote_setup(
                self,
                &relay_config,
                &target,
                &lease_id,
                Some(&leased_agent_id),
            );
            return Err(error);
        }
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
        let relay_config = self.relay_config_for_remote_execution(&remote_execution);
        let discovery_config = self.remote_worker_discovery_config(
            &remote_execution.worker_kernel_id,
            relay_config.clone(),
        )?;
        let uses_remote_execution_relay =
            remote_execution.relay_url.is_some() && remote_execution.relay_token.is_some();
        let worker_kernel = self.select_remote_kernel_for_machine_with_config(
            &remote_execution.worker_machine_id,
            agent.provider(),
            &discovery_config,
        )?;
        let rebound = self.bind_remote_agent_to_worker(
            &agent,
            &worker_kernel,
            None,
            None,
            uses_remote_execution_relay.then_some(relay_config),
        )?;
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

    pub(crate) fn refresh_remote_agent_binding_to_worker_kernel(
        &mut self,
        agent_id: &str,
        worker_kernel: &RelayKernelPresence,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self.agents.get_agent(agent_id)?;
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Err(DaemonError::LocalTransport {
                operation: "refresh remote agent binding",
                message: format!("agent `{agent_id}` is not remote-backed"),
            });
        };
        let relay_config = self.relay_config_for_remote_execution(&remote_execution);
        let uses_remote_execution_relay =
            remote_execution.relay_url.is_some() && remote_execution.relay_token.is_some();
        let rebound = self.bind_remote_agent_to_worker(
            &agent,
            worker_kernel,
            None,
            None,
            uses_remote_execution_relay.then_some(relay_config),
        )?;
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
        let relay_override = self.slice_relay_config_for_kernel_ref(machine_ref);
        let relay_config = relay_override
            .clone()
            .unwrap_or_else(|| self.config.clone());
        let discovery_config =
            self.remote_worker_discovery_config(machine_ref, relay_config.clone())?;
        let worker_kernel = self.select_remote_kernel_for_machine_with_config(
            machine_ref,
            agent.provider(),
            &discovery_config,
        )?;
        let worker_worktree_id = self.worker_worktree_id_for_kernel_ref(machine_ref, None);
        self.bind_remote_agent_to_worker(
            &agent,
            &worker_kernel,
            worker_worktree_id,
            None,
            relay_override,
        )
    }

    pub(crate) fn move_agent_to_local(
        &mut self,
        session_id: &str,
        agent_ref: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .agents
            .get_agent(agent_ref)
            .or_else(|_| self.agents.get_agent_by_ref(agent_ref))?;
        if agent.session_id() != session_id {
            return Err(DaemonError::LocalTransport {
                operation: "move agent to local",
                message: format!("agent `{agent_ref}` does not belong to session `{session_id}`"),
            });
        }
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Err(DaemonError::LocalTransport {
                operation: "move agent to local",
                message: format!("agent `{agent_ref}` is already local"),
            });
        };
        if self
            .providers
            .get_run_for_agent(session_id, agent.id())
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "move agent to local",
                message: "agent has a local provider run; stop or destroy the provider run before moving it"
                    .to_string(),
            });
        }
        let relay_config = self.relay_config_for_remote_execution(&remote_execution);
        let target = ClientTarget {
            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let use_connected_relay =
            self.hosted_shared_slice_uses_connected_relay(&remote_execution.worker_kernel_id);
        match self.send_remote_binding_request(
            &relay_config,
            target.clone(),
            RelayPeerRequest::DestroyLeasedAgent {
                leased_agent_id: remote_execution.leased_agent_id.clone(),
            },
            use_connected_relay,
        )? {
            RelayPeerResponse::LeasedAgentDestroyed { .. } => {}
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "destroy remote leased agent before moving local",
                    message: format!("unexpected peer response: {other:?}"),
                });
            }
        }
        match self.send_remote_binding_request(
            &relay_config,
            target,
            RelayPeerRequest::DestroyExecutionLease {
                lease_id: remote_execution.execution_lease_id.clone(),
            },
            use_connected_relay,
        )? {
            RelayPeerResponse::ExecutionLeaseDestroyed { .. } => {}
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "destroy remote execution lease before moving local",
                    message: format!("unexpected peer response: {other:?}"),
                });
            }
        }
        let moved = self.agents.clear_remote_execution(agent.id())?;
        self.durable_state_store().append_event(
            "agent.updated",
            Some(moved.id().to_string()),
            serde_json::json!({
                "agent": &moved,
                "source": "agent_moved_to_local",
            }),
        )?;
        if let Ok(session) = self.sessions.get_session(moved.session_id()) {
            self.update_session_projection(session);
        }
        Ok(moved)
    }

    pub(crate) fn destroy_remote_execution_binding(
        &self,
        remote_execution: &RemoteAgentBinding,
    ) -> Result<(), DaemonError> {
        let relay_config = self.relay_config_for_remote_execution(remote_execution);
        let target = ClientTarget {
            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let use_connected_relay =
            self.hosted_shared_slice_uses_connected_relay(&remote_execution.worker_kernel_id);
        match self.send_remote_binding_request(
            &relay_config,
            target.clone(),
            RelayPeerRequest::DestroyLeasedAgent {
                leased_agent_id: remote_execution.leased_agent_id.clone(),
            },
            use_connected_relay,
        )? {
            RelayPeerResponse::LeasedAgentDestroyed { .. } => {}
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "destroy remote leased agent",
                    message: format!("unexpected peer response: {other:?}"),
                });
            }
        }
        match self.send_remote_binding_request(
            &relay_config,
            target,
            RelayPeerRequest::DestroyExecutionLease {
                lease_id: remote_execution.execution_lease_id.clone(),
            },
            use_connected_relay,
        )? {
            RelayPeerResponse::ExecutionLeaseDestroyed { .. } => Ok(()),
            other => Err(DaemonError::LocalTransport {
                operation: "destroy remote execution lease",
                message: format!("unexpected peer response: {other:?}"),
            }),
        }
    }

    fn ensure_remote_agent_skill_packages(
        &mut self,
        agent: &AgentInstance,
    ) -> Result<(), DaemonError> {
        let Some(remote_execution) = agent.remote_execution() else {
            return Ok(());
        };
        let skill_grants = agent.skill_grants();
        if skill_grants.is_empty() {
            return Ok(());
        }
        let _ = self.sessions.get_session(agent.session_id())?;
        let roots = crate::skill::CharioxSkillRegistry::user_root()
            .map(|root| vec![root])
            .unwrap_or_default();
        let registry = crate::skill::CharioxSkillRegistry::new(roots);
        let packages = skill_grants
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
        let response = self.send_remote_binding_request(
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
            self.hosted_shared_slice_uses_connected_relay(&remote_execution.worker_kernel_id),
        )?;
        match response {
            RelayPeerResponse::RemoteSkillPackagesEnsured { .. } => Ok(()),
            other => Err(DaemonError::LocalTransport {
                operation: "ensure remote agent skill packages",
                message: format!("unexpected remote skill sync response: {other:?}"),
            }),
        }
    }

    fn select_remote_kernel_for_machine_with_config(
        &self,
        machine_ref: &str,
        provider: &str,
        relay_config: &DaemonConfig,
    ) -> Result<RelayKernelPresence, DaemonError> {
        let machine_ref = crate::config::DaemonConfig::resolve_registered_machine_ref(machine_ref)
            .unwrap_or_else(|| machine_ref.to_string());
        if relay_config.relay_url == self.config.relay_url
            && relay_config.relay_token == self.config.relay_token
        {
            let (_, projected_kernels) = self.remote_relay_inventory_projection_store().snapshot();
            if let Some(kernel) = select_remote_kernel(projected_kernels, &machine_ref, provider) {
                return Ok(kernel);
            }
        }
        let kernels = self.block_on_relay_future(
            relay_discovery::list_live_kernels_for_machine(relay_config, &machine_ref),
        )?;
        let message = no_remote_kernel_available_message(&kernels, &machine_ref, provider);
        select_remote_kernel(kernels, &machine_ref, provider).ok_or_else(|| {
            DaemonError::NoRemoteKernelAvailable {
                machine_ref,
                provider: provider.to_string(),
                message,
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
        let mut last_error = None;
        for attempt in 0..REMOTE_KERNEL_REF_DISCOVERY_ATTEMPTS {
            match self
                .block_on_relay_future(relay_discovery::get_live_kernel(relay_config, kernel_ref))
            {
                Ok(kernel) => {
                    return ensure_kernel_can_host_provider(kernel, kernel_ref, provider);
                }
                Err(error) => {
                    last_error = Some(error);
                    if attempt + 1 < REMOTE_KERNEL_REF_DISCOVERY_ATTEMPTS {
                        std::thread::sleep(Duration::from_millis(
                            REMOTE_KERNEL_REF_DISCOVERY_RETRY_DELAY_MS,
                        ));
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| DaemonError::LocalTransport {
            operation: "select remote kernel",
            message: format!("kernel `{kernel_ref}` did not appear"),
        }))
    }

    fn slice_relay_config_for_kernel_ref(&self, kernel_ref: &str) -> Option<DaemonConfig> {
        let slice = self.slices.resolve_by_worker_kernel_ref(kernel_ref)?;
        self.config.slice_relay_override(&slice)
    }

    fn hosted_shared_slice_uses_connected_relay(&self, kernel_ref: &str) -> bool {
        self.slices
            .resolve_by_worker_kernel_ref(kernel_ref)
            .and_then(|slice| slice.relay_endpoint)
            .is_some_and(|endpoint| {
                !endpoint.private && self.config.relay_url_uses_cloud_profile(&endpoint.url)
            })
    }

    fn remote_account_materialization_target_kind(
        &self,
        kernel_ref: &str,
    ) -> crate::account_profile::ProviderAccountMaterializationTargetKind {
        if self
            .slices
            .resolve_by_worker_kernel_ref(kernel_ref)
            .is_some()
        {
            crate::account_profile::ProviderAccountMaterializationTargetKind::Slice
        } else {
            crate::account_profile::ProviderAccountMaterializationTargetKind::Worker
        }
    }

    fn remote_worker_discovery_config(
        &self,
        kernel_ref: &str,
        mut config: DaemonConfig,
    ) -> Result<DaemonConfig, DaemonError> {
        if !self.hosted_shared_slice_uses_connected_relay(kernel_ref) {
            return Ok(config);
        }
        let profile = config
            .cloud_relay
            .clone()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "discover hosted slice worker",
                message: "hosted shared slice requires a Cloud relay profile".to_string(),
            })?;
        let owner_kernel_ref = self.config.daemon_id.clone();
        let worker_kernel_ref = kernel_ref.to_string();
        let token = self.block_on_relay_future(async move {
            crate::runtime::cloud_api_client::issue_cloud_slice_discovery_token(
                &profile,
                &owner_kernel_ref,
                &worker_kernel_ref,
            )
            .await
        })?;
        config.relay_token = Some(token.token);
        Ok(config)
    }

    fn send_remote_binding_request(
        &self,
        relay_config: &DaemonConfig,
        target: ClientTarget,
        request: RelayPeerRequest,
        use_connected_relay: bool,
    ) -> Result<RelayPeerResponse, DaemonError> {
        if use_connected_relay {
            let relay_state = self.relay_client_state();
            self.block_on_relay_future(send_peer_request_via_connected_relay(
                relay_config,
                &relay_state,
                target,
                request,
            ))
        } else {
            self.block_on_relay_future(send_peer_request_via_temporary_connection(
                relay_config,
                target,
                request,
            ))
        }
    }

    fn worker_worktree_id_for_kernel_ref(
        &self,
        kernel_ref: &str,
        requested_worktree_id: Option<String>,
    ) -> Option<String> {
        self.slices
            .resolve_by_worker_kernel_ref(kernel_ref)
            .map(|slice| {
                slice
                    .development_publication
                    .map(|publication| publication.primary_repository_path)
                    .unwrap_or_else(|| "/workspace".to_string())
            })
            .or(requested_worktree_id)
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
            config.apply_remote_relay_override(relay_url, relay_token);
        }
        config
    }

    pub(crate) fn ensure_remote_agent_binding_protocol(
        &self,
        remote_execution: &RemoteAgentBinding,
    ) -> Result<(), DaemonError> {
        if remote_execution.relay_peer_protocol_compatible() {
            return Ok(());
        }
        Err(DaemonError::LocalTransport {
            operation: "dispatch remote agent prompt",
            message: format!(
                "remote worker `{}` has an incompatible or legacy relay peer protocol {:?}; rebind the remote agent before dispatch (current protocol {})",
                remote_execution.worker_kernel_id,
                remote_execution.relay_peer_protocol_version,
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        })
    }

    fn remember_remote_worker_public_key(
        &self,
        relay_config: &DaemonConfig,
        worker_kernel: &RelayKernelPresence,
    ) {
        let Some(relay_url) = relay_config.relay_url.clone() else {
            return;
        };
        let relay_state = self.relay_client_state();
        let kernel_id = worker_kernel.kernel_id.clone();
        let public_key = worker_kernel.public_key.clone();
        let _ = self.block_on_relay_future(async move {
            let mut state = relay_state.write().await;
            if state.connected()
                && state.connected_relay_url().as_deref() == Some(relay_url.as_str())
            {
                state.remember_peer_public_key(kernel_id, public_key);
            }
            Ok(())
        });
    }
}

fn remote_provider_account_response_matches(
    response: &RelayPeerResponse,
    expected: &crate::account_profile::ProviderAccountReplicaMetadata,
) -> bool {
    matches!(
        response,
        RelayPeerResponse::RemoteProviderAccountEnsured {
            provider,
            account_profile,
        } if provider == &expected.provider && account_profile == &expected.profile_id
    )
}

fn worker_account_profile_for_spawn(
    requested_account_profile: &str,
    materialized_account: Option<&crate::account_profile::ProviderAccountReplicaMetadata>,
) -> String {
    materialized_account
        .map(|account| account.profile_id.clone())
        .unwrap_or_else(|| requested_account_profile.to_string())
}

fn app_mcp_registry_roots(workspace_id: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(not(test))]
    let _ = workspace_id;
    #[cfg(test)]
    if !workspace_id.trim().is_empty() {
        roots.push(crate::mcp::CharioxMcpRegistry::project_root(workspace_id));
    }
    if let Some(root) = crate::mcp::CharioxMcpRegistry::user_root() {
        roots.push(root);
    }
    roots
}

fn app_script_registry_roots(workspace_id: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(not(test))]
    let _ = workspace_id;
    #[cfg(test)]
    if !workspace_id.trim().is_empty() {
        roots.push(crate::script::CharioxScriptRegistry::project_root(
            workspace_id,
        ));
    }
    if let Some(root) = crate::script::CharioxScriptRegistry::user_root() {
        roots.push(root);
    }
    roots
}

fn app_skill_registry_roots(workspace_id: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(not(test))]
    let _ = workspace_id;
    #[cfg(test)]
    if !workspace_id.trim().is_empty() {
        roots.push(crate::skill::CharioxSkillRegistry::project_root(
            workspace_id,
        ));
    }
    if let Some(root) = crate::skill::CharioxSkillRegistry::user_root() {
        roots.push(root);
    }
    roots
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use crate::agent::RemoteAgentBinding;
    use crate::app::DaemonApp;
    use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult,
        RuntimeProviderRun,
    };
    use crate::session::CreateSessionRequest;
    use crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;

    #[test]
    fn remote_account_receipt_matches_the_materialized_profile_id_not_its_alias() {
        let expected = crate::account_profile::ProviderAccountReplicaMetadata {
            owner_user_id: "user-home".to_string(),
            provider: "codex".to_string(),
            profile_id: "codex-1-vfx4dshw".to_string(),
            label: "Codex".to_string(),
            origin: crate::account_profile::ProviderAccountProfileOrigin::Default,
            is_default: true,
        };

        assert!(super::remote_provider_account_response_matches(
            &crate::transport::relay_peer::RelayPeerResponse::RemoteProviderAccountEnsured {
                provider: "codex".to_string(),
                account_profile: "codex-1-vfx4dshw".to_string(),
            },
            &expected,
        ));
        assert!(!super::remote_provider_account_response_matches(
            &crate::transport::relay_peer::RelayPeerResponse::RemoteProviderAccountEnsured {
                provider: "codex".to_string(),
                account_profile: "default".to_string(),
            },
            &expected,
        ));
    }

    #[test]
    fn remote_agent_launch_uses_the_materialized_profile_id() {
        let materialized = crate::account_profile::ProviderAccountReplicaMetadata {
            owner_user_id: "user-home".to_string(),
            provider: "codex".to_string(),
            profile_id: "codex-1-vfx4dshw".to_string(),
            label: "Codex".to_string(),
            origin: crate::account_profile::ProviderAccountProfileOrigin::Default,
            is_default: true,
        };

        assert_eq!(
            super::worker_account_profile_for_spawn("default", Some(&materialized)),
            "codex-1-vfx4dshw"
        );
    }

    #[test]
    fn restored_stale_or_unversioned_binding_is_rejected_until_rebound() {
        let legacy = RemoteAgentBinding {
            worker_kernel_id: "worker-kernel".to_string(),
            worker_machine_id: "worker-machine".to_string(),
            execution_lease_id: "lease-1".to_string(),
            leased_agent_id: "agent-1".to_string(),
            active_worker_provider_run_id: None,
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: None,
        };
        assert!(!legacy.relay_peer_protocol_compatible());
        let stale = RemoteAgentBinding {
            relay_peer_protocol_version: Some(38),
            ..legacy.clone()
        };
        assert!(!stale.relay_peer_protocol_compatible());
        let current = RemoteAgentBinding {
            relay_peer_protocol_version: Some(RELAY_PEER_PROTOCOL_VERSION),
            ..legacy
        };
        assert!(current.relay_peer_protocol_compatible());
        let newer = RemoteAgentBinding {
            relay_peer_protocol_version: Some(RELAY_PEER_PROTOCOL_VERSION + 1),
            ..current
        };
        assert!(newer.relay_peer_protocol_compatible());
    }

    fn cloud_relay_profile(relay_url: &str) -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            api_url: "https://cloud.example.test".to_string(),
            email: "user@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-1".to_string(),
            account_slug: "acct".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: relay_url.to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: None,
            client_alias: None,
            machine_id: Some("machine-1".to_string()),
            machine_alias: None,
            machine_credential: Some("machine-secret".to_string()),
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: Some(42),
        }
    }

    fn accept_cloud_request(
        listener: &std::net::TcpListener,
        timeout: std::time::Duration,
    ) -> std::io::Result<(std::net::TcpStream, std::net::SocketAddr)> {
        listener.set_nonblocking(true)?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match listener.accept() {
                Ok(connection) => return Ok(connection),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "timed out waiting for Cloud fixture request",
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 4096];
            let bytes = stream.read(&mut chunk).expect("read Cloud request");
            if bytes == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..bytes]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        String::from_utf8(request).expect("Cloud request should be UTF-8")
    }

    #[test]
    fn cloud_request_fixture_accept_is_bounded() {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("Cloud fixture should bind");

        let error = accept_cloud_request(&listener, std::time::Duration::from_millis(20))
            .expect_err("fixture accept should time out without a request");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    }

    #[test]
    fn native_remote_prompt_routes_granted_mcp_as_worker_requirement_only() {
        let workspace = std::env::temp_dir().join(format!(
            "chariox-native-remote-prompt-capabilities-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&workspace).expect("workspace should create");
        let mcp_name = format!(
            "native-remote-prompt-mcp-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        );
        let mcp = crate::mcp::CharioxMcpServerConfig::stdio(
            &mcp_name,
            std::env::current_exe()
                .expect("current test executable should resolve")
                .display()
                .to_string(),
            Vec::new(),
        );
        crate::mcp::CharioxMcpRegistry::new(vec![crate::mcp::CharioxMcpRegistry::project_root(
            &workspace,
        )])
        .install(&mcp)
        .expect("project MCP should install");

        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let workspace_id = workspace.to_string_lossy().to_string();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(&workspace_id, &workspace_id))
            .expect("session should create");
        let agent = app
            .agents()
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel".to_string(),
                    worker_machine_id: "worker-machine".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("worker-provider-run-1".to_string()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to worker");
        let agent = app
            .agents()
            .grant_mcp(agent.id(), mcp_name.clone())
            .expect("MCP grant should apply");

        let (required_mcps, required_skills, manifest) = app
            .remote_prompt_capabilities_for_agent(&agent)
            .expect("Chariox prompt capabilities should resolve");
        assert!(required_mcps.is_empty());
        assert!(required_skills.is_none());
        assert_eq!(
            manifest.home_proxy_mcp_server_names().collect::<Vec<_>>(),
            vec![mcp_name.as_str()]
        );

        let request = LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
        .with_agent_id(agent.id())
        .with_client_interface(ProviderClientInterface::NativeTui);
        app.update_provider_run_projection(RuntimeProviderRun::new(
            "leased:leased-agent-1:worker-provider-run-1",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "claude:claude:claude-sonnet-4-6".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: Some(workspace.clone()),
                structured_endpoint: None,
            },
        ));

        let (required_mcps, required_skills, manifest) = app
            .remote_prompt_capabilities_for_agent(&agent)
            .expect("native prompt capabilities should resolve");
        assert_eq!(required_mcps.len(), 1);
        assert_eq!(required_mcps[0].config.name, mcp_name);
        assert!(required_skills.is_some_and(|skills| skills.is_empty()));
        assert!(manifest.home_proxy_mcp_server_names().next().is_none());

        std::fs::remove_dir_all(workspace).expect("workspace should clean up");
    }

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
                    display_mode: crate::slice::SliceDisplayMode::Headed,
                    display_backend: Default::default(),
                    workspace_id: None,
                    worktree_id: None,
                    workspace_mount: Some("/repo".to_string()),
                    development: None,
                    worker_kernel_ref: None,
                    display_url: Some("http://127.0.0.1:6080".to_string()),
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 42,
                },
            )
            .expect("slice should create");

        let relay_config = app
            .slice_relay_config_for_kernel_ref(&slice.worker_kernel_ref)
            .expect("slice worker ref should have relay config");
        assert_eq!(
            app.worker_worktree_id_for_kernel_ref(
                &slice.worker_kernel_ref,
                Some("/host/worktree".to_string()),
            )
            .as_deref(),
            Some("/workspace")
        );
        app.slices()
            .set_development_publication(
                &slice.id,
                crate::slice::SliceDevelopmentPublication {
                    publication_id: "development".to_string(),
                    destination_root: "/state/development".to_string(),
                    primary_repository_path: "/state/development/primary".to_string(),
                    repository_paths: vec![
                        "/state/development/primary".to_string(),
                        "/state/development/supporting".to_string(),
                    ],
                },
                43,
            )
            .expect("slice publication should update");
        assert_eq!(
            app.worker_worktree_id_for_kernel_ref(
                &slice.worker_kernel_ref,
                Some("/host/worktree".to_string()),
            )
            .as_deref(),
            Some("/state/development/primary")
        );
        assert_eq!(
            app.worker_worktree_id_for_kernel_ref(
                "standard-worker",
                Some("/worker/worktree".to_string()),
            )
            .as_deref(),
            Some("/worker/worktree")
        );
        let ports = slice
            .local_docker_ports
            .expect("local Docker slice should have assigned ports");

        let expected_relay_url = format!("ws://127.0.0.1:{}", ports.relay);
        assert_eq!(
            relay_config.relay_url.as_deref(),
            Some(expected_relay_url.as_str())
        );
        assert_eq!(
            relay_config.relay_token.as_deref(),
            Some("slice-local-daemon-test-slice-1")
        );
        assert!(relay_config.cloud_relay.is_none());
    }

    #[test]
    fn slice_worker_machine_ids_resolve_to_private_relay_config() {
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
                    display_mode: crate::slice::SliceDisplayMode::Headed,
                    display_backend: Default::default(),
                    workspace_id: None,
                    worktree_id: None,
                    workspace_mount: Some("/repo".to_string()),
                    development: None,
                    worker_kernel_ref: None,
                    display_url: Some("http://127.0.0.1:6080".to_string()),
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 42,
                },
            )
            .expect("slice should create");
        app.slices()
            .set_worker_presence(
                &slice.id,
                Some("slice-kernel-1".to_string()),
                Some("slice:slice-1".to_string()),
                vec!["codex".to_string()],
                43,
            )
            .expect("slice worker presence should update");

        let relay_config = app
            .slice_relay_config_for_kernel_ref("slice:slice-1")
            .expect("slice worker machine id should have relay config");
        assert_eq!(
            app.worker_worktree_id_for_kernel_ref(
                "slice:slice-1",
                Some("/host/worktree".to_string()),
            )
            .as_deref(),
            Some("/workspace")
        );
        let ports = slice
            .local_docker_ports
            .expect("local Docker slice should have assigned ports");

        let expected_relay_url = format!("ws://127.0.0.1:{}", ports.relay);
        assert_eq!(
            relay_config.relay_url.as_deref(),
            Some(expected_relay_url.as_str())
        );
        assert_eq!(
            relay_config.relay_token.as_deref(),
            Some("slice-local-daemon-test-slice-1")
        );
        assert!(relay_config.cloud_relay.is_none());
    }

    #[test]
    fn slice_worker_kernel_refs_resolve_to_recorded_shared_relay_config() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("ws://127.0.0.1:54909".to_string());
        config.relay_token = Some("home-relay-token".to_string());
        let app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let slice = app
            .slices()
            .create(
                &app.config().daemon_id,
                &app.config().host_machine_id,
                crate::slice::CreateSliceInput {
                    name: "linux-dev".to_string(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".to_string(),
                    display_mode: crate::slice::SliceDisplayMode::Headed,
                    display_backend: Default::default(),
                    workspace_id: None,
                    worktree_id: None,
                    workspace_mount: Some("/repo".to_string()),
                    development: None,
                    worker_kernel_ref: None,
                    display_url: Some("http://127.0.0.1:6080".to_string()),
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 42,
                },
            )
            .expect("slice should create");
        app.slices()
            .set_relay_endpoint(
                &slice.id,
                Some(crate::slice::SliceRelayEndpoint {
                    url: "ws://127.0.0.1:54909".to_string(),
                    private: false,
                }),
                43,
            )
            .expect("slice relay endpoint should update");

        let relay_config = app
            .slice_relay_config_for_kernel_ref(&slice.worker_kernel_ref)
            .expect("slice worker ref should have relay config");

        assert_eq!(
            relay_config.relay_url.as_deref(),
            Some("ws://127.0.0.1:54909")
        );
        assert_eq!(
            relay_config.relay_token.as_deref(),
            Some("home-relay-token")
        );
        assert!(relay_config.cloud_relay.is_none());
    }

    #[test]
    fn slice_worker_kernel_refs_use_home_cloud_relay_profile_for_hosted_shared_relay() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("short-lived-token".to_string());
        config.cloud_relay = Some(cloud_relay_profile("wss://relay.example.test"));
        let app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let slice = app
            .slices()
            .create(
                &app.config().daemon_id,
                &app.config().host_machine_id,
                crate::slice::CreateSliceInput {
                    name: "linux-dev".to_string(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".to_string(),
                    display_mode: crate::slice::SliceDisplayMode::Headed,
                    display_backend: Default::default(),
                    workspace_id: None,
                    worktree_id: None,
                    workspace_mount: Some("/repo".to_string()),
                    development: None,
                    worker_kernel_ref: None,
                    display_url: Some("http://127.0.0.1:6080".to_string()),
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 42,
                },
            )
            .expect("slice should create");
        app.slices()
            .set_relay_endpoint(
                &slice.id,
                Some(crate::slice::SliceRelayEndpoint {
                    url: "wss://relay.example.test".to_string(),
                    private: false,
                }),
                43,
            )
            .expect("slice relay endpoint should update");

        assert!(app
            .slice_relay_config_for_kernel_ref(&slice.worker_kernel_ref)
            .is_none());
        assert!(app.hosted_shared_slice_uses_connected_relay(&slice.worker_kernel_ref));
        assert_eq!(
            app.remote_account_materialization_target_kind(&slice.worker_kernel_ref),
            crate::account_profile::ProviderAccountMaterializationTargetKind::Slice
        );
    }

    #[test]
    fn hosted_shared_slice_discovery_uses_scoped_metadata_token() {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("Cloud fixture should bind");
        let address = listener.local_addr().expect("Cloud fixture address");
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) =
                accept_cloud_request(&listener, std::time::Duration::from_secs(2))
                    .expect("accept Cloud request");
            let request = read_http_request(&mut stream);
            let body = r#"{"token":"slice-metadata-token","expiresAt":"2099-01-01T00:00:00Z"}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .expect("write Cloud response");
            request
        });

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("home-runtime-token".to_string());
        let mut profile = cloud_relay_profile("wss://relay.example.test");
        profile.api_url = format!("http://{address}");
        config.cloud_relay = Some(profile);
        let app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let slice = app
            .slices()
            .create(
                &app.config().daemon_id,
                &app.config().host_machine_id,
                crate::slice::CreateSliceInput {
                    name: "hosted-linux-dev".to_string(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".to_string(),
                    display_mode: crate::slice::SliceDisplayMode::Headed,
                    display_backend: crate::slice::SliceDisplayBackend::default(),
                    workspace_id: None,
                    worktree_id: None,
                    workspace_mount: Some("/repo".to_string()),
                    development: None,
                    worker_kernel_ref: None,
                    display_url: None,
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 42,
                },
            )
            .expect("slice should create");
        app.slices()
            .set_relay_endpoint(
                &slice.id,
                Some(crate::slice::SliceRelayEndpoint {
                    url: "wss://relay.example.test".to_string(),
                    private: false,
                }),
                43,
            )
            .expect("slice relay endpoint should update");

        let discovery = app
            .remote_worker_discovery_config(&slice.worker_kernel_ref, app.config().clone())
            .expect("hosted slice discovery config should issue a scoped token");
        assert_eq!(
            discovery.relay_token.as_deref(),
            Some("slice-metadata-token")
        );
        let request = fixture.join().expect("Cloud fixture should finish");
        assert!(request.starts_with("POST /relay/token HTTP/1.1"));
        assert!(request.contains(&format!(
            r#""subject":"slice-discovery:{}:{}""#,
            app.config().daemon_id,
            slice.worker_kernel_ref
        )));
        assert!(request.contains(r#""allowUnpairedClientSubject":true"#));
        assert!(request.contains(r#""allowedActions":["client.metadata.read"]"#));
    }

    #[test]
    fn ordinary_remote_worker_keeps_temporary_relay_and_worker_materialization() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("refreshable-token".to_string());
        config.cloud_relay = Some(cloud_relay_profile("wss://relay.example.test"));
        let app = DaemonApp::bootstrap(config).expect("daemon should boot");

        assert!(!app.hosted_shared_slice_uses_connected_relay("kernel-remote"));
        assert_eq!(
            app.remote_account_materialization_target_kind("kernel-remote"),
            crate::account_profile::ProviderAccountMaterializationTargetKind::Worker
        );
    }

    #[test]
    fn remote_execution_matching_home_cloud_relay_keeps_refreshable_profile() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("refreshable-token".to_string());
        config.cloud_relay = Some(cloud_relay_profile("wss://relay.example.test/"));
        let app = DaemonApp::bootstrap(config).expect("daemon should boot");

        let relay_config = app.relay_config_for_remote_execution(&RemoteAgentBinding {
            worker_kernel_id: "worker-kernel".to_string(),
            worker_machine_id: "worker-machine".to_string(),
            execution_lease_id: "lease-1".to_string(),
            leased_agent_id: "leased-agent-1".to_string(),
            active_worker_provider_run_id: None,
            relay_url: Some("wss://relay.example.test".to_string()),
            relay_token: Some("short-lived-token".to_string()),
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        });

        assert_eq!(
            relay_config.relay_token.as_deref(),
            Some("refreshable-token")
        );
        assert!(relay_config.cloud_relay.is_some());
    }

    #[test]
    fn remote_execution_different_relay_uses_binding_token() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".to_string());
        config.relay_token = Some("refreshable-token".to_string());
        config.cloud_relay = Some(cloud_relay_profile("wss://relay.example.test"));
        let app = DaemonApp::bootstrap(config).expect("daemon should boot");

        let relay_config = app.relay_config_for_remote_execution(&RemoteAgentBinding {
            worker_kernel_id: "worker-kernel".to_string(),
            worker_machine_id: "worker-machine".to_string(),
            execution_lease_id: "lease-1".to_string(),
            leased_agent_id: "leased-agent-1".to_string(),
            active_worker_provider_run_id: None,
            relay_url: Some("ws://127.0.0.1:54909".to_string()),
            relay_token: Some("binding-token".to_string()),
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        });

        assert_eq!(
            relay_config.relay_url.as_deref(),
            Some("ws://127.0.0.1:54909")
        );
        assert_eq!(relay_config.relay_token.as_deref(), Some("binding-token"));
        assert!(relay_config.cloud_relay.is_none());
    }
}
