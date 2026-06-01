use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn launch_provider_request_from_local_request(
        &self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> crate::provider::LaunchProviderRequest {
        let mut launch_request = crate::provider::LaunchProviderRequest::new(
            request.session_id.clone(),
            request.adapter_key,
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
            if launch_request.adapter_key == "codex" {
                launch_request = launch_request.with_resume_state(
                    crate::provider::ProviderResumeState::from_codex_thread_id(provider_session_id),
                );
            } else if launch_request.adapter_key == "opencode" {
                launch_request = launch_request.with_resume_state(
                    crate::provider::ProviderResumeState::from_opencode_session_id(
                        provider_session_id,
                    ),
                );
            } else if launch_request.adapter_key == "claude" {
                launch_request = launch_request.with_resume_state(
                    crate::provider::ProviderResumeState::from_claude_session_id(
                        provider_session_id,
                    ),
                );
            }
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
        mut request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let session = self.session_store.get_session(&request.session_id)?;
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
            if agent.remote_execution().is_some() {
                return Err(DaemonError::LocalTransport {
                    operation: "launch provider run",
                    message: format!(
                        "agent `{}` is remote-backed and must launch its provider on the worker kernel",
                        agent.id()
                    ),
                });
            }
        }
        let effective_config =
            crate::session::effective_agent_execution_config(&session, agent.as_ref());
        if request.execution_mode.is_none() {
            request = request.with_execution_mode(effective_config.mode);
        }
        if request.permission_level.is_none() {
            request = request.with_permission_level(effective_config.permission_level);
        }
        if request.resume_state.is_none() {
            if let Some(agent) = agent.as_ref() {
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
            let config = self.config_projection.snapshot();
            let workspace_live_sync_roots = workspace_live_sync_protected_roots(
                &session,
                request.working_directory.as_deref(),
                &config.host_machine_id,
                &config.daemon_id,
            );
            request = request.with_workspace_live_sync_roots(workspace_live_sync_roots);
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
            let credential_env_names = crate::credential::load_user_credentials()
                .map(|credentials| {
                    crate::secret::RuntimeSecretService::credential_env_names_from(&credentials)
                })
                .unwrap_or_default();
            request = request.with_provider_env_remove(credential_env_names.into_iter().collect());
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
        let config = self.config_projection.snapshot();
        let mcp_servers = std::mem::take(&mut request.mcp_servers);
        request = request.with_mcp_servers(crate::app::resolve_mcp_credentials_for_launch(
            &config,
            mcp_servers,
        )?);
        Ok(request)
    }
}

fn workspace_live_sync_protected_roots(
    session: &crate::session::RuntimeSession,
    working_directory: Option<&std::path::Path>,
    host_machine_id: &str,
    host_daemon_id: &str,
) -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = working_directory
        .and_then(resolve_git_root)
        .or_else(|| working_directory.map(std::path::PathBuf::from))
    {
        push_unique_root(&mut roots, root);
    }
    for link in session.workspace_links() {
        for attachment in link.attachments() {
            if attachment.machine_id() == host_machine_id
                && attachment.kernel_id() == host_daemon_id
            {
                push_unique_root(&mut roots, std::path::PathBuf::from(attachment.repo_root()));
            }
        }
    }
    roots
}

fn resolve_git_root(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    let root = root.trim();
    (!root.is_empty()).then(|| std::path::PathBuf::from(root))
}

fn push_unique_root(roots: &mut Vec<std::path::PathBuf>, root: std::path::PathBuf) {
    if !roots.iter().any(|existing| existing == &root) {
        roots.push(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_live_sync_protected_roots_include_working_directory_and_local_links() {
        let mut session = crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "/repo/main",
            "machine-1",
            "daemon-1",
        );
        session.create_workspace_link(crate::session::WorkspaceLinkDefinition::new(
            "link-1",
            "session-1",
            "shared",
            "local",
        ));
        session
            .workspace_link_mut("link-1")
            .expect("link should exist")
            .attach(crate::session::WorkspaceLinkAttachment::new(
                "link-1",
                "local",
                "machine-1",
                "daemon-1",
                "/repo/attached",
                None,
                None,
            ));
        session
            .workspace_link_mut("link-1")
            .expect("link should exist")
            .attach(crate::session::WorkspaceLinkAttachment::new(
                "link-1",
                "peer",
                "remote-machine",
                "remote-daemon",
                "/remote/repo",
                None,
                None,
            ));

        let roots = workspace_live_sync_protected_roots(
            &session,
            Some(std::path::Path::new("/repo/main")),
            "machine-1",
            "daemon-1",
        );

        assert_eq!(
            roots,
            vec![
                std::path::PathBuf::from("/repo/main"),
                std::path::PathBuf::from("/repo/attached"),
            ]
        );
    }

    #[test]
    fn workspace_live_sync_protected_roots_do_not_include_sibling_repos() {
        let base = std::env::temp_dir().join(format!(
            "arroba-live-sync-root-scope-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let selected = base.join("selected");
        let selected_child = selected.join("src");
        let sibling = base.join("sibling");
        std::fs::create_dir_all(&selected_child).expect("selected repo fixture should exist");
        std::fs::create_dir_all(&sibling).expect("sibling repo fixture should exist");
        run_git_init(&selected);
        run_git_init(&sibling);
        let session = crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            selected_child.to_string_lossy().to_string(),
            "machine-1",
            "daemon-1",
        );

        let roots = workspace_live_sync_protected_roots(
            &session,
            Some(selected_child.as_path()),
            "machine-1",
            "daemon-1",
        );

        let canonical_selected = selected
            .canonicalize()
            .expect("selected repo should canonicalize");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(roots, vec![canonical_selected]);
    }

    fn run_git_init(path: &std::path::Path) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("init")
            .arg("-b")
            .arg("main")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git init should run");
        assert!(
            status.success(),
            "git init should succeed in {}",
            path.display()
        );
    }
}
