use super::*;

impl KernelRuntimeState {
    pub(crate) async fn grant_agent_extension(
        &self,
        agent_ref: &str,
        grant: crate::extension::ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        match grant.kind {
            crate::extension::ExtensionKind::Mcp => {
                self.grant_agent_mcp(agent_ref, grant.name, caller_user_id)
                    .await
            }
            crate::extension::ExtensionKind::Skill => {
                self.grant_agent_skill(agent_ref, grant.name, caller_user_id)
                    .await
            }
            crate::extension::ExtensionKind::Script => {
                self.ensure_agent_extension_tool_names_available(
                    agent_ref,
                    &grant,
                    caller_user_id,
                )?;
                let agent =
                    self.owned
                        .grant_agent_extension(agent_ref, grant.clone(), caller_user_id)?;
                self.append_agent_durable_event(
                    "agent.extension_granted",
                    &agent,
                    Some(&format!("script:{}", grant.name)),
                )
                .await?;
                Ok(agent)
            }
            crate::extension::ExtensionKind::Connector => {
                self.ensure_agent_extension_tool_names_available(
                    agent_ref,
                    &grant,
                    caller_user_id,
                )?;
                let agent =
                    self.owned
                        .grant_agent_extension(agent_ref, grant.clone(), caller_user_id)?;
                self.append_agent_durable_event(
                    "agent.extension_granted",
                    &agent,
                    Some(&format!("connector:{}", grant.name)),
                )
                .await?;
                Ok(agent)
            }
        }
    }

    fn ensure_agent_extension_tool_names_available(
        &self,
        agent_ref: &str,
        proposed: &crate::extension::ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<(), DaemonError> {
        let agent = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        self.owned
            .ensure_agent_owner(agent.id(), caller_user_id, "grant agent extension")?;
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let mut reserved = static_runtime_tool_names();
        let script_registry = crate::script::ArrobaScriptRegistry::new(
            crate::runtime::capability_registry::script_registry_roots(Some(
                session.workspace_id(),
            ))?,
        );
        let connector_registry = crate::connector::ArrobaConnectorRegistry::user()?;
        for grant in agent.extension_grants() {
            if grant.kind == proposed.kind && grant.name == proposed.name {
                continue;
            }
            match grant.kind {
                crate::extension::ExtensionKind::Script => {
                    if let Some(script) = script_registry.get(&grant.name)? {
                        reserved.insert(script.name);
                    }
                }
                crate::extension::ExtensionKind::Connector => {
                    if let Some(connector) = connector_registry.get(&grant.name)? {
                        let max_safety =
                            crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())?;
                        reserved.extend(connector.allowed_operation_tool_names(max_safety));
                    }
                }
                crate::extension::ExtensionKind::Mcp | crate::extension::ExtensionKind::Skill => {}
            }
        }
        let proposed_names = match proposed.kind {
            crate::extension::ExtensionKind::Script => script_registry
                .get(&proposed.name)?
                .map(|script| vec![script.name])
                .unwrap_or_default(),
            crate::extension::ExtensionKind::Connector => {
                let Some(connector) = connector_registry.get(&proposed.name)? else {
                    return Ok(());
                };
                let max_safety =
                    crate::connector::ConnectorSafety::parse(proposed.max_safety.as_deref())?;
                connector.allowed_operation_tool_names(max_safety)
            }
            crate::extension::ExtensionKind::Mcp | crate::extension::ExtensionKind::Skill => {
                Vec::new()
            }
        };
        for name in proposed_names {
            if reserved.contains(&name) {
                return Err(DaemonError::LocalTransport {
                    operation: "agent.extension.grant",
                    message: format!("extension tool name `{name}` is already in use"),
                });
            }
            reserved.insert(name);
        }
        Ok(())
    }

    pub(crate) async fn revoke_agent_extension(
        &self,
        agent_ref: &str,
        kind: crate::extension::ExtensionKind,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        match kind {
            crate::extension::ExtensionKind::Mcp => {
                self.revoke_agent_mcp(agent_ref, name, caller_user_id).await
            }
            crate::extension::ExtensionKind::Skill => {
                self.revoke_agent_skill(agent_ref, name, caller_user_id)
                    .await
            }
            crate::extension::ExtensionKind::Script => {
                let agent = self.owned.revoke_agent_extension(
                    agent_ref,
                    crate::extension::ExtensionKind::Script,
                    name,
                    caller_user_id,
                )?;
                self.append_agent_durable_event(
                    "agent.extension_revoked",
                    &agent,
                    Some(&format!("script:{name}")),
                )
                .await?;
                Ok(agent)
            }
            crate::extension::ExtensionKind::Connector => {
                let agent = self.owned.revoke_agent_extension(
                    agent_ref,
                    crate::extension::ExtensionKind::Connector,
                    name,
                    caller_user_id,
                )?;
                self.append_agent_durable_event(
                    "agent.extension_revoked",
                    &agent,
                    Some(&format!("connector:{name}")),
                )
                .await?;
                Ok(agent)
            }
        }
    }

    pub(crate) async fn grant_agent_mcp(
        &self,
        agent_ref: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let existing = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        self.owned
            .ensure_agent_owner(existing.id(), caller_user_id, "grant agent capability")?;
        if existing.remote_execution().is_some() && !existing.mcp_grants().contains(&name) {
            let mut checked = existing.clone();
            checked.grant_mcp(name.clone());
            self.ensure_remote_mcp_availability_for_agent(&checked)
                .await?;
        }
        let agent = self
            .owned
            .grant_agent_mcp(agent_ref, name.clone(), caller_user_id)?;
        self.append_agent_durable_event("agent.mcp_granted", &agent, Some(&name))
            .await?;
        let _ = self
            .apply_provider_reload_policy(ProviderReloadTrigger::AgentMcpChanged {
                session_id: agent.session_id().to_string(),
                agent_id: agent.id().to_string(),
                name,
            })
            .await?;
        Ok(agent)
    }

    pub(crate) async fn revoke_agent_mcp(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .revoke_agent_mcp(agent_ref, name, caller_user_id)?;
        self.append_agent_durable_event("agent.mcp_revoked", &agent, Some(name))
            .await?;
        let _ = self
            .apply_provider_reload_policy(ProviderReloadTrigger::AgentMcpChanged {
                session_id: agent.session_id().to_string(),
                agent_id: agent.id().to_string(),
                name: name.to_string(),
            })
            .await?;
        Ok(agent)
    }

    pub(crate) async fn grant_agent_skill(
        &self,
        agent_ref: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .grant_agent_skill(agent_ref, name.clone(), caller_user_id)?;
        self.append_agent_durable_event("agent.skill_granted", &agent, Some(&name))
            .await?;
        self.ensure_remote_skill_packages_for_agent(&agent).await?;
        Ok(agent)
    }

    pub(crate) async fn revoke_agent_skill(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .revoke_agent_skill(agent_ref, name, caller_user_id)?;
        self.append_agent_durable_event("agent.skill_revoked", &agent, Some(name))
            .await?;
        Ok(agent)
    }

    pub(crate) async fn update_session_config(
        &self,
        session_id: &str,
        attachment_id: &str,
        values: std::collections::BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<crate::session::SessionConfigState, DaemonError> {
        self.owned
            .update_session_config(session_id, attachment_id, values, requires_idle)
    }

    pub(crate) async fn update_agent_config(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        execution_mode_override: Option<Option<crate::provider::AgentExecutionMode>>,
        permission_level_override: Option<Option<crate::provider::AgentPermissionLevel>>,
        workspace_id: Option<Option<String>>,
        worktree_id: Option<Option<String>>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let update = self.owned.update_agent_config(
            session_id,
            agent_id,
            caller_user_id,
            execution_mode_override.clone(),
            permission_level_override.clone(),
            workspace_id,
            worktree_id,
        )?;
        for provider_run_id in update.terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            self.owned
                .remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        let mut agent = update.agent;
        if let Some(remote_update) = update.remote_update {
            let mut config = self.config_snapshot().await;
            if let (Some(relay_url), Some(relay_token)) = (
                remote_update.relay_url.clone(),
                remote_update.relay_token.clone(),
            ) {
                config.relay_url = Some(relay_url);
                config.relay_token = Some(relay_token);
                config.cloud_relay = None;
            }
            match tokio::time::timeout(
                Duration::from_secs(5),
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &config,
                    ClientTarget {
                        daemon_id: Some(remote_update.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::UpdateLeasedAgentConfig {
                        leased_agent_id: remote_update.leased_agent_id,
                        execution_mode: remote_update.execution_mode,
                        permission_level: remote_update.permission_level,
                    },
                ),
            )
            .await
            {
                Ok(Ok(RelayPeerResponse::LeasedAgentConfigUpdated { .. })) => {}
                Ok(Ok(other)) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "update remote leased agent config",
                        message: format!("unexpected remote config response: {other:?}"),
                    });
                }
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "update remote leased agent config",
                        message: "timed out waiting for remote worker config update".to_string(),
                    });
                }
            }
            agent = self.owned.commit_remote_agent_config_update(
                session_id,
                agent_id,
                execution_mode_override,
                permission_level_override,
            )?;
        }
        Ok(agent)
    }

    pub(crate) async fn update_agent_profile(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        provider: Option<String>,
        model: Option<String>,
        effort: Option<Option<String>>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let (agent, terminated_run_ids) = self.owned.update_agent_profile(
            session_id,
            agent_id,
            caller_user_id,
            provider,
            model,
            effort,
        )?;
        for provider_run_id in terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            self.owned
                .remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        Ok(agent)
    }

    pub(crate) async fn alias_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        alias: Option<String>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .alias_agent(session_id, agent_id, caller_user_id, alias)
    }

    pub(crate) async fn update_agent_substitutes(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        action: crate::local::AgentSubstituteAction,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .update_agent_substitutes(session_id, agent_id, caller_user_id, action)
    }

    pub(crate) async fn ensure_agent_owner(
        &self,
        agent_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .ensure_agent_owner(agent_id, caller_user_id, operation)
    }
}

fn static_runtime_tool_names() -> std::collections::BTreeSet<String> {
    crate::transport::runtime_tools::managed_io_runtime_tool_specs()
        .into_iter()
        .chain(crate::transport::runtime_tools::extension_runtime_tool_specs())
        .chain(crate::transport::runtime_tools::credential_runtime_tool_specs())
        .chain(crate::transport::runtime_tools::workflow_runtime_tool_specs())
        .chain(crate::transport::runtime_tools::slice_runtime_tool_specs())
        .map(|spec| spec.name)
        .collect()
}
