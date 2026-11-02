use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn launch_provider_request_from_local_request(
        &self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> crate::provider::LaunchProviderRequest {
        let adapter_key =
            crate::provider::adapter_key_for_provider(&request.adapter_key).to_string();
        let mut launch_request = crate::provider::LaunchProviderRequest::new(
            request.session_id.clone(),
            adapter_key,
            request.provider,
            request.account_profile,
            request.model,
        )
        .with_variant(request.variant);
        if let Some(endpoint) = request.structured_endpoint {
            launch_request = launch_request.with_structured_endpoint(endpoint);
        }
        if request.native_tui {
            launch_request = launch_request
                .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
        }
        if let Some(provider_session_id) = request.provider_session_id {
            let resume_state = crate::provider::ProviderResumeState::from_external_provider_session(
                &launch_request.adapter_key,
                provider_session_id,
            );
            if !resume_state.is_empty() {
                launch_request = launch_request.with_resume_state(resume_state);
            }
        }
        if let Some(binding) = request.native_checkpoint_binding {
            launch_request = launch_request.with_native_checkpoint_binding(binding);
        }
        let config = self.config_projection.snapshot();
        let session = self.session_store.get_session(&request.session_id).ok();
        let workspace_live_sync_mode =
            crate::provider::provider_workspace_live_sync_mode_for_session(
                &launch_request.provider,
                &config,
                session.as_ref(),
            );
        launch_request = launch_request.with_workspace_live_sync_mode(workspace_live_sync_mode);
        if let Some(agent_id) = request.agent_id.clone().or_else(|| {
            self.session_store
                .get_session(&request.session_id)
                .ok()
                .and_then(|session| session.focused_agent_id().map(str::to_string))
                .or_else(|| {
                    self.agent_store
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                })
        }) {
            launch_request = if let Ok(agent) = self.agent_store.get_agent(&agent_id) {
                let session = self.session_store.get_session(&request.session_id).ok();
                let effective_config = session
                    .as_ref()
                    .map(|session| {
                        crate::session::effective_agent_execution_config(session, Some(&agent))
                    })
                    .unwrap_or_default();
                launch_request
                    .with_agent_id(agent_id)
                    .with_owner_user_id(agent.owner_user_id().to_string())
                    .with_execution_mode(effective_config.mode)
                    .with_permission_level(effective_config.permission_level)
            } else {
                launch_request.with_agent_id(agent_id)
            };
        } else {
            let session = self.session_store.get_session(&request.session_id).ok();
            let effective_config = session
                .as_ref()
                .map(|session| crate::session::effective_agent_execution_config(session, None))
                .unwrap_or_default();
            launch_request = launch_request
                .with_execution_mode(effective_config.mode)
                .with_permission_level(effective_config.permission_level);
        }
        launch_request
    }

    pub(super) fn prepare_provider_launch_request(
        &self,
        request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let request = self.prepare_provider_launch_request_without_account_credentials(
            request,
            runtime_mcp_url,
        )?;
        let request = self.attach_provider_account_credentials(request)?;
        self.attach_project_environment(request)
    }

    // MP-08 / MP-10 / MP-11: a lease selects the transferred replica explicitly,
    // before the common account authority resolves home aliases.
    pub(super) fn prepare_leased_provider_launch_account(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        if crate::provider::canonical_provider_family(&request.provider).is_none() {
            return Ok(request);
        }
        let session = self.session_store.get_session(&request.session_id)?;
        request.owner_user_id = request
            .agent_id
            .as_deref()
            .map(|id| {
                self.agent_store
                    .get_agent(id)
                    .map(|agent| agent.owner_user_id().to_string())
            })
            .transpose()?
            .unwrap_or_else(|| session.owner_user_id().to_string());
        let profile = self.provider_account_profiles.get(
            &request.owner_user_id,
            &request.provider,
            &request.account_profile,
        )?;
        let selection = request.account_profile.clone();
        request.account_profile = profile.profile_id;
        let owner = request.owner_user_id.clone();
        crate::account_profile::bind_provider_account_authority(&mut request, owner, selection);
        Ok(request)
    }

    #[cfg(test)]
    pub(super) fn prepare_leased_provider_launch_request(
        &self,
        request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let request = self.prepare_leased_provider_launch_account(request)?;
        self.prepare_provider_launch_request(request, runtime_mcp_url)
    }

    pub(super) fn prepare_workflow_provider_launch_request(
        &self,
        request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let request = self.prepare_provider_launch_request_without_account_credentials(
            request,
            runtime_mcp_url,
        )?;
        if self.provider_launch_request_uses_vaulted_account_credential(&request)? {
            return self.attach_project_environment(request);
        }
        let request = self.attach_provider_account_credentials(request)?;
        self.attach_project_environment(request)
    }

    pub(super) fn prepare_provider_launch_account(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let session = self.session_store.get_session(&request.session_id)?;
        let config = self.config_projection.snapshot();
        if request.agent_id.is_none() {
            request.agent_id = self
                .session_store
                .get_session(&request.session_id)?
                .focused_agent_id()
                .map(str::to_string)
                .or_else(|| {
                    self.agent_store
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                });
        }
        let agent = request
            .agent_id
            .as_deref()
            .and_then(|agent_id| self.agent_store.get_agent(agent_id).ok());
        if let Some(agent) = agent.as_ref() {
            if agent.session_id() != session.id() {
                return Err(DaemonError::AgentNotInSession {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                });
            }
            // Return must probe its native session before committing the local
            // route. Only its durable preparation may cross this account guard.
            if agent.remote_execution().is_some_and(|source| {
                !self
                    .agent_moves
                    .prepares_return(&request, source, &config.daemon_id)
            }) {
                return Err(DaemonError::LocalTransport {
                    operation: "launch provider run",
                    message: format!(
                        "agent `{}` is remote-backed and must launch its provider on the worker kernel",
                        agent.id()
                    ),
                });
            }
            request = request.with_owner_user_id(agent.owner_user_id().to_string());
        } else {
            request = request.with_owner_user_id(session.owner_user_id().to_string());
        }
        if crate::provider::canonical_provider_family(&request.provider)
            .is_some_and(|provider| matches!(provider, "codex" | "claude" | "opencode"))
        {
            let account_owner_user_id = match request.provider_account_owner_user_id.clone() {
                Some(owner) => owner,
                None => crate::account_profile::provider_account_authority_for_launch(
                    &config,
                    &self.provider_account_profiles,
                    &request,
                )?,
            };
            let profile = self.provider_account_profiles.get(
                &account_owner_user_id,
                &request.provider,
                &request.account_profile,
            )?;
            let provider_account_env = self.provider_account_profiles.resolve_environment(
                &account_owner_user_id,
                &request.provider,
                &profile.profile_id,
            )?;
            let selection = request
                .resolved_provider_account
                .as_ref()
                .map(|account| account.selection().to_string())
                .unwrap_or_else(|| request.account_profile.clone());
            request.account_profile = profile.profile_id;
            crate::account_profile::bind_provider_account_authority(
                &mut request,
                account_owner_user_id,
                selection,
            );
            request = request.with_provider_account_env(provider_account_env);
        }
        Ok(request)
    }

    pub(super) fn prepare_provider_launch_request_without_account_credentials(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        request.adapter_key =
            crate::provider::adapter_key_for_provider(&request.adapter_key).to_string();
        request = self.prepare_provider_launch_account(request)?;
        let session = self.session_store.get_session(&request.session_id)?;
        let session = request
            .agent_id
            .as_deref()
            .map(|agent| self.agent_moves.execution_session(&session, agent))
            .transpose()?
            .unwrap_or(session);
        let config = self.config_projection.snapshot();
        let agent = request
            .agent_id
            .as_deref()
            .and_then(|agent_id| self.agent_store.get_agent(agent_id).ok());
        let effective_config =
            crate::session::effective_agent_execution_config(&session, agent.as_ref());
        if request.execution_mode.is_none() {
            request = request.with_execution_mode(effective_config.mode);
        }
        if request.permission_level.is_none() {
            request = request.with_permission_level(effective_config.permission_level);
        }
        if request.resume_state.is_none()
            || agent.as_ref().is_some_and(|agent| {
                agent
                    .provider_resume_state()
                    .required_native_session()
                    .is_some()
                    && request
                        .resume_state
                        .as_ref()
                        .is_none_or(|state| state.required_native_session().is_none())
            })
        {
            if let Some(agent) = agent.as_ref() {
                if agent
                    .provider_resume_state()
                    .required_native_session()
                    .is_some()
                    && request
                        .resume_state
                        .as_ref()
                        .and_then(|state| state.provider_session_id(&request.adapter_key))
                        .is_some_and(|id| {
                            Some(id)
                                != agent
                                    .provider_resume_state()
                                    .provider_session_id(&request.adapter_key)
                        })
                {
                    return Err(crate::provider::NativeCheckpointFailure::SessionChanged
                        .into_daemon_error());
                }

                let resume_state = crate::app::sanitize_resume_state_for_launch(&request, agent);
                if !resume_state.is_empty() {
                    request = request.with_resume_state(resume_state);
                }
            }
        }
        if request.working_directory.is_none() {
            let agent_worktree = agent
                .as_ref()
                .and_then(|agent| agent.worktree_id().map(std::path::PathBuf::from));
            request.working_directory = Some(
                agent_worktree.unwrap_or_else(|| std::path::PathBuf::from(session.worktree_id())),
            );
        }
        if request.uses_workspace_live_sync() && request.workspace_live_sync_roots.is_empty() {
            let workspace_live_sync_roots = crate::app::workspace_live_sync_protected_roots(
                &session,
                request.working_directory.as_deref(),
                &config.host_machine_id,
                &config.daemon_id,
            );
            request = request.with_workspace_live_sync_roots(workspace_live_sync_roots);
        }
        if !(crate::provider::managed_provider_isolation_required()
            && request.uses_workspace_live_sync())
        {
            if let Some(working_directory) = request.working_directory.as_deref() {
                crate::git_worktree_placement::preflight_working_directory(
                    working_directory,
                    "launch provider run",
                    false,
                    &[],
                )?;
            }
        }
        if request.runtime_mcp_binding.is_none() {
            let shared_auth_token = request
                .agent_id
                .is_none()
                .then(|| {
                    self.provider_store
                        .get_session_run_for_provider(&request.session_id, &request.provider)
                        .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string))
                })
                .flatten();
            request = request.with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                runtime_mcp_url,
                shared_auth_token.unwrap_or_else(crate::app::generate_runtime_mcp_auth_token),
            ));
        }
        if request.provider_env_remove.is_empty() {
            request =
                request.with_provider_env_remove(crate::app::default_provider_env_remove(&config));
        }
        for name in crate::account_profile::provider_auth_env_vars(&request.provider) {
            if !request
                .provider_env_remove
                .iter()
                .any(|existing| existing == name)
            {
                request.provider_env_remove.push((*name).to_string());
            }
        }
        if request.mcp_servers.is_empty() {
            if let Some(agent) = agent.as_ref() {
                request =
                    request.with_mcp_servers(crate::app::granted_mcp_servers_for_agent_launch(
                        "provider.launch.mcps",
                        &session,
                        agent,
                    )?);
            }
        }
        let mcp_servers = std::mem::take(&mut request.mcp_servers);
        request = request.with_mcp_servers(crate::app::resolve_mcp_credentials_for_launch(
            &config,
            mcp_servers,
        )?);
        request = crate::app::apply_metaagent_launch_policy(request, agent.as_ref());
        request = super::restore_agent_move_preparation_environment(
            &config,
            &self.durable_state_store,
            &self.agent_moves,
            request,
        )?;
        if let Some(checkpoint) = self.agent_moves.checkpoint_for_launch(&request)? {
            request = request.with_native_checkpoint(checkpoint);
        }
        Ok(request)
    }

    fn attach_project_environment(
        &self,
        request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let session = self.session_store.get_session(&request.session_id)?;
        let session = request
            .agent_id
            .as_deref()
            .map(|agent| self.agent_moves.execution_session(&session, agent))
            .transpose()?
            .unwrap_or(session);
        let agent = request
            .agent_id
            .as_deref()
            .and_then(|id| self.agent_store.get_agent(id).ok());
        crate::project_environment::attach_project_provider_environment(
            &self.config_projection.snapshot(),
            &session,
            agent.as_ref(),
            request,
        )
    }

    pub(super) fn claude_enrollment_is_rejected(
        &self,
        request: &crate::provider::LaunchProviderRequest,
    ) -> Result<bool, DaemonError> {
        if crate::provider::canonical_provider_family(&request.provider) != Some("claude") {
            return Ok(false);
        }
        let owner = crate::account_profile::provider_account_authority_for_launch(
            &self.config_projection.snapshot(),
            &self.provider_account_profiles,
            request,
        )?;
        let profile =
            self.provider_account_profiles
                .get(&owner, "claude", &request.account_profile)?;
        if !crate::provider::provider_account_credential_registered(
            &owner,
            "claude",
            &profile.profile_id,
        )? {
            return Ok(false);
        }
        let id =
            crate::provider::provider_account_credential_id(&owner, "claude", &profile.profile_id);
        Ok(
            profile.auth_state == crate::account_profile::ProviderAccountAuthState::Expired
                || self.claude_token_checks.is_rejected(&id),
        )
    }

    fn attach_provider_account_credentials(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        if !request.provider_credential_env.is_empty()
            || self.claude_enrollment_is_rejected(&request)?
        {
            return Ok(request);
        }
        if crate::provider::canonical_provider_family(&request.provider)
            .is_some_and(|provider| matches!(provider, "codex" | "claude" | "opencode"))
        {
            let config = self.config_projection.snapshot();
            let account_owner_user_id = match request.provider_account_owner_user_id.clone() {
                Some(owner) => owner,
                None => crate::account_profile::provider_account_authority_for_launch(
                    &config,
                    &self.provider_account_profiles,
                    &request,
                )?,
            };
            let provider_credential_env = match crate::app::handed_over_credentials(&request) {
                Some(credentials) => credentials,
                None => crate::provider::resolve_provider_account_credentials_for_launch(
                    &config,
                    &self.provider_account_profiles,
                    &account_owner_user_id,
                    &request.provider,
                    &request.account_profile,
                    request.client_interface,
                )?,
            };
            request = request.with_provider_credential_env(provider_credential_env);
        }
        Ok(request)
    }

    fn provider_launch_request_uses_vaulted_account_credential(
        &self,
        request: &crate::provider::LaunchProviderRequest,
    ) -> Result<bool, DaemonError> {
        if self.claude_enrollment_is_rejected(request)? {
            return Ok(false);
        }
        let config = self.config_projection.snapshot();
        if config.user_config.credential_vault.backend
            != crate::config::CredentialVaultBackend::CharioxEncrypted
        {
            return Ok(false);
        }
        let account_owner_user_id = match request.provider_account_owner_user_id.clone() {
            Some(owner) => owner,
            None => crate::account_profile::provider_account_authority_for_launch(
                &config,
                &self.provider_account_profiles,
                request,
            )?,
        };
        crate::provider::launch_uses_vault_credential(
            &self.provider_account_profiles,
            &account_owner_user_id,
            &request.provider,
            &request.account_profile,
            request.client_interface,
        )
    }
}

#[cfg(test)]
pub(super) async fn owned_runtime_state(
    app: &Arc<Mutex<crate::app::DaemonApp>>,
) -> KernelRuntimeState {
    let (
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    ) = {
        let app = app.lock().await;
        (
            app.config_projection_store(),
            app.session_state_store(),
            app.agents().clone(),
            app.attachments().clone(),
            app.providers().clone(),
            app.provider_process_tracking_store(),
            app.slices(),
            app.session_state_projection_store(),
            app.provider_run_projection_store(),
            app.operational_history_store(),
            app.durable_state_store(),
            app.prompt_state_owner(),
            app.active_turn_store(),
            app.prompt_activity_store(),
            app.prompt_workspace_claim_store(),
            app.structured_output_record_store(),
            app.terminal_stream_store(),
            app.workflow_design_event_store(),
            app.metaagent_event_store(),
            app.workspace_coordinator(),
        )
    };
    KernelRuntimeState::new_with_owned_state(
        Arc::clone(app),
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn mp08_mp11_refused_delegated_grant_preserves_kernel_enrollment() {
        crate::test_support::isolated_env_test!();
        use std::io::{Read, Write};
        let root = crate::test_support::TestWorktree::new("delegated-refusal");
        std::env::set_var("CHARIOX_HOME", root.path().join("home"));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = [0; 8192];
            let _ = stream.read(&mut bytes).unwrap();
            let body =
                r#"{"error":{"code":"session_invalid","message":"receiving terminal revoked"}}"#;
            write!(stream, "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let mut config = crate::config::DaemonConfig::for_tests();
        let original = crate::config::PersistedCloudRelayProfile {
            api_url,
            kernel_id: Some(config.daemon_id.clone()),
            kernel_credential: Some("synthetic-kernel-grant".into()),
            user_id: "cloud-owner".into(),
            account_id: "account".into(),
            realm_id: "realm".into(),
            ..Default::default()
        };
        config.cloud_relay = Some(original.clone());
        let app = Arc::new(Mutex::new(
            crate::app::DaemonApp::bootstrap(config).unwrap(),
        ));
        let runtime = owned_runtime_state(&app).await;
        let result = crate::runtime::cloud_relay_connection_executor::execute_issue_cloud_relay_client_token_request(&runtime, &runtime.owned.config_projection, crate::local::IssueCloudRelayClientTokenRequest { target_daemon_alias:"kernel-other".into(), client_id:"revoked-terminal".into(), session_id:None, public_key_thumbprint:Some("a".repeat(64)) }).await;
        server.join().unwrap();
        assert!(result.is_err());
        let retained = runtime
            .owned
            .config_projection
            .snapshot()
            .cloud_relay
            .expect("delegated refusal must not clear enrollment");
        assert_eq!(retained.kernel_id, original.kernel_id);
        assert_eq!(retained.kernel_credential, original.kernel_credential);
    }

    #[tokio::test]
    async fn mp08_mp11_refused_legacy_terminal_pairing_grant_preserves_machine_enrollment() {
        crate::test_support::isolated_env_test!();
        use std::io::{Read, Write};
        let root = crate::test_support::TestWorktree::new("legacy-pairing-refusal");
        std::env::set_var("CHARIOX_HOME", root.path().join("home"));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = [0; 8192];
            let _ = stream.read(&mut bytes).unwrap();
            let body =
                r#"{"error":{"code":"session_invalid","message":"receiving terminal revoked"}}"#;
            write!(stream, "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".into());
        config.relay_token = Some("synthetic-relay-token".into());
        let original = crate::config::PersistedCloudRelayProfile {
            api_url,
            relay_url: "wss://relay.example.test".into(),
            machine_id: Some("legacy-machine".into()),
            machine_credential: Some("synthetic-machine-credential".into()),
            user_id: "cloud-owner".into(),
            account_id: "account".into(),
            realm_id: "realm".into(),
            ..Default::default()
        };
        config.cloud_relay = Some(original.clone());
        let app = Arc::new(Mutex::new(
            crate::app::DaemonApp::bootstrap(config).unwrap(),
        ));
        let runtime = owned_runtime_state(&app).await;
        let result =
            crate::runtime::pairing_invite_executor::execute_create_terminal_pairing_link_request(
                &runtime,
                &runtime.owned.config_projection,
                crate::local::CreateTerminalPairingLinkRequest {
                    terminal_type: None,
                    alias: None,
                    expires_in_ms: None,
                },
            )
            .await;
        server.join().unwrap();
        assert!(result.is_err());
        let retained = runtime
            .owned
            .config_projection
            .snapshot()
            .cloud_relay
            .expect("legacy terminal grant refusal must not clear enrollment");
        assert_eq!(retained.machine_id, original.machine_id);
        assert_eq!(retained.machine_credential, original.machine_credential);
    }

    #[tokio::test]
    async fn mp08_mp11_claude_first_use_uses_launch_bound_replica() {
        crate::test_support::isolated_env_test!();
        let _claude = crate::test_support::TestClaudeExecutable::new();
        let root = crate::test_support::TestWorktree::new("claude-first-use-replica");
        std::env::set_var("CHARIOX_HOME", root.path().join("home"));
        let mut config = crate::config::DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "cloud-owner".into(),
            ..Default::default()
        });
        let app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let registry = app.provider_account_profile_registry();
        let profile = registry
            .create_managed("cloud-owner", "claude", "Replica")
            .unwrap();
        registry
            .update_observation(
                "cloud-owner",
                "claude",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let mut request = crate::provider::LaunchProviderRequest::new(
            "s",
            "claude",
            "claude",
            &profile.profile_id,
            "haiku",
        );
        request.owner_user_id = "cloud-owner".into();
        crate::account_profile::bind_provider_account_authority(
            &mut request,
            "cloud-owner".into(),
            profile.profile_id.clone(),
        );
        let mut credentials = crate::provider::ProviderCredentialEnvironment::default();
        credentials.insert(
            "CLAUDE_CODE_OAUTH_TOKEN",
            zeroize::Zeroizing::new("synthetic-setup-token".into()),
        );
        assert!(runtime
            .checked_claude_credentials(&request, credentials)
            .await
            .is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mp08_mp10_owned_native_claude_initializes_before_credentials_are_separated() {
        crate::test_support::isolated_env_test!();
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-native-onboarding-{}",
            std::process::id()
        ));
        let profile = root.join("selected");
        std::fs::create_dir_all(&profile).unwrap();
        let executable = root.join("claude");
        std::fs::write(&executable, "#!/bin/sh\nsleep 60\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("CHARIOX_CLAUDE_BIN", &executable);
        let mut app =
            crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new("home", "home"))
            .unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let mut credentials = crate::provider::ProviderCredentialEnvironment::default();
        credentials.insert(
            "CLAUDE_CODE_OAUTH_TOKEN",
            zeroize::Zeroizing::new("synthetic-setup-token".into()),
        );
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            "selected",
            "haiku",
        )
        .with_agent_id(agent.id())
        .with_client_interface(crate::provider::ProviderClientInterface::NativeTui)
        .with_provider_account_env(std::collections::BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".into(),
            profile.to_string_lossy().into_owned(),
        )]))
        .with_provider_credential_env(credentials);
        let started = runtime.owned.start_provider_launch(request).unwrap();
        assert!(started
            .provider_credential_env
            .iter()
            .any(|(name, _)| name == "CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(
            !started
                .run
                .pty_env()
                .contains_key("CLAUDE_CODE_OAUTH_TOKEN"),
            "credentials must remain outside persisted launch metadata"
        );
        let state:serde_json::Value = serde_json::from_slice(&std::fs::read(profile.join(".claude.json")).expect("owned native launch must initialize selected onboarding before separating credentials")).unwrap();
        assert_eq!(state["hasCompletedOnboarding"], true);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn home_default_launch_keeps_authority_when_replica_has_same_profile_id() {
        crate::test_support::isolated_env_test!();
        let _claude = crate::test_support::TestClaudeExecutable::new();
        let root = crate::test_support::TestWorktree::new("home-default-account-authority");
        std::env::set_var("CHARIOX_HOME", root.path().join("home"));
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "cloud-owner".into(),
            ..Default::default()
        });
        let vault_path = root.path().join("home/credentials.vault");
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = vault_path.display().to_string();
        let mut app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                crate::session::CreateSessionRequest::new(
                    workspace.to_string_lossy(),
                    workspace.to_string_lossy(),
                )
                .with_owner_user_id("cloud-owner"),
            )
            .unwrap();
        let registry = app.provider_account_profile_registry();
        let profile = registry.create_managed("local", "claude", "Home").unwrap();
        registry
            .set_default("local", "claude", &profile.profile_id)
            .unwrap();
        let local_env = registry
            .resolve_environment("local", "claude", &profile.profile_id)
            .unwrap();
        let mut replica = registry
            .export_materialization("local", "claude", &profile.profile_id)
            .unwrap();
        replica.profile.owner_user_id = "cloud-owner".into();
        registry
            .materialize_replica("cloud-owner", &replica)
            .unwrap();
        assert_eq!(
            registry
                .get("local", "claude", "default")
                .unwrap()
                .profile_id,
            profile.profile_id
        );
        assert_eq!(
            registry
                .get("cloud-owner", "claude", &profile.profile_id)
                .unwrap()
                .profile_id,
            profile.profile_id
        );
        assert_ne!(
            registry
                .resolve_environment("cloud-owner", "claude", &profile.profile_id)
                .unwrap(),
            local_env
        );
        crate::secret::unlock_chariox_encrypted_vault(
            &vault_path,
            "test passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        crate::provider::store_provider_account_credential(
            &config,
            "local",
            "claude",
            &profile.profile_id,
            "synthetic-home-token",
            false,
        )
        .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::for_user(
                session.id(),
                "account-next-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
                "cloud-owner",
            ))
            .unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            "default",
            "haiku",
        )
        .with_agent_id(agent.id());
        let workflow = runtime
            .owned
            .prepare_workflow_provider_launch_request(request.clone(), config.runtime_mcp_url())
            .expect("workflow preparation must keep the home Vault decision");
        assert!(
            runtime
                .owned
                .provider_launch_request_uses_vaulted_account_credential(&workflow)
                .unwrap(),
            "resolved home default must retain the local vaulted credential"
        );
        let prepared = runtime
            .owned
            .prepare_provider_launch_request(request, config.runtime_mcp_url())
            .expect("home credential must attach despite the credential-less replica");
        assert_eq!(prepared.provider_account_env, local_env);
        assert!(prepared
            .provider_credential_env
            .iter()
            .any(|(name, value)| name == "CLAUDE_CODE_OAUTH_TOKEN"
                && value == "synthetic-home-token"));
        let repeated = runtime
            .owned
            .prepare_provider_launch_request(prepared.clone(), config.runtime_mcp_url())
            .unwrap();
        assert_eq!(repeated.provider_account_env, local_env);
        let run = crate::provider::RuntimeProviderRun::new(
            "account-authority-regression",
            &prepared,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "account-authority-regression".into(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: local_env.clone(),
                pty_env_remove: Vec::new(),
                working_directory: prepared.working_directory.clone(),
                structured_endpoint: None,
            },
        );
        // MP-08 / MP-10 / MP-11: finish the real launch state transition,
        // then admit a second prompt while the same-id replica is expired.
        registry
            .update_observation(
                "local",
                "claude",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        registry
            .update_observation(
                "cloud-owner",
                "claude",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Expired,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        let started = crate::app::StartedProviderLaunch {
            run: run.clone(),
            previous_active_run_id: None,
            provider_credential_env: prepared.provider_credential_env.clone(),
        };
        let completed = runtime
            .owned
            .finish_provider_launch_success(&started, None)
            .unwrap();
        let source = attachment.id();
        let prompt = crate::app::KernelPreparedPromptSubmission {
            session_id: session.id().into(),
            prompt: crate::session::PromptQueueItem::new(
                "account-next-turn",
                source,
                agent.id(),
                "continue on the authenticated home account",
                crate::session::PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        };
        assert!(runtime
            .owned
            .submit_local_prepared_prompt_for_provider_run(&prompt, Some(completed.id()))
            .expect("next prompt must use the completed home run's account authority")
            .is_some());
        let completed_agent = runtime.owned.agent_store.get_agent(agent.id()).unwrap();
        assert_eq!(
            completed_agent.provider_account_profile(),
            "default",
            "launch completion must retain the home selection alias"
        );
        let restored_completed: crate::provider::RuntimeProviderRun =
            serde_json::from_value(serde_json::to_value(&completed).unwrap()).unwrap();
        runtime
            .owned
            .require_agent_account_authenticated(
                &completed_agent,
                Some(&restored_completed),
                "MP-08 / MP-10 / MP-11 restored home admission",
            )
            .unwrap();
        assert!(runtime.owned.provider_account_allows_queued_prompt_advance(
            session.id(),
            &completed_agent,
            "MP-08 / MP-10 / MP-11 home queue"
        ));
        // MP-08 / MP-10 / MP-11: recover from the current authenticated
        // launch before testing expired-account denial. Expired credentials
        // now enter the product login recovery flow rather than bypassing it.
        let credentials = runtime
            .resolve_provider_account_credentials_for_run_with_vault(
                &completed,
                "MP-08 / MP-10 / MP-11 home recovery",
            )
            .await
            .unwrap();
        assert!(credentials
            .iter()
            .any(|(name, value)| name == "CLAUDE_CODE_OAUTH_TOKEN"
                && value == "synthetic-home-token"));
        registry
            .update_observation(
                "local",
                "claude",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Expired,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        registry
            .update_observation(
                "cloud-owner",
                "claude",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        // MP-08 / MP-10 / MP-11: #945 admits a registered Claude token
        // for its first-use check even after an expired observation. Remove
        // this disposable home credential to test denial without that exception.
        runtime
            .owned
            .require_agent_account_authenticated(
                &completed_agent,
                Some(&completed),
                "MP-08 / MP-10 / MP-11 registered Claude first use",
            )
            .unwrap();
        let reload = super::super::provider_reload::policy_reload_launch_request(
            &run,
            agent.id(),
            crate::provider::ProviderResumeState::default(),
        );
        let reloaded = runtime
            .owned
            .prepare_provider_launch_request(reload, config.runtime_mcp_url())
            .unwrap();
        assert_eq!(reloaded.provider_account_env, local_env);
        let serialized = serde_json::to_value(&run).unwrap();
        assert!(serialized.get("resolved_provider_account").is_none());
        let restored: crate::provider::RuntimeProviderRun =
            serde_json::from_value(serialized).unwrap();
        let restored_reload = super::super::provider_reload::policy_reload_launch_request(
            &restored,
            agent.id(),
            crate::provider::ProviderResumeState::default(),
        );
        let restored_prepared = runtime
            .owned
            .prepare_provider_launch_request(restored_reload, config.runtime_mcp_url())
            .expect("restored account roots must retain the home authority");
        assert_eq!(restored_prepared.provider_account_env, local_env);
        // Managed providers retain roots in --setenv and host bind arguments,
        // not in the scrubbed parent environment. Restoring those runs must
        // preserve the same authority without a new serialized field.
        let mut managed = serde_json::to_value(&run).unwrap();
        let source = local_env.get("CLAUDE_CONFIG_DIR").unwrap();
        let destination = "/home/chariox/.provider-account/root-1";
        managed["pty_env"] = serde_json::json!({});
        managed["pty_args"] = serde_json::json!([
            "--setenv",
            crate::provider::MANAGED_PROVIDER_ISOLATION_MARKER_ENV,
            "1",
            "--bind",
            source,
            destination,
            "--setenv",
            "CLAUDE_CONFIG_DIR",
            destination,
            "--",
            "claude",
        ]);
        let managed: crate::provider::RuntimeProviderRun = serde_json::from_value(managed).unwrap();
        let managed_reload = super::super::provider_reload::policy_reload_launch_request(
            &managed,
            agent.id(),
            crate::provider::ProviderResumeState::default(),
        );
        let managed_prepared = runtime
            .owned
            .prepare_provider_launch_request(managed_reload, config.runtime_mcp_url())
            .expect("restored managed bindings must retain home authority");
        assert_eq!(managed_prepared.provider_account_env, local_env);
        let mut tampered = repeated;
        tampered.owner_user_id = "another-owner".into();
        assert!(
            crate::account_profile::provider_account_authority_for_launch(
                &config,
                &runtime.owned.provider_account_profiles,
                &tampered,
            )
            .is_err(),
            "resolution provenance cannot authorize another runtime owner"
        );
        // MP-08 / MP-10 / MP-11: remove the disposable credential only
        // after verifying reload authority with the credential still present.
        crate::credential::CharioxCredentialRegistry::user()
            .unwrap()
            .remove(&crate::provider::provider_account_credential_id(
                "local",
                "claude",
                &profile.profile_id,
            ))
            .unwrap();
        assert!(
            runtime
                .owned
                .submit_local_prepared_prompt_for_provider_run(&prompt, Some(completed.id()))
                .is_err(),
            "authenticated replica must never rescue an expired home run"
        );
        assert!(
            !runtime.owned.provider_account_allows_queued_prompt_advance(
                session.id(),
                &completed_agent,
                "MP-08 / MP-10 / MP-11 expired home queue"
            )
        );
        crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
        crate::secret::clear_vault_secret_process_cache().unwrap();
    }

    #[tokio::test]
    async fn owned_launch_preparation_preserves_account_without_injecting_project_repositories() {
        crate::test_support::isolated_env_test!();
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-provider-launch-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let primary = root.join("primary");
        let supporting = root.join("supporting");
        std::fs::create_dir_all(&primary).expect("primary workspace");
        std::fs::create_dir_all(&supporting).expect("supporting workspace");

        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                crate::session::CreateSessionRequest::new(
                    primary.to_string_lossy(),
                    primary.to_string_lossy(),
                )
                .with_project_selection(
                    crate::session::SessionProjectSelection::New { name: None },
                ),
            )
            .expect("session");
        app.sessions_mut()
            .update_project_workspaces(
                session.project_id(),
                vec![
                    primary.to_string_lossy().into_owned(),
                    supporting.to_string_lossy().into_owned(),
                ],
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .expect("project workspaces");
        let registry = app.provider_account_profile_registry();
        let profile = registry
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                "Owned launch test",
            )
            .expect("managed Codex profile");
        let environment = registry
            .resolve_environment(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                &profile.profile_id,
            )
            .expect("profile environment");
        std::fs::write(
            Path::new(&environment["CODEX_HOME"]).join("auth.json"),
            br#"{"tokens":{"access_token":"test"}}"#,
        )
        .expect("Codex credential fixture");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "codex",
            "codex",
            profile.profile_id,
            "gpt-5.6-luna",
        )
        .with_agent_id(agent.id())
        .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked);

        let _env = crate::env_lock::lock();
        let previous_isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, "1");
        let prepared = runtime
            .owned
            .prepare_provider_launch_request(request, "http://127.0.0.1:43120/mcp".to_string())
            .expect("owned launch preparation");
        match previous_isolation {
            Some(value) => {
                std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, value)
            }
            None => std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV),
        }

        assert_eq!(prepared.provider_account_env, environment);
        assert_eq!(prepared.workspace_live_sync_roots, vec![primary.clone()]);
        for name in ["OPENAI_API_KEY", "CODEX_API_KEY"] {
            assert!(prepared
                .provider_env_remove
                .iter()
                .any(|existing| existing == name));
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn claude_agents_launch_on_the_accounts_own_login_on_every_platform() {
        crate::test_support::isolated_env_test!();
        let _env = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-missing-claude-credential-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test root should exist");
        std::env::set_var("CHARIOX_HOME", &root);
        let mut app = crate::app::DaemonApp::bootstrap(
            crate::config::DaemonConfig::for_tests()
                .with_session_history_root(root.join("session-history")),
        )
        .expect("daemon bootstrap");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .expect("session should create");
        let profile = app
            .provider_account_profile_registry()
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "claude",
                "Missing Claude credential",
            )
            .expect("managed Claude profile should create");
        let claude_config_dir = std::path::PathBuf::from(
            app.provider_account_profile_registry()
                .resolve_environment(
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    "claude",
                    &profile.profile_id,
                )
                .expect("Claude environment should resolve")["CLAUDE_CONFIG_DIR"]
                .clone(),
        );
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "claude")
                    .with_account_profile(profile.profile_id.clone()),
            )
            .expect("Claude agent should create");
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id());
        let error = runtime
            .owned
            .prepare_provider_launch_request(
                request.clone(),
                "http://127.0.0.1:43120/mcp".to_string(),
            )
            .expect_err("a launch on a signed-out account must fail before spawn");
        assert!(error
            .to_string()
            .contains("this Claude account is not signed in"));

        let foreground = runtime
            .owned
            .prepare_provider_launch_request(
                request
                    .clone()
                    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui),
                "http://127.0.0.1:43120/mcp".to_string(),
            )
            .expect("native Claude TUI must remain available for interactive sign-in");
        assert!(foreground.provider_credential_env.is_empty());

        std::fs::write(
            claude_config_dir.join(".credentials.json"),
            br#"{"claudeAiOauth":{"refreshToken":"portable-refresh-token"}}"#,
        )
        .expect("portable Claude credential fixture should write");
        // One sign-in: the account's own login runs background agents on Linux
        // (its credential file) and macOS (its verified Keychain login) alike.
        app.lock()
            .await
            .provider_account_profile_registry()
            .update_observation(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "claude",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .expect("the verified login should record");
        let launch = runtime
            .owned
            .prepare_provider_launch_request(request, "http://127.0.0.1:43120/mcp".to_string())
            .expect("the account's own login runs background agents");
        assert!(
            launch.provider_credential_env.is_empty(),
            "no Chariox-vault token is needed"
        );
        // MP-08 / MP-11: explicit token enrollment selects the Vault credential
        // for every mode, even when a previous native login remains available.
        let credential_id = crate::provider::provider_account_credential_id(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
        );
        crate::credential::CharioxCredentialRegistry::user()
            .expect("credential registry should resolve")
            .upsert(crate::config::UserCredentialConfig {
                id: credential_id.clone(),
                description: None,
                source: crate::config::UserCredentialSourceConfig::Vault { key: credential_id },
                allowed_hosts: Vec::new(),
                allowed_uses: vec![crate::config::UserCredentialUse::Provider],
                injection: crate::config::UserCredentialInjectionConfig::Provider,
                metadata: None,
            })
            .expect("vault credential should register");
        assert!(crate::provider::provider_account_credential_uses_vault(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
        )
        .expect("MP-08/MP-11: a registered credential requires Vault admission for every mode"));
        drop(app);

        std::env::remove_var("CHARIOX_HOME");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn owned_managed_launch_does_not_inject_a_project_workspace_allowlist() {
        crate::test_support::isolated_env_test!();
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-managed-pool-launch-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let primary = root.join("primary");
        let supporting = root.join("supporting");
        let instance_worktree = root.join("workflow-runtime").join("instance-2");
        let unregistered_worktree = root.join("workflow-runtime").join("unregistered");
        let traversal_worktree = instance_worktree.join("..").join("unregistered");
        std::fs::create_dir_all(&primary).expect("primary workspace");
        std::fs::create_dir_all(&supporting).expect("supporting workspace");
        std::fs::create_dir_all(&instance_worktree).expect("runtime instance worktree");
        std::fs::create_dir_all(&unregistered_worktree).expect("unregistered worktree");

        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap");
        let (session, source_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                crate::session::CreateSessionRequest::new(
                    primary.to_string_lossy(),
                    primary.to_string_lossy(),
                )
                .with_project_selection(
                    crate::session::SessionProjectSelection::New { name: None },
                ),
            )
            .expect("session");
        app.sessions_mut()
            .update_project_workspaces(
                session.project_id(),
                vec![
                    primary.to_string_lossy().into_owned(),
                    supporting.to_string_lossy().into_owned(),
                ],
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .expect("project workspaces");
        let runtime_agent = app.agents().clone().materialize_workflow_runtime_agent(
            source_agent.clone(),
            session.id(),
            &instance_worktree.to_string_lossy(),
        );
        let unregistered_agent = app.agents().clone().materialize_workflow_runtime_agent(
            source_agent,
            session.id(),
            &unregistered_worktree.to_string_lossy(),
        );
        app.sessions_mut()
            .register_workflow_runtime_instance(
                session.id(),
                crate::session::WorkflowEndpointRuntimeInstance::new(
                    "instance-2",
                    "workflow-1",
                    "endpoint-1",
                    1,
                    2,
                    false,
                    std::collections::BTreeMap::from([(
                        "node-1".to_string(),
                        runtime_agent.id().to_string(),
                    )]),
                    instance_worktree.to_string_lossy(),
                ),
            )
            .expect("runtime instance should register");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "model",
        )
        .with_agent_id(runtime_agent.id())
        .with_workspace_live_sync_managed();

        let _env = crate::env_lock::lock();
        let previous_isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, "1");
        let prepared = runtime
            .owned
            .prepare_provider_launch_request(request, "http://127.0.0.1:43120/mcp".to_string());
        let unregistered = runtime.owned.prepare_provider_launch_request(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "model",
            )
            .with_agent_id(unregistered_agent.id())
            .with_workspace_live_sync_managed(),
            "http://127.0.0.1:43120/mcp".to_string(),
        );
        let traversal = runtime.owned.prepare_provider_launch_request(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "model",
            )
            .with_agent_id(runtime_agent.id())
            .with_working_directory(traversal_worktree.clone())
            .with_workspace_live_sync_managed(),
            "http://127.0.0.1:43120/mcp".to_string(),
        );
        #[cfg(unix)]
        let symlink_escape = {
            let link = instance_worktree.join("outside-link");
            std::os::unix::fs::symlink(&unregistered_worktree, &link)
                .expect("escape symlink fixture");
            runtime.owned.prepare_provider_launch_request(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "model",
                )
                .with_agent_id(runtime_agent.id())
                .with_working_directory(link)
                .with_workspace_live_sync_managed(),
                "http://127.0.0.1:43120/mcp".to_string(),
            )
        };
        restore_env(
            crate::provider::MANAGED_PROVIDER_ISOLATION_ENV,
            previous_isolation,
        );
        let prepared = prepared.expect("managed pool clone launch should prepare");
        let unregistered = unregistered.expect("unregistered hidden launch should prepare");
        let traversal = traversal.expect("traversal launch should prepare without expanding roots");
        #[cfg(unix)]
        let symlink_escape =
            symlink_escape.expect("symlink escape should prepare without expanding roots");

        for (label, roots, working_directory) in [
            (
                "registered workflow agent",
                &prepared.workspace_live_sync_roots,
                &instance_worktree,
            ),
            (
                "unregistered workflow agent",
                &unregistered.workspace_live_sync_roots,
                &unregistered_worktree,
            ),
            (
                "lexical traversal",
                &traversal.workspace_live_sync_roots,
                &traversal_worktree,
            ),
        ] {
            assert!(
                roots.iter().any(|root| root == working_directory),
                "{label} should retain its explicit working-directory policy"
            );
            assert!(
                !roots
                    .iter()
                    .any(|root| root == &primary || root == &supporting),
                "{label} must not inherit the Project workspace list"
            );
        }
        #[cfg(unix)]
        {
            assert!(symlink_escape
                .workspace_live_sync_roots
                .iter()
                .any(|root| root == &instance_worktree.join("outside-link")));
            assert!(!symlink_escape
                .workspace_live_sync_roots
                .iter()
                .any(|root| root == &primary || root == &supporting));
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn workflow_runtime_launch_with_chariox_home_uses_kernel_owned_instance_root() {
        crate::test_support::isolated_env_test!();
        let _env = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-workflow-runtime-home-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("workflow test home should exist");
        let previous_home = std::env::var_os("CHARIOX_HOME");
        let previous_user_home = std::env::var_os("HOME");
        std::env::set_var("CHARIOX_HOME", &root);
        std::env::set_var("HOME", root.join("unrelated-home"));

        let config = crate::config::DaemonConfig::load_from_env();
        let workflow_root = config
            .workflow_runtime_artifact_root()
            .join("instances")
            .join("workflow-session");
        let instance_worktree = workflow_root.join("instance-1");
        let unrelated = config.durable_state_path().with_file_name("unrelated");
        std::fs::create_dir_all(&instance_worktree).expect("workflow instance worktree");
        std::fs::create_dir_all(&unrelated).expect("unrelated state fixture");
        let source = root.join("source-workspace");
        std::fs::create_dir_all(&source).expect("source workspace");

        let mut app = crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap");
        let (session, source_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                source.to_string_lossy(),
                source.to_string_lossy(),
            ))
            .expect("session should create");
        let runtime_agent = app.agents().clone().materialize_workflow_runtime_agent(
            source_agent,
            session.id(),
            &instance_worktree.to_string_lossy(),
        );
        app.sessions_mut()
            .register_workflow_runtime_instance(
                session.id(),
                crate::session::WorkflowEndpointRuntimeInstance::new(
                    "instance-1",
                    "workflow-1",
                    "endpoint-1",
                    1,
                    1,
                    false,
                    std::collections::BTreeMap::from([(
                        "node-1".to_string(),
                        runtime_agent.id().to_string(),
                    )]),
                    instance_worktree.to_string_lossy(),
                ),
            )
            .expect("workflow runtime instance should register");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let prepared = runtime
            .owned
            .prepare_provider_launch_request(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "model",
                )
                .with_agent_id(runtime_agent.id()),
                "http://127.0.0.1:43120/mcp".to_string(),
            )
            .expect("kernel-owned workflow runtime cwd should prepare with CHARIOX_HOME");
        assert_eq!(
            prepared.working_directory.as_deref(),
            Some(instance_worktree.as_path())
        );

        let escalation = runtime.owned.prepare_provider_launch_request(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "model",
            )
            .with_agent_id(runtime_agent.id())
            .with_working_directory(unrelated.clone())
            .with_workspace_live_sync_roots(vec![unrelated.clone()]),
            "http://127.0.0.1:43120/mcp".to_string(),
        );
        assert!(
            escalation.is_err(),
            "a caller-supplied live-sync root must not authorize unrelated kernel state"
        );

        drop(runtime);
        drop(app);
        restore_env("CHARIOX_HOME", previous_home);
        restore_env("HOME", previous_user_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn owned_launch_preparation_allows_hidden_remote_lease_session_under_managed_isolation() {
        crate::test_support::isolated_env_test!();
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-hidden-provider-launch-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("hidden session workspace should exist");
        let app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap");
        let session = app
            .sessions_mut()
            .create_ephemeral_session(
                crate::session::CreateSessionRequest::new(
                    root.to_string_lossy(),
                    root.to_string_lossy(),
                )
                .with_hidden(true),
            )
            .expect("hidden remote lease session should create");
        assert!(session.project_id().is_empty());

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "model",
        );

        let _env = crate::env_lock::lock();
        let previous_isolation = std::env::var_os(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::set_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV, "1");
        let prepared = runtime
            .owned
            .prepare_provider_launch_request(request, "http://127.0.0.1:43120/mcp".to_string());
        restore_env(
            crate::provider::MANAGED_PROVIDER_ISOLATION_ENV,
            previous_isolation,
        );
        let prepared = prepared.expect("hidden provider launch should prepare");
        assert_eq!(prepared.working_directory.as_deref(), Some(root.as_path()));
        assert!(prepared.workspace_live_sync_roots.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn vaulted_claude_launch_waits_for_chariox_unlock_interaction() {
        crate::test_support::isolated_env_test!();
        let _env = crate::env_lock::lock();
        #[cfg(unix)]
        let _claude = crate::test_support::TestClaudeExecutable::new();
        let root = std::env::temp_dir().join(format!(
            "chariox-owned-vaulted-claude-launch-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test root should exist");
        std::env::set_var("CHARIOX_HOME", &root);
        let vault_path = root.join("credentials.vault");
        let mut config = crate::config::DaemonConfig::for_tests()
            .with_session_history_root(root.join("session-history"));
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.unlock_policy =
            crate::config::CredentialVaultUnlockPolicy::Always;
        config.user_config.credential_vault.path = vault_path.display().to_string();
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());

        let mut app = crate::app::DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .expect("session should create");
        let profile = app
            .provider_account_profile_registry()
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "claude",
                "Vaulted Claude",
            )
            .expect("managed Claude profile should create");
        let workflow_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "claude")
                    .with_model("claude-sonnet")
                    .with_account_profile(profile.profile_id.clone()),
            )
            .expect("workflow Claude agent should create");
        crate::secret::unlock_chariox_encrypted_vault(
            &vault_path,
            "correct horse battery staple",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .expect("vault should initialize");
        crate::provider::store_provider_account_credential(
            &config,
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
            "setup-token-secret",
            false,
        )
        .expect("provider credential should store");
        // This test covers unlock policy; the token was already verified.
        crate::test_support::authenticate_provider_account(
            &app.provider_account_profile_registry(),
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
        )
        .unwrap();
        crate::secret::lock_chariox_encrypted_vault(&vault_path).expect("vault should lock");
        crate::secret::clear_vault_secret_process_cache().expect("secret cache should clear");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let (workflow_provider_run_id, retired_provider_run_id) = runtime
            .owned
            .workflow_ensure_provider_run(session.id(), workflow_agent.id(), false, None)
            .expect("locked vault must not block synchronous workflow admission");
        assert!(retired_provider_run_id.is_none());
        assert!(runtime
            .owned
            .take_pending_provider_launch_credentials(&workflow_provider_run_id)
            .is_empty());
        let workflow_run = runtime
            .owned
            .provider_store
            .get_run(&workflow_provider_run_id)
            .expect("workflow provider run should exist");
        let mut workflow_credentials = Box::pin(
            runtime.resolve_provider_account_credentials_for_run_with_vault(
                &workflow_run,
                "test vaulted workflow Claude launch",
            ),
        );
        tokio::select! {
            result = &mut workflow_credentials => panic!("workflow credential resolution completed before unlock: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
        }
        let interaction = runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available")
            .active_interaction_for_agent(workflow_agent.id())
            .expect("workflow vault unlock interaction should be visible")
            .clone();
        resolve_vault_passphrase_interaction(&runtime, session.id(), &interaction).await;
        assert_eq!(
            workflow_credentials
                .await
                .expect("workflow credentials should resolve after unlock")
                .iter()
                .collect::<Vec<_>>(),
            vec![("CLAUDE_CODE_OAUTH_TOKEN", "setup-token-secret")]
        );

        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id());
        let mut preparation = Box::pin(
            runtime
                .prepare_provider_launch_request_with_vault(request, "test vaulted Claude launch"),
        );
        tokio::select! {
            result = &mut preparation => panic!("launch preparation resolved before unlock: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
        }
        let pending_session = runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available");
        let interaction = pending_session
            .active_interaction_for_agent(agent.id())
            .expect("vault unlock interaction should be visible")
            .clone();
        assert_eq!(interaction.title(), Some("Unlock Chariox Vault"));
        resolve_vault_passphrase_interaction(&runtime, session.id(), &interaction).await;
        let prepared = preparation
            .await
            .expect("launch should prepare after Chariox unlock");
        assert_eq!(
            prepared.provider_credential_env.iter().collect::<Vec<_>>(),
            vec![("CLAUDE_CODE_OAUTH_TOKEN", "setup-token-secret")]
        );
        assert!(
            !crate::secret::chariox_encrypted_vault_status(&vault_path)
                .expect("vault status should resolve")
                .unlocked
        );

        let mut remote_credential = Box::pin(runtime.resolve_remote_provider_launch_credential(
            session.id(),
            workflow_agent.id(),
            "test remote Claude launch",
        ));
        tokio::select! {
            result = &mut remote_credential => panic!("remote credential resolved before unlock: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
        }
        let interaction = runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available")
            .active_interaction_for_agent(workflow_agent.id())
            .expect("remote launch vault interaction should be visible")
            .clone();
        resolve_vault_passphrase_interaction(&runtime, session.id(), &interaction).await;
        let credential = remote_credential
            .await
            .expect("remote launch credential should resolve")
            .expect("Claude launch should carry a credential");
        assert_eq!(credential.provider, "claude");
        assert_eq!(credential.account_profile, profile.profile_id);
        assert!(!format!("{credential:?}").contains("setup-token-secret"));
        assert_eq!(
            credential.secret_input.into_zeroizing().as_str(),
            "setup-token-secret"
        );
        assert!(
            !crate::secret::chariox_encrypted_vault_status(&vault_path)
                .expect("vault status should resolve")
                .unlocked
        );

        std::env::remove_var("CHARIOX_HOME");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_continuation_retries_when_prompt_starts_during_vault_unlock() {
        crate::test_support::isolated_env_test!();
        let _env = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-mcp-continuation-vault-race-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test root should exist");
        let previous_home = std::env::var_os("CHARIOX_HOME");
        let previous_claude_bin = std::env::var_os("CHARIOX_CLAUDE_BIN");
        struct RestoreEnvironment(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
        impl Drop for RestoreEnvironment {
            fn drop(&mut self) {
                restore_env("CHARIOX_HOME", self.0.take());
                restore_env("CHARIOX_CLAUDE_BIN", self.1.take());
            }
        }
        let _restore = RestoreEnvironment(previous_home, previous_claude_bin);
        std::env::set_var("CHARIOX_HOME", &root);
        // MP-08/MP-10: forced catalog reload needs a live local CLI fixture.
        // Consume stdin without printing the synthetic credential environment.
        let claude_fixture = root.join("claude-fixture");
        std::fs::write(&claude_fixture, "#!/bin/sh\nexec cat >/dev/null\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&claude_fixture, std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        std::env::set_var("CHARIOX_CLAUDE_BIN", &claude_fixture);

        let vault_path = root.join("credentials.vault");
        let mut config = crate::config::DaemonConfig::for_tests()
            .with_session_history_root(root.join("session-history"));
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.unlock_policy =
            crate::config::CredentialVaultUnlockPolicy::Always;
        config.user_config.credential_vault.path = vault_path.display().to_string();
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());

        let _mcp = crate::test_support::TestRuntimeMcp::serve(&mut config);
        let mut app = crate::app::DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .expect("session should create");
        let profile = app
            .provider_account_profile_registry()
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "claude",
                "Vaulted continuation Claude",
            )
            .expect("managed Claude profile should create");
        // The continuation is a prompt submission, which requires an
        // authenticated account; a new managed profile starts as Unknown.
        crate::test_support::authenticate_provider_account(
            &app.provider_account_profile_registry(),
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
        )
        .expect("managed Claude profile should authenticate");
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "claude")
                    .with_model("claude-sonnet")
                    .with_account_profile(profile.profile_id.clone()),
            )
            .expect("Claude agent should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "mcp-continuation-vault-race-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("test client should attach");
        crate::secret::unlock_chariox_encrypted_vault(
            &vault_path,
            "correct horse battery staple",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .expect("vault should initialize");
        crate::provider::store_provider_account_credential(
            &config,
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
            "setup-token-secret",
            false,
        )
        .expect("provider credential should store");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            &profile.profile_id,
            "claude-sonnet",
        )
        .with_agent_id(agent.id());
        let mut preparation = Box::pin(
            runtime
                .prepare_provider_launch_request_with_vault(request, "prepare test provider run"),
        );
        tokio::select! {
            result = &mut preparation => panic!("Always policy must request approval even while unlocked: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
        }
        let initial_unlock = runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available")
            .active_interaction_for_agent(agent.id())
            .expect("initial launch should request vault approval")
            .clone();
        resolve_vault_passphrase_interaction(&runtime, session.id(), &initial_unlock).await;
        // Unlocking derives the vault key on the CPU, so its duration follows
        // machine load; use the liveness bound of the other waits in this test.
        let prepared = tokio::time::timeout(std::time::Duration::from_secs(45), preparation)
            .await
            .expect("approved initial launch should finish")
            .expect("initial launch should prepare after approval");
        let started = runtime
            .owned
            .provider_store
            .start_run_provider_only(prepared)
            .expect("test provider run should start");
        let running = runtime
            .owned
            .provider_store
            .mark_run_running(started.run().id())
            .expect("test provider run should become idle and running");
        runtime
            .owned
            .provider_run_projection
            .update(running.clone());
        crate::secret::lock_chariox_encrypted_vault(&vault_path).expect("vault should lock");
        crate::secret::clear_vault_secret_process_cache().expect("secret cache should clear");

        runtime.remember_pending_mcp_continuation(
            session.id(),
            agent.id(),
            "playwright",
            "continue after granting playwright",
        );
        let first_unlock = tokio::time::timeout(std::time::Duration::from_secs(45), async {
            loop {
                if let Some(interaction) = runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("session should remain available")
                    .active_interaction_for_agent(agent.id())
                {
                    break interaction.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first vault unlock interaction should appear");
        let active_prompt = crate::session::PromptQueueItem::new(
            "prompt-started-during-unlock",
            attachment.id(),
            agent.id(),
            "new prompt",
            crate::session::PromptStatus::Running,
        )
        .with_prompt_origin(crate::session::PromptOrigin::External);
        app.lock()
            .await
            .prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(active_prompt))
            .expect("active prompt should start during unlock");
        resolve_vault_passphrase_interaction(&runtime, session.id(), &first_unlock).await;

        tokio::time::timeout(std::time::Duration::from_secs(45), async {
            loop {
                if runtime
                    .owned
                    .pending_mcp_continuations
                    .write()
                    .contains_key(agent.id())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("deferred MCP continuation should remain pending");
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run_for_agent(session.id(), agent.id())
                .expect("provider run should remain available")
                .id(),
            running.id()
        );

        app.lock()
            .await
            .prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
            .expect("agent should become idle");
        let second_unlock = tokio::time::timeout(std::time::Duration::from_secs(45), async {
            loop {
                if let Some(interaction) = runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("session should remain available")
                    .active_interaction_for_agent(agent.id())
                {
                    if interaction.title() == Some("Unlock Chariox Vault") {
                        break interaction.clone();
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("idle retry should request vault unlock again");
        resolve_vault_passphrase_interaction(&runtime, session.id(), &second_unlock).await;
        // MP-08/MP-10: forced catalog activation uses the existing 12s policy
        // relaunch delay. Match its bounded 30s readiness gate, then retain the
        // separate continuation-delivery assertion below.
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                if runtime
                    .owned
                    .provider_store
                    .get_run_for_agent(session.id(), agent.id())
                    .is_some_and(|run| {
                        run.id() != running.id()
                            && run.state() == crate::provider::ProviderRunState::Running
                    })
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("forced catalog reload should produce a new running provider");
        tokio::time::timeout(std::time::Duration::from_secs(45), async {
            loop {
                let resumed = runtime
                    .owned
                    .operational_history_store
                    .query_events(crate::history::HistoryEventQuery {
                        session_id: Some(session.id().to_string()),
                        agent_id: Some(agent.id().to_string()),
                        text: Some("continue after granting playwright".to_string()),
                        limit: Some(10),
                        ..crate::history::HistoryEventQuery::default()
                    })
                    .expect("continuation history should remain queryable")
                    .into_iter()
                    .any(|event| {
                        event
                            .prompt_id
                            .as_deref()
                            .is_some_and(|prompt_id| prompt_id.starts_with("prompt-"))
                    });
                if resumed {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|error| {
            let session = runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap();
            let run = runtime
                .owned
                .provider_store
                .get_run_for_agent(session.id(), agent.id());
            let pending = runtime
                .owned
                .pending_mcp_continuations
                .write()
                .contains_key(agent.id());
            panic!(
                "MCP continuation should resume after the agent becomes idle: {error}; run={:?}, interaction={:?}, pending={}, run states={:?}; notices={:?}",
                run.map(|r| r.state()),
                session
                    .active_interaction_for_agent(agent.id())
                    .map(|i| i.title()),
                pending,
                runtime
                    .owned
                    .provider_store
                    .list_runs()
                    .iter()
                    .map(|run| (run.id().to_string(), run.state()))
                    .collect::<Vec<_>>(),
                runtime.owned.terminal_stream.notice_records()
            );
        });

        if let Some(run) = runtime
            .owned
            .provider_store
            .get_run_for_agent(session.id(), agent.id())
        {
            runtime
                .owned
                .provider_store
                .terminate_run_provider_only(session.id(), run.id())
                .expect("stop fixture-owned Claude process");
        }
        let _ = crate::secret::lock_chariox_encrypted_vault(&vault_path);
        let _ = crate::secret::clear_vault_secret_process_cache();
        let _ = std::fs::remove_dir_all(root);
    }

    fn restore_env(name: &str, previous: Option<std::ffi::OsString>) {
        match previous {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    async fn resolve_vault_passphrase_interaction(
        runtime: &KernelRuntimeState,
        session_id: &str,
        passphrase_interaction: &crate::session::RuntimeInteraction,
    ) {
        runtime
            .resolve_terminal_runtime_interaction(
                session_id,
                passphrase_interaction.id(),
                "passphrase",
                Some("correct horse battery staple"),
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .expect("vault passphrase interaction should resolve");
    }

    mod leased_account_authority_tests;

    #[tokio::test]
    async fn leased_launch_keeps_the_home_replica_namespace_on_a_same_owner_worker() {
        crate::test_support::isolated_env_test!();
        let root = std::env::temp_dir().join(format!(
            "chariox-leased-account-launch-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let worktree = root.join("workspace");
        std::fs::create_dir_all(&worktree).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "shared-cloud-owner".into(),
            ..Default::default()
        });
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let registry = app.provider_account_profile_registry();
        let profile = registry
            .create_managed("shared-cloud-owner", "codex", "Home replica fixture")
            .unwrap();
        let environment = registry
            .resolve_environment("shared-cloud-owner", "codex", &profile.profile_id)
            .unwrap();
        std::fs::write(
            Path::new(&environment["CODEX_HOME"]).join("auth.json"),
            br#"{"tokens":{"access_token":"test"}}"#,
        )
        .unwrap();
        let local_profile = registry
            .create_managed("local", "codex", "Worker account fixture")
            .unwrap();
        let local_environment = registry
            .resolve_environment("local", "codex", &local_profile.profile_id)
            .unwrap();
        std::fs::write(
            Path::new(&local_environment["CODEX_HOME"]).join("auth.json"),
            br#"{"tokens":{"access_token":"test"}}"#,
        )
        .unwrap();
        registry
            .update_observation(
                "shared-cloud-owner",
                "codex",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let mut worker = crate::app::RemoteLeaseRuntime::new(&mut app);
        let lease = worker
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "shared-cloud-owner",
            )
            .unwrap();
        let backing = worker
            .create_leased_agent(
                &lease.id,
                "codex",
                &profile.profile_id,
                Some("gpt-6.1-sol".into()),
                None,
                None,
                None,
                None,
                Some(worktree.to_string_lossy().into_owned()),
                None,
            )
            .unwrap();
        let backing_agent = app.agents().get_agent(&backing.backing_agent_id).unwrap();
        app.provider_account_profile_registry()
            .require_agent_authenticated(
                &app.provider_account_authority_owner_for_agent(&backing_agent)
                    .unwrap(),
                &backing_agent,
                "leased prompt admission",
            )
            .expect(
                "leased prompt admission must validate the home replica, not the worker account",
            );
        let activated = app
            .prepare_app_provider_launch_request(
                crate::provider::LaunchProviderRequest::new(
                    &backing.backing_session_id,
                    "codex",
                    "codex",
                    &profile.profile_id,
                    "gpt-6.1-sol",
                )
                .with_agent_id(&backing.backing_agent_id)
                .with_client_interface(crate::provider::ProviderClientInterface::NativeTui),
                "leased compatibility activation",
            )
            .expect("compatibility activation also retains the replica namespace");
        assert_eq!(activated.provider_account_env, environment);
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            &backing.backing_session_id,
            "codex",
            "codex",
            &profile.profile_id,
            "gpt-6.1-sol",
        )
        .with_agent_id(&backing.backing_agent_id);
        let mut local_request = request.clone();
        local_request.account_profile = local_profile.profile_id;
        let home_prepared = runtime
            .owned
            .prepare_provider_launch_request(
                local_request.clone(),
                "http://127.0.0.1:43120/mcp".into(),
            )
            .expect("home launches still alias the Cloud owner to local accounts");
        assert_eq!(home_prepared.provider_account_env, local_environment);
        assert!(
            runtime
                .owned
                .prepare_leased_provider_launch_request(
                    local_request,
                    "http://127.0.0.1:43120/mcp".into()
                )
                .is_err(),
            "a missing replica must never fall back to the worker's own account"
        );
        let prepared = runtime.owned.prepare_leased_provider_launch_request(
            request, "http://127.0.0.1:43120/mcp".into()
        ).expect("the leased launch must resolve the home-owned replica, not the worker local namespace");
        assert_eq!(prepared.owner_user_id, "shared-cloud-owner");
        assert_eq!(prepared.provider_account_env, environment);
        let mut run = crate::provider::RuntimeProviderRun::new(
            "leased-account-fixture",
            &prepared,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "account scope fixture".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        runtime.owned.provider_run_projection.update(run.clone());
        runtime
            .owned
            .provider_run_projection
            .mark_leased_provider_run(run.id());
        assert!(
            runtime.owned.provider_account_allows_queued_prompt_advance(
                &backing.backing_session_id,
                &backing_agent,
                "promote leased startup prompt",
            ),
            "a prompt queued during leased startup must retain the replica account scope"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn session_snapshot_remains_responsive_during_provider_launch_authority_lock() {
        crate::test_support::isolated_env_test!();
        let workspace =
            std::env::temp_dir().join(format!("snapshot-launch-lock-{}", rand::random::<u64>()));
        std::fs::create_dir(&workspace).expect("workspace");
        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                workspace.to_string_lossy(),
                workspace.to_string_lossy(),
            ))
            .expect("session");
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "codex",
            "codex",
            "default",
            "codex/gpt-6.1-sol",
        )
        .with_agent_id(agent.id());
        let run = crate::provider::RuntimeProviderRun::new(
            "launching-run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "snapshot-launch-regression".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        runtime.owned.provider_run_projection.update(run.clone());
        let store = runtime.owned.provider_store.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _authority = store.write();
            ready_tx.send(()).expect("announce held launch authority");
            let _ = release_rx.recv_timeout(std::time::Duration::from_secs(1));
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("authority held");
        let started = std::time::Instant::now();
        let snapshot = runtime
            .owned
            .session_snapshot(session.id())
            .expect("snapshot");
        let elapsed = started.elapsed();
        let _ = release_tx.send(());
        holder.join().expect("authority released");
        std::fs::remove_dir(&workspace).expect("remove test workspace");
        assert_eq!(snapshot.active_provider_run_id(), Some("launching-run"));
        assert!(elapsed < std::time::Duration::from_millis(150),
            "presentation must use the existing projection while launch authority is held: {elapsed:?}");
        // Once authority is available, its newer lifecycle state wins over the
        // cached Starting projection.
        let mut current = run;
        current.mark_parked();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(current);
        assert_eq!(
            runtime
                .owned
                .session_snapshot(session.id())
                .unwrap()
                .active_provider_run_id(),
            None
        );
    }
}
