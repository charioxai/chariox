use super::*;

impl KernelRuntimeState {
    pub(crate) async fn grant_agent_extension(
        &self,
        agent_ref: &str,
        grant: crate::extension::ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if grant.source == crate::extension::ExtensionSource::Worker {
            return self
                .grant_agent_worker_extension(agent_ref, grant, caller_user_id)
                .await;
        }
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
                self.append_home_extension_grant_audit_event(
                    "home_extension.grant.created",
                    &agent,
                    caller_user_id,
                    &grant,
                )?;
                self.sync_remote_extension_manifest_for_agent(
                    &agent,
                    Some(caller_user_id),
                    Some(false),
                )
                .await?;
                self.owned.agent_store.get_agent(agent.id())
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
                self.append_home_extension_grant_audit_event(
                    "home_extension.grant.created",
                    &agent,
                    caller_user_id,
                    &grant,
                )?;
                self.sync_remote_extension_manifest_for_agent(
                    &agent,
                    Some(caller_user_id),
                    Some(false),
                )
                .await?;
                self.owned.agent_store.get_agent(agent.id())
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
        self.owned.ensure_agent_extension_authority(
            agent.id(),
            caller_user_id,
            "grant agent extension",
        )?;
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let mut reserved = static_runtime_tool_names();
        let script_registry = crate::script::ArrobaScriptRegistry::new(
            crate::runtime::capability_registry::script_registry_roots(Some(
                session.workspace_id(),
            ))?,
        );
        let connector_registry = crate::connector::ArrobaConnectorRegistry::user()?;
        for grant in agent.extension_grants() {
            if grant.source != proposed.source
                && grant.kind == proposed.kind
                && grant.name == proposed.name
            {
                return Err(DaemonError::LocalTransport {
                    operation: "agent.extension.grant",
                    message: format!(
                        "extension `{}:{}` is already granted from {:?} and would collide",
                        grant.kind.as_str(),
                        grant.name,
                        grant.source
                    ),
                });
            }
            if grant.source != crate::extension::ExtensionSource::Home {
                continue;
            }
            if grant.source == proposed.source
                && grant.kind == proposed.kind
                && grant.name == proposed.name
            {
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
        source: crate::extension::ExtensionSource,
        kind: crate::extension::ExtensionKind,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if source == crate::extension::ExtensionSource::Worker {
            return self
                .revoke_agent_worker_extension(agent_ref, kind, name, caller_user_id)
                .await;
        }
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
                    crate::extension::ExtensionSource::Home,
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
                self.append_home_extension_named_grant_audit_event(
                    "home_extension.grant.revoked",
                    &agent,
                    caller_user_id,
                    crate::extension::ExtensionKind::Script,
                    name,
                )?;
                self.sync_remote_extension_manifest_for_agent(
                    &agent,
                    Some(caller_user_id),
                    Some(true),
                )
                .await?;
                self.owned.agent_store.get_agent(agent.id())
            }
            crate::extension::ExtensionKind::Connector => {
                let agent = self.owned.revoke_agent_extension(
                    agent_ref,
                    crate::extension::ExtensionSource::Home,
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
                self.append_home_extension_named_grant_audit_event(
                    "home_extension.grant.revoked",
                    &agent,
                    caller_user_id,
                    crate::extension::ExtensionKind::Connector,
                    name,
                )?;
                self.sync_remote_extension_manifest_for_agent(
                    &agent,
                    Some(caller_user_id),
                    Some(true),
                )
                .await?;
                self.owned.agent_store.get_agent(agent.id())
            }
        }
    }

    async fn grant_agent_worker_extension(
        &self,
        agent_ref: &str,
        grant: crate::extension::ExtensionGrant,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let existing = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        self.owned.ensure_agent_extension_authority(
            existing.id(),
            caller_user_id,
            "grant worker extension",
        )?;
        if existing.remote_execution().is_none() {
            return Err(DaemonError::LocalTransport {
                operation: "agent.extension.grant",
                message: "worker extensions require an agent assigned to a worker kernel"
                    .to_string(),
            });
        }
        if existing.has_extension_grant_from(
            crate::extension::ExtensionSource::Home,
            grant.kind.clone(),
            &grant.name,
        ) {
            return Err(DaemonError::LocalTransport {
                operation: "agent.extension.grant",
                message: format!(
                    "extension `{}:{}` is already granted from home and would collide with the worker grant",
                    grant.kind.as_str(),
                    grant.name
                ),
            });
        }
        let agent = self
            .owned
            .grant_agent_extension(agent_ref, grant.clone(), caller_user_id)?;
        self.append_agent_durable_event(
            "agent.extension_granted",
            &agent,
            Some(&format!("worker:{}:{}", grant.kind.as_str(), grant.name)),
        )
        .await?;
        self.append_home_extension_grant_audit_event(
            "worker_extension.grant.created",
            &agent,
            caller_user_id,
            &grant,
        )?;
        self.sync_worker_extension_grants_for_agent(&agent, Some(false))
            .await?;
        self.owned.agent_store.get_agent(agent.id())
    }

    async fn revoke_agent_worker_extension(
        &self,
        agent_ref: &str,
        kind: crate::extension::ExtensionKind,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.owned.revoke_agent_extension(
            agent_ref,
            crate::extension::ExtensionSource::Worker,
            kind.clone(),
            name,
            caller_user_id,
        )?;
        self.append_agent_durable_event(
            "agent.extension_revoked",
            &agent,
            Some(&format!("worker:{}:{name}", kind.as_str())),
        )
        .await?;
        let grant = crate::extension::ExtensionGrant::new(kind, name)
            .from_source(crate::extension::ExtensionSource::Worker);
        self.append_home_extension_grant_audit_event(
            "worker_extension.grant.revoked",
            &agent,
            caller_user_id,
            &grant,
        )?;
        self.sync_worker_extension_grants_for_agent(&agent, Some(true))
            .await?;
        self.owned.agent_store.get_agent(agent.id())
    }

    pub(crate) async fn sync_worker_extension_grants_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
        pending_revoke_intent: Option<bool>,
    ) -> Result<(), DaemonError> {
        let Some(remote) = agent.remote_execution().cloned() else {
            return Ok(());
        };
        let mut grants = agent.extension_grants_from(crate::extension::ExtensionSource::Worker);
        grants.sort();
        let manifest_hash = crate::extension::extension_grant_manifest_hash(&grants)?;
        let pending_revoke = pending_revoke_intent.unwrap_or(false)
            || agent
                .worker_extension_grant_sync()
                .is_some_and(|status| status.pending_revoke == Some(true));
        let syncing = crate::extension::RemoteExtensionManifestSyncStatus::pending(
            manifest_hash.clone(),
            pending_revoke,
        )
        .syncing();
        if self
            .owned
            .agent_store
            .begin_extension_sync_attempt(
                agent.id(),
                crate::extension::ExtensionSource::Worker,
                &remote,
                &manifest_hash,
                syncing.clone(),
            )?
            .is_none()
        {
            return Ok(());
        }

        let mut config = self.config_snapshot().await;
        if let (Some(relay_url), Some(relay_token)) =
            (remote.relay_url.clone(), remote.relay_token.clone())
        {
            config.apply_missing_remote_relay_override(relay_url, relay_token);
        }
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
            &config,
            ClientTarget {
                daemon_id: Some(remote.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::UpdateLeasedAgentWorkerExtensionGrants {
                leased_agent_id: remote.leased_agent_id.clone(),
                grants: grants.clone(),
            },
        )
        .await;
        let completed_status = match response {
            Ok(RelayPeerResponse::LeasedAgentWorkerExtensionGrantsUpdated {
                leased_agent_id,
                manifest_hash: applied_hash,
                grants: applied_grants,
            }) if {
                leased_agent_id == remote.leased_agent_id && applied_hash == manifest_hash && {
                    let mut canonical_applied_grants = applied_grants.clone();
                    canonical_applied_grants.sort();
                    canonical_applied_grants == grants
                }
            } =>
            {
                crate::extension::RemoteExtensionManifestSyncStatus::synced(manifest_hash.clone())
            }
            Ok(other) => syncing.clone().failed(format!(
                "unexpected worker extension sync response: {other:?}"
            )),
            Err(error) => syncing.clone().failed(error.to_string()),
        };
        if let Some(applied_agent) = self.owned.agent_store.finish_extension_sync_attempt(
            agent.id(),
            crate::extension::ExtensionSource::Worker,
            &remote,
            &manifest_hash,
            &syncing,
            completed_status,
        )? {
            self.publish_extension_sync_agent(&applied_agent)?;
        }
        Ok(())
    }

    pub(crate) async fn reconcile_worker_extension_grants_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if agent
            .extension_grants_from(crate::extension::ExtensionSource::Worker)
            .is_empty()
            && agent.worker_extension_grant_sync().is_none()
        {
            return Ok(agent.clone());
        }
        let current_manifest_hash = crate::extension::extension_grant_manifest_hash(
            &agent.extension_grants_from(crate::extension::ExtensionSource::Worker),
        )?;
        if !worker_extension_grants_are_synced(agent, &current_manifest_hash) {
            self.sync_worker_extension_grants_for_agent(agent, None)
                .await?;
        }
        let refreshed = self.owned.agent_store.get_agent(agent.id())?;
        let refreshed_manifest_hash = crate::extension::extension_grant_manifest_hash(
            &refreshed.extension_grants_from(crate::extension::ExtensionSource::Worker),
        )?;
        if !worker_extension_grants_are_synced(&refreshed, &refreshed_manifest_hash) {
            return Err(DaemonError::LocalTransport {
                operation: "worker extension grant reconciliation",
                message: refreshed
                    .worker_extension_grant_sync()
                    .and_then(|status| status.last_error.clone())
                    .unwrap_or_else(|| {
                        "worker extension grants are not synchronized with the current manifest"
                            .to_string()
                    }),
            });
        }
        Ok(refreshed)
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
        self.owned.ensure_agent_extension_authority(
            existing.id(),
            caller_user_id,
            "grant agent capability",
        )?;
        if existing.has_extension_grant_from(
            crate::extension::ExtensionSource::Worker,
            crate::extension::ExtensionKind::Mcp,
            &name,
        ) {
            return Err(DaemonError::LocalTransport {
                operation: "agent.extension.grant",
                message: format!(
                    "MCP `{name}` is already granted from worker and would collide with the home grant"
                ),
            });
        }
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
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.created",
            &agent,
            caller_user_id,
            crate::extension::ExtensionKind::Mcp,
            &name,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(false))
            .await?;
        let _ = self
            .apply_provider_reload_policy(ProviderReloadTrigger::AgentMcpChanged {
                session_id: agent.session_id().to_string(),
                agent_id: agent.id().to_string(),
                name,
            })
            .await?;
        self.owned.agent_store.get_agent(agent.id())
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
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.revoked",
            &agent,
            caller_user_id,
            crate::extension::ExtensionKind::Mcp,
            name,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(true))
            .await?;
        let _ = self
            .apply_provider_reload_policy(ProviderReloadTrigger::AgentMcpChanged {
                session_id: agent.session_id().to_string(),
                agent_id: agent.id().to_string(),
                name: name.to_string(),
            })
            .await?;
        self.owned.agent_store.get_agent(agent.id())
    }

    pub(crate) async fn grant_agent_skill(
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
        if existing.has_extension_grant_from(
            crate::extension::ExtensionSource::Worker,
            crate::extension::ExtensionKind::Skill,
            &name,
        ) {
            return Err(DaemonError::LocalTransport {
                operation: "agent.extension.grant",
                message: format!(
                    "skill `{name}` is already granted from worker and would collide with the home grant"
                ),
            });
        }
        let agent = self
            .owned
            .grant_agent_skill(agent_ref, name.clone(), caller_user_id)?;
        self.append_agent_durable_event("agent.skill_granted", &agent, Some(&name))
            .await?;
        self.ensure_remote_skill_packages_for_agent(&agent).await?;
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.created",
            &agent,
            caller_user_id,
            crate::extension::ExtensionKind::Skill,
            &name,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(false))
            .await?;
        self.owned.agent_store.get_agent(agent.id())
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
        self.append_home_extension_named_grant_audit_event(
            "home_extension.grant.revoked",
            &agent,
            caller_user_id,
            crate::extension::ExtensionKind::Skill,
            name,
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), Some(false))
            .await?;
        self.owned.agent_store.get_agent(agent.id())
    }

    async fn sync_remote_extension_manifest_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_user_id: Option<&str>,
        pending_revoke_intent: Option<bool>,
    ) -> Result<(), DaemonError> {
        self.sync_remote_extension_manifest_for_agent_inner::<true>(
            agent,
            caller_user_id,
            pending_revoke_intent,
        )
        .await
    }

    async fn sync_remote_extension_manifest_for_agent_inner<const SCHEDULE_RETRIES: bool>(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_user_id: Option<&str>,
        pending_revoke_intent: Option<bool>,
    ) -> Result<(), DaemonError> {
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(());
        };
        let manifest = self.remote_extension_manifest_for_agent(agent)?;
        let manifest_hash = manifest.manifest_hash();
        let grant_manifest_hash = crate::extension::extension_grant_manifest_hash(
            &agent.extension_grants_from(crate::extension::ExtensionSource::Home),
        )?;
        let tool_count = manifest.tools.len();
        let pending_revoke = remote_extension_manifest_pending_revoke(
            agent.remote_extension_manifest_sync(),
            pending_revoke_intent,
        );
        let syncing_status = crate::extension::RemoteExtensionManifestSyncStatus::pending(
            manifest_hash.clone(),
            pending_revoke,
        )
        .syncing();
        if self
            .owned
            .agent_store
            .begin_extension_sync_attempt(
                agent.id(),
                crate::extension::ExtensionSource::Home,
                &remote_execution,
                &grant_manifest_hash,
                syncing_status.clone(),
            )?
            .is_none()
        {
            return Ok(());
        }
        let mut config = self.config_snapshot().await;
        if let (Some(relay_url), Some(relay_token)) = (
            remote_execution.relay_url.clone(),
            remote_execution.relay_token.clone(),
        ) {
            config.apply_missing_remote_relay_override(relay_url, relay_token);
        }
        let target = ClientTarget {
            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let request = RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            remote_extension_manifest: manifest,
        };
        let response = match self.connected_relay_state_for_config(&config).await {
            Some(relay_state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay(
                    &config,
                    &relay_state,
                    target,
                    request,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &config, target, request,
                )
                .await
            }
        };
        if !self
            .home_extension_manifest_attempt_is_current::<SCHEDULE_RETRIES>(
                agent.id(),
                &remote_execution,
                &syncing_status,
                &manifest_hash,
            )
            .await?
        {
            return Ok(());
        }
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                let error_message = error.to_string();
                let applied_agent = self.owned.agent_store.finish_extension_sync_attempt(
                    agent.id(),
                    crate::extension::ExtensionSource::Home,
                    &remote_execution,
                    &grant_manifest_hash,
                    &syncing_status,
                    syncing_status.clone().failed(error_message.clone()),
                )?;
                let Some(applied_agent) = applied_agent else {
                    return Ok(());
                };
                let _ = self.append_home_extension_manifest_audit_event(
                    "home_extension.manifest.failed",
                    &applied_agent,
                    caller_user_id,
                    &manifest_hash,
                    tool_count,
                    pending_revoke,
                    Some("failed"),
                    Some(&error_message),
                    None,
                    None,
                );
                self.publish_extension_sync_agent(&applied_agent)?;
                if SCHEDULE_RETRIES {
                    self.schedule_remote_extension_manifest_retry(
                        &applied_agent,
                        caller_user_id,
                        manifest_hash.clone(),
                        tool_count,
                        pending_revoke,
                        error_message.clone(),
                    )
                    .await;
                }
                crate::logging::warn_with_fields(
                    "daemon.remote_extension",
                    "remote extension manifest sync failed; home validation remains authoritative",
                    serde_json::json!({
                        "agent_id": agent.id(),
                        "worker_kernel_id": remote_execution.worker_kernel_id,
                        "error": error_message,
                    }),
                );
                return Ok(());
            }
        };
        if !matches!(
            &response,
            RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated { leased_agent_id }
                if leased_agent_id == &remote_execution.leased_agent_id
        ) {
            let error = "unexpected worker manifest sync response".to_string();
            let applied_agent = self.owned.agent_store.finish_extension_sync_attempt(
                agent.id(),
                crate::extension::ExtensionSource::Home,
                &remote_execution,
                &grant_manifest_hash,
                &syncing_status,
                syncing_status.clone().failed(error.clone()),
            )?;
            let Some(applied_agent) = applied_agent else {
                return Ok(());
            };
            let _ = self.append_home_extension_manifest_audit_event(
                "home_extension.manifest.failed",
                &applied_agent,
                caller_user_id,
                &manifest_hash,
                tool_count,
                pending_revoke,
                Some("failed"),
                Some(&error),
                None,
                None,
            );
            self.publish_extension_sync_agent(&applied_agent)?;
            if SCHEDULE_RETRIES {
                self.schedule_remote_extension_manifest_retry(
                    &applied_agent,
                    caller_user_id,
                    manifest_hash.clone(),
                    tool_count,
                    pending_revoke,
                    error,
                )
                .await;
            }
            crate::logging::warn_with_fields(
                "daemon.remote_extension",
                "remote extension manifest sync returned an unexpected response",
                serde_json::json!({
                    "agent_id": agent.id(),
                    "worker_kernel_id": remote_execution.worker_kernel_id,
                    "response": format!("{response:?}"),
                }),
            );
        } else {
            let applied_agent = self.owned.agent_store.finish_extension_sync_attempt(
                agent.id(),
                crate::extension::ExtensionSource::Home,
                &remote_execution,
                &grant_manifest_hash,
                &syncing_status,
                crate::extension::RemoteExtensionManifestSyncStatus::synced(manifest_hash.clone()),
            )?;
            let Some(applied_agent) = applied_agent else {
                return Ok(());
            };
            self.owned
                .remote_extension_manifest_retry_counts
                .lock()
                .await
                .remove(&remote_extension_manifest_retry_key(
                    agent.id(),
                    &manifest_hash,
                ));
            self.append_home_extension_manifest_audit_event(
                "home_extension.manifest.synced",
                &applied_agent,
                caller_user_id,
                &manifest_hash,
                tool_count,
                pending_revoke,
                Some("synced"),
                None,
                None,
                None,
            )?;
            self.publish_extension_sync_agent(&applied_agent)?;
        }
        Ok(())
    }

    fn publish_extension_sync_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let _ = self.owned.session_snapshot(agent.session_id())?;
        Ok(())
    }

    async fn home_extension_manifest_attempt_is_current<const SCHEDULE_RETRIES: bool>(
        &self,
        agent_id: &str,
        expected_binding: &crate::agent::RemoteAgentBinding,
        expected_syncing_status: &crate::extension::RemoteExtensionManifestSyncStatus,
        expected_manifest_hash: &str,
    ) -> Result<bool, DaemonError> {
        let current = self.owned.agent_store.get_agent(agent_id)?;
        if current.remote_execution() != Some(expected_binding)
            || current.remote_extension_manifest_sync() != Some(expected_syncing_status)
        {
            return Ok(false);
        }
        let current_manifest_hash = self
            .remote_extension_manifest_for_agent(&current)?
            .manifest_hash();
        if current_manifest_hash == expected_manifest_hash {
            return Ok(true);
        }

        // Registry definitions are part of the manifest authority even when the
        // grant identities are unchanged. Replace the obsolete in-flight attempt
        // before its response can publish a stale Synced state.
        Box::pin(
            self.sync_remote_extension_manifest_for_agent_inner::<SCHEDULE_RETRIES>(
                &current, None, None,
            ),
        )
        .await?;
        Ok(false)
    }

    async fn schedule_remote_extension_manifest_retry(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_user_id: Option<&str>,
        manifest_hash: String,
        tool_count: usize,
        pending_revoke: bool,
        error: String,
    ) {
        const RETRY_DELAYS_SECONDS: [u64; 3] = [2, 10, 30];
        let agent_id = agent.id().to_string();
        let retry_key = remote_extension_manifest_retry_key(&agent_id, &manifest_hash);
        let attempt = {
            let mut counts = self
                .owned
                .remote_extension_manifest_retry_counts
                .lock()
                .await;
            let count = counts.entry(retry_key.clone()).or_insert(0);
            if *count >= RETRY_DELAYS_SECONDS.len() as u32 {
                return;
            }
            *count += 1;
            *count
        };
        let delay = RETRY_DELAYS_SECONDS[(attempt - 1) as usize];
        let mut payload = self.home_extension_agent_audit_payload(agent, caller_user_id);
        payload.insert(
            "manifest_hash".to_string(),
            serde_json::json!(manifest_hash),
        );
        payload.insert("tool_count".to_string(), serde_json::json!(tool_count));
        payload.insert(
            "pending_revoke".to_string(),
            serde_json::json!(pending_revoke),
        );
        payload.insert("attempt".to_string(), serde_json::json!(attempt));
        payload.insert("delay_sec".to_string(), serde_json::json!(delay));
        payload.insert("error".to_string(), serde_json::json!(error));
        payload.insert("status".to_string(), serde_json::json!("retry_scheduled"));
        let _ = self.owned.durable_state_store.append_event(
            "home_extension.manifest.retry_scheduled",
            Some(agent_id.clone()),
            serde_json::Value::Object(payload),
        );
        let state = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(delay));
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                let Ok(agent) = state.owned.agent_store.get_agent(&agent_id) else {
                    return;
                };
                let status_matches = agent
                    .remote_extension_manifest_sync()
                    .and_then(|status| status.manifest_hash.as_deref())
                    == Some(manifest_hash.as_str());
                if !status_matches {
                    return;
                }
                let _ = state
                    .sync_remote_extension_manifest_for_agent_inner::<false>(&agent, None, None)
                    .await;
            });
        });
    }

    pub(crate) async fn retry_remote_extension_manifest_sync(
        &self,
        agent_ref: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        self.owned.ensure_agent_extension_authority(
            agent.id(),
            caller_user_id,
            "remote extension manifest sync retry",
        )?;
        self.sync_remote_extension_manifest_for_agent(&agent, Some(caller_user_id), None)
            .await?;
        if !agent
            .extension_grants_from(crate::extension::ExtensionSource::Worker)
            .is_empty()
            || agent.worker_extension_grant_sync().is_some()
        {
            self.sync_worker_extension_grants_for_agent(&agent, None)
                .await?;
        }
        self.owned.agent_store.get_agent(agent.id())
    }

    fn append_home_extension_grant_audit_event(
        &self,
        kind: &'static str,
        agent: &crate::agent::AgentInstance,
        caller_user_id: &str,
        grant: &crate::extension::ExtensionGrant,
    ) -> Result<(), DaemonError> {
        let mut payload = self.home_extension_agent_audit_payload(agent, Some(caller_user_id));
        payload.insert(
            "grant".to_string(),
            serde_json::json!({
                "source": grant.source,
                "kind": grant.kind.as_str(),
                "name": grant.name,
                "environment": grant.environment,
                "credential_present": grant.credential.is_some(),
                "max_safety": grant.max_safety,
            }),
        );
        self.owned.durable_state_store.append_event(
            kind,
            Some(agent.id().to_string()),
            serde_json::Value::Object(payload),
        )?;
        Ok(())
    }

    fn append_home_extension_manifest_audit_event(
        &self,
        kind: &'static str,
        agent: &crate::agent::AgentInstance,
        caller_user_id: Option<&str>,
        manifest_hash: &str,
        tool_count: usize,
        pending_revoke: bool,
        status: Option<&str>,
        error: Option<&str>,
        attempt: Option<u32>,
        delay_sec: Option<u64>,
    ) -> Result<(), DaemonError> {
        let mut payload = self.home_extension_agent_audit_payload(agent, caller_user_id);
        payload.insert(
            "manifest_hash".to_string(),
            serde_json::json!(manifest_hash),
        );
        payload.insert("tool_count".to_string(), serde_json::json!(tool_count));
        payload.insert(
            "pending_revoke".to_string(),
            serde_json::json!(pending_revoke),
        );
        if pending_revoke && kind == "home_extension.manifest.synced" {
            payload.insert("revoke_acknowledged".to_string(), serde_json::json!(true));
        }
        if let Some(status) = status {
            payload.insert("status".to_string(), serde_json::json!(status));
        }
        if let Some(error) = error {
            payload.insert("error".to_string(), serde_json::json!(error));
        }
        if let Some(attempt) = attempt {
            payload.insert("attempt".to_string(), serde_json::json!(attempt));
        }
        if let Some(delay_sec) = delay_sec {
            payload.insert("delay_sec".to_string(), serde_json::json!(delay_sec));
        }
        self.owned.durable_state_store.append_event(
            kind,
            Some(agent.id().to_string()),
            serde_json::Value::Object(payload),
        )?;
        Ok(())
    }

    fn append_home_extension_named_grant_audit_event(
        &self,
        kind: &'static str,
        agent: &crate::agent::AgentInstance,
        caller_user_id: &str,
        extension_kind: crate::extension::ExtensionKind,
        name: &str,
    ) -> Result<(), DaemonError> {
        let mut payload = self.home_extension_agent_audit_payload(agent, Some(caller_user_id));
        payload.insert(
            "grant".to_string(),
            serde_json::json!({
                "source": crate::extension::ExtensionSource::Home,
                "kind": extension_kind.as_str(),
                "name": name,
            }),
        );
        self.owned.durable_state_store.append_event(
            kind,
            Some(agent.id().to_string()),
            serde_json::Value::Object(payload),
        )?;
        Ok(())
    }

    pub(in crate::runtime::state) fn home_extension_agent_audit_payload(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_user_id: Option<&str>,
    ) -> serde_json::Map<String, serde_json::Value> {
        let session = self
            .owned
            .session_store
            .get_session(agent.session_id())
            .ok();
        let remote_execution = agent.remote_execution();
        let mut payload = serde_json::Map::new();
        payload.insert(
            "home_session_id".to_string(),
            serde_json::json!(agent.session_id()),
        );
        payload.insert(
            "home_user_id".to_string(),
            serde_json::json!(session.as_ref().map(|session| session.owner_user_id())),
        );
        payload.insert(
            "caller_user_id".to_string(),
            serde_json::json!(caller_user_id),
        );
        payload.insert("agent_id".to_string(), serde_json::json!(agent.id()));
        payload.insert(
            "agent_ref".to_string(),
            serde_json::json!(agent.agent_ref()),
        );
        payload.insert(
            "agent_owner_user_id".to_string(),
            serde_json::json!(agent.owner_user_id()),
        );
        payload.insert(
            "lease_id".to_string(),
            serde_json::json!(remote_execution.map(|remote| remote.execution_lease_id.as_str())),
        );
        payload.insert(
            "leased_agent_id".to_string(),
            serde_json::json!(remote_execution.map(|remote| remote.leased_agent_id.as_str())),
        );
        payload.insert(
            "worker_kernel_id".to_string(),
            serde_json::json!(remote_execution.map(|remote| remote.worker_kernel_id.as_str())),
        );
        payload.insert(
            "worker_machine_id".to_string(),
            serde_json::json!(remote_execution.map(|remote| remote.worker_machine_id.as_str())),
        );
        payload.insert(
            "active_worker_provider_run_id".to_string(),
            serde_json::json!(
                remote_execution.and_then(|remote| remote.active_worker_provider_run_id.as_deref())
            ),
        );
        payload
    }

    pub(crate) fn list_home_extension_audit_events(
        &self,
        agent_ref: &str,
        caller_user_id: &str,
        limit: usize,
    ) -> Result<Vec<crate::durable_state::DurableStateEvent>, DaemonError> {
        let agent = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        self.owned.ensure_agent_extension_authority(
            agent.id(),
            caller_user_id,
            "home extension audit",
        )?;
        let events = self
            .owned
            .durable_state_store
            .load_subject_events(agent.id(), limit)?;
        Ok(events
            .into_iter()
            .filter(|event| {
                event.kind.starts_with("home_extension.")
                    || event.kind.starts_with("agent.extension")
                    || event.kind.starts_with("agent.mcp_")
                    || event.kind.starts_with("agent.skill_")
                    || event.kind.starts_with("extension.registration.")
            })
            .collect())
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
                config.apply_remote_relay_override(relay_url, relay_token);
            }
            let target = ClientTarget {
                daemon_id: Some(remote_update.worker_kernel_id.clone()),
                daemon_alias: None,
            };
            let request = RelayPeerRequest::UpdateLeasedAgentConfig {
                leased_agent_id: remote_update.leased_agent_id,
                execution_mode: remote_update.execution_mode,
                permission_level: remote_update.permission_level,
            };
            let response = match self.connected_relay_state_for_config(&config).await {
                Some(relay_state) => {
                    crate::transport::relay_client::send_peer_request_via_connected_relay(
                        &config,
                        &relay_state,
                        target,
                        request,
                    )
                    .await
                }
                None => {
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &config, target, request,
                    )
                    .await
                }
            };
            match response {
                Ok(RelayPeerResponse::LeasedAgentConfigUpdated { .. }) => {}
                Ok(other) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "update remote leased agent config",
                        message: format!("unexpected remote config response: {other:?}"),
                    });
                }
                Err(error) => return Err(error),
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
        let update = self.owned.update_agent_profile(
            session_id,
            agent_id,
            caller_user_id,
            provider.clone(),
            model.clone(),
            effort.clone(),
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
                config.apply_remote_relay_override(relay_url, relay_token);
            }
            let target = ClientTarget {
                daemon_id: Some(remote_update.worker_kernel_id.clone()),
                daemon_alias: None,
            };
            let request = RelayPeerRequest::UpdateLeasedAgentProfile {
                leased_agent_id: remote_update.leased_agent_id,
                provider: remote_update.provider.clone(),
                model: remote_update.model.clone(),
                effort: remote_update.effort.clone(),
            };
            let response = match self.connected_relay_state_for_config(&config).await {
                Some(relay_state) => {
                    crate::transport::relay_client::send_peer_request_via_connected_relay(
                        &config,
                        &relay_state,
                        target,
                        request,
                    )
                    .await
                }
                None => {
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &config, target, request,
                    )
                    .await
                }
            };
            match response {
                Ok(RelayPeerResponse::LeasedAgentProfileUpdated { .. }) => {}
                Ok(other) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "update remote leased agent profile",
                        message: format!("unexpected remote profile response: {other:?}"),
                    });
                }
                Err(error) => return Err(error),
            }
            agent = self.owned.commit_remote_agent_profile_update(
                session_id,
                agent_id,
                remote_update.provider,
                remote_update.model,
                remote_update.effort,
            )?;
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

    pub(crate) async fn ensure_agent_prompt_access(
        &self,
        agent_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .ensure_agent_prompt_access(agent_id, caller_user_id, operation)
    }
}

fn remote_extension_manifest_retry_key(agent_id: &str, manifest_hash: &str) -> String {
    format!("{agent_id}:{manifest_hash}")
}

fn worker_extension_grants_are_synced(
    agent: &crate::agent::AgentInstance,
    current_manifest_hash: &str,
) -> bool {
    agent.worker_extension_grant_sync().is_some_and(|status| {
        status.state == crate::extension::RemoteExtensionManifestSyncState::Synced
            && status.manifest_hash.as_deref() == Some(current_manifest_hash)
    })
}

fn remote_extension_manifest_pending_revoke(
    current_status: Option<&crate::extension::RemoteExtensionManifestSyncStatus>,
    intent: Option<bool>,
) -> bool {
    match intent {
        Some(pending_revoke) => pending_revoke,
        None => current_status
            .and_then(|status| status.pending_revoke)
            .unwrap_or(false),
    }
}

fn static_runtime_tool_names() -> std::collections::BTreeSet<String> {
    crate::transport::runtime_tools::workspace_live_sync_runtime_tool_specs()
        .into_iter()
        .chain(crate::transport::runtime_tools::extension_runtime_tool_specs())
        .chain(crate::transport::runtime_tools::credential_runtime_tool_specs())
        .chain(crate::transport::runtime_tools::workflow_runtime_tool_specs())
        .chain(crate::transport::runtime_tools::slice_runtime_tool_specs())
        .map(|spec| spec.name)
        .collect()
}

#[cfg(test)]
mod tests;
