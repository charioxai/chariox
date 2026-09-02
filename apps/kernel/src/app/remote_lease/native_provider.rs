use std::path::PathBuf;

use base64::Engine;

use crate::error::DaemonError;
use crate::provider::{
    LaunchProviderRequest, ProviderClientInterface, ProviderResumeState, ProviderRunState,
    RuntimeProviderRun,
};
use crate::transport::relay_peer::RequiredRemoteMcp;

use super::RemoteLeaseRuntime;

fn leased_native_source_attachment_id(leased_agent_id: &str, attachment_id: &str) -> String {
    format!("remote-native:{leased_agent_id}:{attachment_id}")
}

impl<'a> RemoteLeaseRuntime<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_leased_native_provider_launch(
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
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<LaunchProviderRequest, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        if let Some(required_skills) = required_skills.as_deref() {
            self.apply_required_remote_skills(&leased_agent, required_skills)?;
        }
        self.ensure_required_remote_mcps_available(&leased_agent, &required_mcps)?;
        self.ensure_home_proxy_manifest_has_no_worker_collisions(
            &leased_agent,
            &required_mcps,
            &remote_extension_manifest,
            "launch remote native provider run",
        )?;
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
                mcp_servers.push(crate::mcp::CharioxMcpServerConfig::streamable_http(
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
            let resume_state = ProviderResumeState::from_external_provider_session(
                adapter_key,
                provider_session_id,
            );
            if !resume_state.is_empty() {
                request = request.with_resume_state(resume_state);
            }
        }
        Ok(request)
    }

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
        required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let request = self.prepare_leased_native_provider_launch(
            leased_agent_id,
            adapter_key,
            provider,
            account_profile,
            model,
            variant,
            structured_endpoint,
            provider_session_id,
            required_mcps,
            required_skills,
            remote_extension_manifest,
        )?;
        self.app.launch_leased_provider(request)
    }

    pub(crate) fn send_leased_native_provider_input(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        attachment_id: &str,
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
        let source_attachment_id =
            leased_native_source_attachment_id(leased_agent_id, attachment_id);
        crate::app::terminal_input::ProviderTerminalInput::new(self.app)
            .send_unattached_provider_input(
                provider_run.session_id(),
                provider_run_id,
                &source_attachment_id,
                &bytes,
            )?;
        Ok(byte_count)
    }

    pub(crate) fn resize_leased_provider_terminal(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let provider_run = self.app.providers.get_run(provider_run_id)?;
        if provider_run.session_id() != leased_agent.backing_session_id
            || provider_run.agent_instance_id() != Some(leased_agent.backing_agent_id.as_str())
        {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: leased_agent.backing_session_id,
                provider_run_id: provider_run_id.to_string(),
            });
        }
        let backing_session_id = provider_run.session_id().to_string();
        crate::app::KernelSessionService::new(self.app).resize_provider_terminal(
            &backing_session_id,
            provider_run_id,
            cols,
            rows,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{DaemonApp, RemoteLeaseRuntime};
    use crate::config::DaemonConfig;
    use crate::mcp::CharioxMcpServerConfig;
    use crate::transport::relay_peer::RequiredRemoteMcp;

    #[test]
    fn leased_native_provider_launch_preserves_required_mcp_set() {
        let _guard = crate::env_lock::lock();
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let worktree = std::env::temp_dir().join(format!(
            "chariox-native-provider-mcp-test-{}",
            std::process::id()
        ));
        let isolation_root = std::env::temp_dir().join(format!(
            "chariox-native-provider-mcp-isolation-test-{}",
            std::process::id()
        ));
        std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation_root);
        std::env::set_var("CHARIOX_SLICE_MACHINE_ID", "slice:native-provider-test");
        std::fs::create_dir_all(&worktree).expect("worktree should create");
        let mut runtime = RemoteLeaseRuntime::new(&mut app);
        let lease = runtime
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "local-user",
            )
            .expect("lease should create");
        let leased_agent = runtime
            .create_leased_agent(
                &lease.id,
                "dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                Some(worktree.display().to_string()),
                None,
            )
            .expect("leased agent should create");
        let mcp = CharioxMcpServerConfig::stdio(
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
                Some(Vec::new()),
                crate::extension::RemoteExtensionManifest::default(),
            )
            .expect("native run should launch");
        let colliding_attachment_id = leased_agent.backing_attachment_id.clone();
        let byte_count = runtime
            .send_leased_native_provider_input(
                &leased_agent.id,
                run.id(),
                &colliding_attachment_id,
                "eA==",
            )
            .expect("native input should be sent");
        assert_eq!(byte_count, 1);
        runtime
            .resize_leased_provider_terminal(&leased_agent.id, run.id(), 80, 24)
            .expect("native provider terminal should resize");
        drop(runtime);

        assert_eq!(run.mcp_servers().len(), 1);
        assert_eq!(run.mcp_servers()[0].name, "browser");
        assert!(!run.client_interface().is_chariox());
        let expected_source_attachment_id = format!(
            "remote-native:{}:{}",
            leased_agent.id, colliding_attachment_id
        );
        assert_eq!(
            app.terminal()
                .input_records()
                .last()
                .map(|record| record.source_attachment_id.as_str()),
            Some(expected_source_attachment_id.as_str())
        );
        assert_ne!(
            app.terminal()
                .input_records()
                .last()
                .map(|record| record.source_attachment_id.as_str()),
            Some(leased_agent.backing_attachment_id.as_str()),
            "home attachment ids must not collide with the worker backing attachment",
        );
        assert_eq!(app.pty().size(run.id()), Some((80, 24)));
        assert!(crate::mcp::CharioxMcpRegistry::user_root()
            .expect("isolated user MCP root should resolve")
            .join("browser.json")
            .exists());
        std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
        std::env::remove_var("CHARIOX_SLICE_MACHINE_ID");
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
            "chariox-native-provider-home-proxy-mcp-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&worktree).expect("worktree should create");
        let mut runtime = RemoteLeaseRuntime::new(&mut app);
        let lease = runtime
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "local-user",
            )
            .expect("lease should create");
        let leased_agent = runtime
            .create_leased_agent(
                &lease.id,
                "dev-stub",
                "default",
                Some("default".to_string()),
                None,
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
                Some(Vec::new()),
                manifest.clone(),
            )
            .expect("native run should launch");

        assert_eq!(run.remote_extension_manifest(), &manifest);
        assert_eq!(run.mcp_servers().len(), 1);
        assert_eq!(run.mcp_servers()[0].name, "home_browser");
        let _ = std::fs::remove_dir_all(&worktree);
    }

    #[test]
    fn standard_home_worker_does_not_install_required_mcp_payload() {
        let _guard = crate::env_lock::lock();
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let worktree = std::env::temp_dir().join(format!(
            "chariox-standard-native-provider-mcp-test-{}",
            std::process::id()
        ));
        let isolation_root = std::env::temp_dir().join(format!(
            "chariox-standard-native-provider-mcp-isolation-test-{}",
            std::process::id()
        ));
        std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation_root);
        std::env::remove_var("CHARIOX_SLICE_MACHINE_ID");
        std::fs::create_dir_all(&worktree).expect("worktree should create");
        let mut runtime = RemoteLeaseRuntime::new(&mut app);
        let lease = runtime
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "local-user",
            )
            .expect("lease should create");
        let leased_agent = runtime
            .create_leased_agent(
                &lease.id,
                "dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                Some(worktree.display().to_string()),
                None,
            )
            .expect("leased agent should create");
        let mcp = CharioxMcpServerConfig::stdio(
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

        let result = runtime.launch_leased_native_provider_run(
            &leased_agent.id,
            "dev-stub",
            "dev-stub",
            "default",
            "default",
            None,
            None,
            None,
            vec![required],
            Some(Vec::new()),
            crate::extension::RemoteExtensionManifest::default(),
        );

        std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
        let _ = std::fs::remove_dir_all(&worktree);
        let _ = std::fs::remove_dir_all(&isolation_root);
        let error = result.expect_err("standard worker must require a preinstalled MCP");
        assert!(error.to_string().contains("missing on worker"));
    }
}
