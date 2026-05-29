use std::path::PathBuf;

use base64::Engine;

use crate::error::DaemonError;
use crate::provider::{
    LaunchProviderRequest, ProviderClientInterface, ProviderResumeState, ProviderRunState,
    RuntimeProviderRun,
};
use crate::transport::relay_peer::RequiredRemoteMcp;

use super::RemoteLeaseRuntime;

impl<'a> RemoteLeaseRuntime<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn launch_leased_native_provider_run(
        &mut self,
        leased_agent_id: &str,
        adapter_key: &str,
        provider: &str,
        account_profile: &str,
        model: &str,
        variant: Option<String>,
        structured_endpoint: Option<String>,
        provider_session_id: Option<String>,
        required_mcps: Vec<RequiredRemoteMcp>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        self.ensure_required_remote_mcps_available(&leased_agent, &required_mcps)?;
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        let backing_session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let mut request = LaunchProviderRequest::new(
            leased_agent.backing_session_id.clone(),
            adapter_key,
            provider,
            account_profile,
            model,
        )
        .with_agent_id(leased_agent.backing_agent_id.clone())
        .with_owner_user_id(lease.owner_user_id)
        .with_working_directory(PathBuf::from(backing_session.worktree_id()))
        .with_client_interface(ProviderClientInterface::NativeTui)
        .with_mcp_servers(
            required_mcps
                .iter()
                .map(|required| required.config.clone())
                .collect(),
        )
        .with_remote_extension_manifest(remote_extension_manifest.clone())
        .with_variant(variant);
        let mut mcp_servers = request.mcp_servers.clone();
        for name in remote_extension_manifest.home_proxy_mcp_server_names() {
            if !mcp_servers.iter().any(|server| server.name == name) {
                mcp_servers.push(crate::mcp::ArrobaMcpServerConfig::streamable_http(
                    name,
                    "http://127.0.0.1/mcp",
                ));
            }
        }
        request = request.with_mcp_servers(mcp_servers);
        if let Some(execution_mode) = leased_agent.execution_mode {
            request = request.with_execution_mode(execution_mode);
        }
        if let Some(permission_level) = leased_agent.permission_level {
            request = request.with_permission_level(permission_level);
        }
        if let Some(endpoint) = structured_endpoint {
            request = request.with_structured_endpoint(endpoint);
        }
        if let Some(provider_session_id) = provider_session_id {
            request = match adapter_key {
                "codex" => request.with_resume_state(ProviderResumeState::from_codex_thread_id(
                    provider_session_id,
                )),
                "opencode" => request.with_resume_state(
                    ProviderResumeState::from_opencode_session_id(provider_session_id),
                ),
                "claude" => request.with_resume_state(ProviderResumeState::from_claude_session_id(
                    provider_session_id,
                )),
                _ => request,
            };
        }
        self.app.launch_provider(request)
    }

    pub(crate) fn send_leased_native_provider_input(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        _attachment_id: &str,
        data_base64: &str,
    ) -> Result<usize, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let mut provider_run = self.app.providers.get_run(provider_run_id)?;
        if provider_run.session_id() != leased_agent.backing_session_id
            || provider_run.agent_instance_id() != Some(leased_agent.backing_agent_id.as_str())
        {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: leased_agent.backing_session_id,
                provider_run_id: provider_run_id.to_string(),
            });
        }
        if provider_run.state() == ProviderRunState::Parked {
            let backing_session_id = provider_run.session_id().to_string();
            let outcome = self
                .app
                .providers
                .resume_run_provider_only(&backing_session_id, provider_run_id)?;
            self.app.sessions.set_active_provider_run(
                &backing_session_id,
                Some(outcome.run().id().to_string()),
            )?;
            let run = outcome.into_run();
            self.app.update_provider_run_projection(run.clone());
            provider_run = run;
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data_base64)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "send leased native provider input",
                message: format!("data_base64 is not valid base64: {error}"),
            })?;
        let byte_count = bytes.len();
        self.app.send_terminal_input(
            provider_run.session_id(),
            &leased_agent.backing_attachment_id,
            Some(provider_run_id),
            &bytes,
        )?;
        Ok(byte_count)
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{DaemonApp, RemoteLeaseRuntime};
    use crate::config::DaemonConfig;
    use crate::mcp::ArrobaMcpServerConfig;
    use crate::transport::relay_peer::RequiredRemoteMcp;

    #[test]
    fn leased_native_provider_launch_preserves_required_mcp_set() {
        let _guard = crate::env_lock::lock();
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let worktree = std::env::temp_dir().join(format!(
            "arroba-native-provider-mcp-test-{}",
            std::process::id()
        ));
        let isolation_root = std::env::temp_dir().join(format!(
            "arroba-native-provider-mcp-isolation-test-{}",
            std::process::id()
        ));
        std::env::set_var("ARROBA_CAPABILITY_ISOLATION_ROOT", &isolation_root);
        std::fs::create_dir_all(&worktree).expect("worktree should create");
        let mut runtime = RemoteLeaseRuntime::new(&mut app);
        let lease = runtime
            .create_execution_lease("home-kernel", "home-session", "home-agent", "local-user")
            .expect("lease should create");
        let leased_agent = runtime
            .create_leased_agent(
                &lease.id,
                "dev-stub",
                Some("default".to_string()),
                None,
                None,
                None,
                Some(worktree.display().to_string()),
                None,
            )
            .expect("leased agent should create");
        let mcp = ArrobaMcpServerConfig::stdio(
            "browser",
            std::env::current_exe()
                .expect("current test executable should resolve")
                .display()
                .to_string(),
            Vec::new(),
        );
        let required = RequiredRemoteMcp {
            definition_hash: mcp.definition_hash().expect("hash should compute"),
            config: mcp,
        };

        let run = runtime
            .launch_leased_native_provider_run(
                &leased_agent.id,
                "dev-stub",
                "dev-stub",
                "default",
                "default",
                None,
                None,
                None,
                vec![required],
                crate::extension::RemoteExtensionManifest::default(),
            )
            .expect("native run should launch");

        assert_eq!(run.mcp_servers().len(), 1);
        assert_eq!(run.mcp_servers()[0].name, "browser");
        assert!(!run.client_interface().is_arroba());
        assert!(crate::mcp::ArrobaMcpRegistry::project_root(&worktree)
            .join("browser.json")
            .exists());
        std::env::remove_var("ARROBA_CAPABILITY_ISOLATION_ROOT");
        let _ = std::fs::remove_dir_all(&worktree);
        let _ = std::fs::remove_dir_all(&isolation_root);
    }

    #[test]
    fn leased_native_provider_launch_projects_home_proxy_mcp_manifest() {
        let _guard = crate::env_lock::lock();
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let worktree = std::env::temp_dir().join(format!(
            "arroba-native-provider-home-proxy-mcp-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&worktree).expect("worktree should create");
        let mut runtime = RemoteLeaseRuntime::new(&mut app);
        let lease = runtime
            .create_execution_lease("home-kernel", "home-session", "home-agent", "local-user")
            .expect("lease should create");
        let leased_agent = runtime
            .create_leased_agent(
                &lease.id,
                "dev-stub",
                Some("default".to_string()),
                None,
                None,
                None,
                Some(worktree.display().to_string()),
                None,
            )
            .expect("leased agent should create");
        let manifest = crate::extension::RemoteExtensionManifest {
            tools: vec![crate::extension::RemoteExtensionTool {
                kind: crate::extension::ExtensionKind::Mcp,
                name: "home_browser".to_string(),
                tool_name: "home_browser".to_string(),
                description: "Home browser".to_string(),
                input_schema: serde_json::json!({}),
                authority: crate::extension::ExtensionAuthority::Home,
                definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
                execution_location: crate::extension::ExtensionExecutionLocation::Home,
                safety: None,
                timeout_sec: Some(30),
                version_hash: Some("hash".to_string()),
            }],
        };

        let run = runtime
            .launch_leased_native_provider_run(
                &leased_agent.id,
                "dev-stub",
                "dev-stub",
                "default",
                "default",
                None,
                None,
                None,
                Vec::new(),
                manifest.clone(),
            )
            .expect("native run should launch");

        assert_eq!(run.remote_extension_manifest(), &manifest);
        assert_eq!(run.mcp_servers().len(), 1);
        assert_eq!(run.mcp_servers()[0].name, "home_browser");
        let _ = std::fs::remove_dir_all(&worktree);
    }
}
