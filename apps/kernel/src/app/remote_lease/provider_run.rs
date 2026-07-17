use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::provider::LaunchProviderRequest;
use crate::transport::relay_peer::RequiredRemoteMcp;

use super::mcp_availability::provider_run_mcp_set_matches;
use super::RemoteLeaseRuntime;

pub(crate) enum LeasedProviderRunMatch {
    Ready(String),
    LaunchRequired(LaunchProviderRequest),
}

impl<'a> RemoteLeaseRuntime<'a> {
    pub(super) fn ensure_home_proxy_manifest_has_no_worker_collisions(
        &self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
        remote_extension_manifest: &crate::extension::RemoteExtensionManifest,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        remote_extension_manifest.validate_unique_tool_names(operation)?;
        if remote_extension_manifest.is_empty() {
            return Ok(());
        }

        let session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let backing_agent = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
        let mut worker_tool_names = std::collections::BTreeMap::<String, String>::new();
        for spec in crate::transport::runtime_tools::workspace_live_sync_runtime_tool_specs()
            .into_iter()
            .chain(crate::transport::runtime_tools::extension_runtime_tool_specs())
            .chain(crate::transport::runtime_tools::recall_runtime_tool_specs())
            .chain(crate::transport::runtime_tools::credential_runtime_tool_specs())
            .chain(crate::transport::runtime_tools::workflow_runtime_tool_specs())
        {
            worker_tool_names.insert(spec.name, "worker runtime tool".to_string());
        }

        for name in remote_extension_manifest.home_proxy_mcp_server_names() {
            if required_mcps
                .iter()
                .any(|required| required.config.name == name)
            {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: format!(
                        "home-proxy MCP `{name}` collides with a worker-local MCP; rename one before launching the remote agent"
                    ),
                });
            }
        }

        let script_roots = crate::runtime::capability_registry::script_registry_roots(Some(
            session.workspace_id(),
        ))?;
        let script_registry = crate::script::ArrobaScriptRegistry::new(script_roots);
        for grant in backing_agent.worker_script_grants() {
            if let Some(script) = script_registry.get(&grant.name)? {
                worker_tool_names
                    .insert(script.name, format!("worker-local script `{}`", grant.name));
            }
        }

        let connector_registry = crate::connector::ArrobaConnectorRegistry::user()?;
        for grant in backing_agent.worker_connector_grants() {
            let Some(connector) = connector_registry.get(&grant.name)? else {
                continue;
            };
            let max_safety = crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())?;
            for operation_config in connector.operations {
                if operation_config.safety > max_safety {
                    continue;
                }
                worker_tool_names.insert(
                    crate::connector::connector_tool_name(&connector.name, &operation_config.name),
                    format!("worker-local connector `{}`", connector.name),
                );
            }
        }

        for tool in &remote_extension_manifest.tools {
            if tool.execution_location != crate::extension::ExtensionExecutionLocation::Home {
                continue;
            }
            if let Some(worker_source) = worker_tool_names.get(&tool.tool_name) {
                return Err(DaemonError::LocalTransport {
                    operation,
                    message: format!(
                        "home-proxy extension tool `{}` (`{}:{}`) collides with {worker_source}; rename one before launching the remote agent",
                        tool.tool_name,
                        tool.kind.as_str(),
                        tool.name
                    ),
                });
            }
        }

        Ok(())
    }

    pub(crate) fn prepare_leased_provider_run_matches_mcps(
        &mut self,
        leased_agent: &LeasedAgent,
        required_mcps: &[RequiredRemoteMcp],
        remote_extension_manifest: &crate::extension::RemoteExtensionManifest,
    ) -> Result<LeasedProviderRunMatch, DaemonError> {
        let effective_required_mcps =
            self.worker_effective_required_mcps(leased_agent, required_mcps)?;
        self.ensure_home_proxy_manifest_has_no_worker_collisions(
            leased_agent,
            &effective_required_mcps,
            remote_extension_manifest,
            "remote provider launch",
        )?;
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        let existing = self.app.providers.get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        );
        if let Some(run) = existing.as_ref() {
            if provider_run_mcp_set_matches(run, &effective_required_mcps)? {
                if !remote_extension_manifest.is_empty() {
                    let updated = self.app.providers.update_run_remote_extension_manifest(
                        run.id(),
                        remote_extension_manifest.clone(),
                    )?;
                    self.app.update_provider_run_projection(updated);
                }
                return Ok(LeasedProviderRunMatch::Ready(run.id().to_string()));
            }
            if self
                .app
                .prompt_owner_active_prompt_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )?
                .is_some()
            {
                return Err(DaemonError::LocalTransport {
                    operation: "remote MCP provider reload",
                    message: format!(
                        "remote worker provider run `{}` does not have the required MCP set and is currently busy; retry after the active turn completes",
                        run.id()
                    ),
                });
            }
            let run_id = run.id().to_string();
            let _ = crate::app::provider_runtime::ProviderProcessTracker::new(self.app)
                .remove_run(&run_id);
            if let Ok(outcome) = self
                .app
                .providers
                .terminate_run_provider_only(run.session_id(), run.id())
            {
                let _ = self
                    .app
                    .sessions
                    .set_active_provider_run(outcome.run().session_id(), None);
                self.app.update_provider_run_projection(outcome.into_run());
            }
        }

        let mut request = LaunchProviderRequest::new(
            &leased_agent.backing_session_id,
            &leased_agent.provider,
            &leased_agent.provider,
            "default",
            leased_agent
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        )
        .with_agent_id(&leased_agent.backing_agent_id)
        .with_owner_user_id(lease.owner_user_id)
        .with_working_directory(std::path::PathBuf::from(
            self.app
                .sessions
                .get_session(&leased_agent.backing_session_id)?
                .worktree_id(),
        ))
        .with_mcp_servers(
            effective_required_mcps
                .iter()
                .map(|required| required.config.clone())
                .collect(),
        );
        let mut mcp_servers = request.mcp_servers.clone();
        for name in remote_extension_manifest.home_proxy_mcp_server_names() {
            if !mcp_servers.iter().any(|server| server.name == name) {
                mcp_servers.push(crate::mcp::ArrobaMcpServerConfig::streamable_http(
                    name,
                    "http://127.0.0.1/mcp",
                ));
            }
        }
        request = request
            .with_mcp_servers(mcp_servers)
            .with_remote_extension_manifest(remote_extension_manifest.clone());
        if let Some(execution_mode) = leased_agent.execution_mode {
            request = request.with_execution_mode(execution_mode);
        }
        if let Some(permission_level) = leased_agent.permission_level {
            request = request.with_permission_level(permission_level);
        }
        if leased_agent.effort.is_some() {
            request = request.with_variant(leased_agent.effort.clone());
        }
        if let Some(run) = existing.as_ref() {
            request = request.with_resume_state(run.resume_state().clone());
            if request.variant.is_none() {
                request = request.with_variant(run.variant().map(str::to_string));
            }
        }
        let session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        request = request.with_workspace_live_sync_mode(
            crate::provider::provider_workspace_live_sync_mode_for_session(
                &leased_agent.provider,
                self.app.config(),
                Some(&session),
            ),
        );
        Ok(LeasedProviderRunMatch::LaunchRequired(request))
    }

    fn worker_effective_required_mcps(
        &self,
        leased_agent: &LeasedAgent,
        home_required_mcps: &[RequiredRemoteMcp],
    ) -> Result<Vec<RequiredRemoteMcp>, DaemonError> {
        let backing_agent = self.app.agents.get_agent(&leased_agent.backing_agent_id)?;
        let backing_session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let registry = crate::mcp::ArrobaMcpRegistry::new(
            crate::runtime::capability_registry::mcp_registry_roots(Some(
                backing_session.workspace_id(),
            ))?,
        );
        let mut required = home_required_mcps.to_vec();
        for name in backing_agent.worker_mcp_grants() {
            let config = registry
                .get(&name)?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "worker MCP provider launch",
                    message: format!("worker-local MCP `{name}` is granted but not installed"),
                })?;
            if !config.enabled {
                return Err(DaemonError::LocalTransport {
                    operation: "worker MCP provider launch",
                    message: format!("worker-local MCP `{name}` is disabled"),
                });
            }
            if !required
                .iter()
                .any(|existing| existing.config.name == config.name)
            {
                required.push(RequiredRemoteMcp {
                    definition_hash: config.definition_hash()?,
                    config,
                });
            }
        }
        Ok(required)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_but_ungranted_worker_mcp_does_not_block_home_proxy_mcp() {
        let _guard = crate::env_lock::lock();
        let isolation_root = std::env::temp_dir().join(format!(
            "arroba-home-proxy-installed-worker-mcp-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::env::set_var("ARROBA_CAPABILITY_ISOLATION_ROOT", &isolation_root);
        let name = "installed-but-ungranted-worker-mcp";
        let registry =
            crate::mcp::ArrobaMcpRegistry::new(vec![crate::mcp::ArrobaMcpRegistry::user_root()
                .expect("isolated MCP root should resolve")]);
        registry
            .install(&crate::mcp::ArrobaMcpServerConfig::stdio(
                name,
                "true",
                Vec::new(),
            ))
            .expect("worker MCP should install");

        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = crate::app::DaemonApp::bootstrap(config).expect("daemon should boot");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .expect("execution lease should create");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should create");
        let manifest = crate::extension::RemoteExtensionManifest {
            tools: vec![crate::extension::RemoteExtensionTool {
                kind: crate::extension::ExtensionKind::Mcp,
                name: name.to_string(),
                tool_name: name.to_string(),
                description: "home MCP proxy".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                authority: crate::extension::ExtensionAuthority::Home,
                definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
                execution_location: crate::extension::ExtensionExecutionLocation::Home,
                safety: None,
                timeout_sec: None,
                version_hash: None,
            }],
        };

        RemoteLeaseRuntime::new(&mut app)
            .ensure_home_proxy_manifest_has_no_worker_collisions(
                &leased_agent,
                &[],
                &manifest,
                "test home proxy collision",
            )
            .expect("an installed but ungranted worker MCP must not claim execution authority");

        std::env::remove_var("ARROBA_CAPABILITY_ISOLATION_ROOT");
        let _ = std::fs::remove_dir_all(isolation_root);
    }
}
