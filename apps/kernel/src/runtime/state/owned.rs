use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let mut session = self.session_store.get_session(session_id)?;
        let agents = self.agent_store.get_session_agents(session_id);
        session.set_agents(agents);
        self.project_session_runtime_view(&mut session);
        self.session_projection.update(session.clone());
        Ok(session)
    }

    pub(super) fn project_session_runtime_view(
        &self,
        session: &mut crate::session::RuntimeSession,
    ) {
        if let Some(active_provider_run_id) = session.active_provider_run_id() {
            if let Ok(active_run) = self.provider_store.get_run(active_provider_run_id) {
                let active_run_agent_id = active_run.agent_instance_id();
                let active_prompt_is_running = active_run_agent_id
                    .and_then(|agent_id| {
                        self.prompt_state_owner
                            .active_prompt_for_agent_snapshot(session, agent_id)
                    })
                    .is_some();
                if active_run.state() == crate::provider::ProviderRunState::Running
                    && active_prompt_is_running
                {
                    return;
                }
            }
        }

        let projected_run_id = session.focused_agent_id().and_then(|agent_id| {
            self.provider_store
                .get_run_for_agent(session.id(), agent_id)
                .map(|run| run.id().to_string())
        });
        session.set_active_provider_run(projected_run_id);
    }

    pub(super) fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let attachment = self.attachment_store.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        Ok(attachment)
    }

    pub(super) fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        if !matches!(
            attachment.capability_level(),
            crate::attachment::ClientCapabilityLevel::FullTerminal
                | crate::attachment::ClientCapabilityLevel::InteractiveStructured
        ) {
            return Err(DaemonError::AttachmentCapabilityDenied {
                session_id: session_id.to_string(),
                attachment_id: attachment.id().to_string(),
                capability,
            });
        }
        Ok(CapabilityRuntimeSnapshot {
            workspace_id: session.workspace_id().to_string(),
            worktree_root: std::path::PathBuf::from(session.worktree_id()),
            workspace_coordinator: self.workspace_coordinator.clone(),
        })
    }

    pub(super) fn managed_io_domain_from_arg(
        domain: Option<&str>,
    ) -> Result<crate::io::ArtifactDomainKind, DaemonError> {
        match domain.unwrap_or("text") {
            "text" => Ok(crate::io::ArtifactDomainKind::TextDocument),
            "structured" => Ok(crate::io::ArtifactDomainKind::StructuredDocument),
            "opaque" => Ok(crate::io::ArtifactDomainKind::OpaqueBlob),
            other => Err(DaemonError::LocalTransport {
                operation: "runtime_tool_managed_io",
                message: format!("unsupported artifact domain `{other}`"),
            }),
        }
    }

    pub(super) fn prepare_provider_launch_request(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
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
        if request.resume_state.is_none() {
            if let Some(agent_id) = request.agent_id.as_deref() {
                if let Ok(agent) = self.agent_store.get_agent(agent_id) {
                    let resume_state =
                        crate::app::sanitize_resume_state_for_launch(&request, &agent);
                    if !resume_state.is_empty() {
                        request = request.with_resume_state(resume_state);
                    }
                }
            }
        }
        if (request.adapter_key == "opencode" || request.adapter_key == "codex")
            && request.working_directory.is_none()
        {
            let agent_worktree = request.agent_id.as_deref().and_then(|agent_id| {
                self.agent_store
                    .get_agent(agent_id)
                    .ok()
                    .and_then(|agent| agent.worktree_id().map(std::path::PathBuf::from))
            });
            request.working_directory = Some(agent_worktree.unwrap_or_else(|| {
                std::path::PathBuf::from(
                    self.session_store
                        .get_session(&request.session_id)
                        .map(|session| session.worktree_id().to_string())
                        .unwrap_or_default(),
                )
            }));
        }
        if request.runtime_mcp_binding.is_none() {
            let shared_auth_token = self
                .provider_store
                .get_session_run_for_provider(&request.session_id, &request.provider)
                .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string));
            request = request.with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                runtime_mcp_url,
                shared_auth_token.unwrap_or_else(crate::app::generate_runtime_mcp_auth_token),
            ));
        }
        Ok(request)
    }

    pub(super) fn create_session_response(
        &self,
        request: crate::session::CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut session =
            SessionStateOwner::new(self.session_store.clone()).create_session(request)?;
        let agent_request = crate::agent::CreateAgentRequest::new(session.id(), "default")
            .with_worktree(session.worktree_id());
        let mut sessions = self.session_store.write();
        let agent = self
            .agent_store
            .create_agent(agent_request, &mut sessions)?;
        drop(sessions);
        session = self.session_store.get_session(session.id())?;
        let agents = self.agent_store.get_session_agents(session.id());
        session.set_agents(agents);
        self.project_session_runtime_view(&mut session);
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::SessionCreated { session, agent })
    }

    pub(super) fn update_session_config(
        &self,
        session_id: &str,
        attachment_id: &str,
        values: std::collections::BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<crate::session::SessionConfigState, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        let (_session, config) = SessionStateOwner::new(self.session_store.clone()).update_config(
            session_id,
            attachment_id,
            values,
            requires_idle,
        )?;
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|id| id != attachment_id)
            .collect::<Vec<_>>();
        if !recipient_attachment_ids.is_empty() {
            let active_provider_run_id = self
                .session_store
                .get_session(session_id)?
                .active_provider_run_id()
                .map(str::to_string);
            self.record_notice(
                session_id,
                active_provider_run_id.as_deref(),
                recipient_attachment_ids,
                format!(
                    "Attachment `{attachment_id}` updated configuration for session `{session_id}`."
                ),
            );
        }
        Ok(config)
    }

    pub(super) fn alias_session(
        &self,
        session_id: &str,
        alias: String,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        SessionStateOwner::new(self.session_store.clone())
            .assign_session_alias(session_id, alias)?;
        self.session_snapshot(session_id)
    }

    pub(super) fn spawn_agent(
        &self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let mut sessions = self.session_store.write();
        self.agent_store.create_agent(request, &mut sessions)
    }

    pub(super) fn destroy_agent(
        &self,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        let session_id = agent.session_id().to_string();
        let provider_run_ids = self
            .provider_store
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == session_id
                    && run.agent_instance_id() == Some(agent_id)
                    && run.state() != crate::provider::ProviderRunState::Ended
            })
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        for provider_run_id in provider_run_ids {
            let ended = self
                .provider_store
                .terminate_run_provider_only(&session_id, &provider_run_id)?
                .into_run();
            if self
                .session_store
                .get_session(&session_id)?
                .active_provider_run_id()
                == Some(ended.id())
            {
                self.session_store
                    .set_active_provider_run(&session_id, None)?;
            }
            self.provider_run_projection.update(ended.clone());
            self.clear_prompt_activity(ended.id());
            self.remove_provider_process_tracking_for_run(ended.id(), None);
        }
        self.prompt_state_owner.remove_agent(&session_id, agent_id);
        self.session_store.mirror_agent_prompt_state(
            &session_id,
            agent_id,
            None,
            std::collections::VecDeque::new(),
        )?;
        let mut sessions = self.session_store.write();
        self.agent_store.destroy_agent(agent_id, &mut sessions)
    }

    pub(super) fn attach(
        &self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let session_id = request.session_id.clone();
        let client_id = request.client_id.clone();
        let capability_level = format!("{:?}", request.capability_level);
        let replaced_attachment_ids = self
            .attachment_store
            .list_client_attachments(&client_id)
            .into_iter()
            .map(|attachment| attachment.id().to_string())
            .collect::<Vec<_>>();
        for attachment_id in &replaced_attachment_ids {
            let _ = self.detach(attachment_id)?;
        }

        let mut sessions = self.session_store.write();
        let attachment = self.attachment_store.attach(&mut sessions, request)?;
        drop(sessions);

        if self.agent_store.get_session_agents(&session_id).is_empty() {
            let worktree_id = self
                .session_store
                .get_session(&session_id)?
                .worktree_id()
                .to_string();
            let agent_request = crate::agent::CreateAgentRequest::new(&session_id, "default")
                .with_worktree(worktree_id);
            let mut sessions = self.session_store.write();
            let _ = self
                .agent_store
                .create_agent(agent_request, &mut sessions)?;
            drop(sessions);
            crate::logging::info_with_fields(
                "daemon.app",
                "created default agent for session",
                serde_json::json!({
                    "session_id": session_id,
                    "reason": "session had no agents (possibly after being ended and reattached)",
                }),
            );
        }

        self.sync_focused_provider_run_if_idle(&session_id)?;

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment joined session",
            serde_json::json!({
                "session_id": session_id,
                "attachment_id": attachment.id(),
                "client_id": client_id,
                "capability_level": capability_level,
                "replaced_attachment_ids": replaced_attachment_ids,
            }),
        );
        Ok(attachment)
    }

    pub(super) fn detach(
        &self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let mut sessions = self.session_store.write();
        let (attachment, effect) = self
            .attachment_store
            .detach_with_effect(&mut sessions, attachment_id)?;
        drop(sessions);

        let session = self.session_store.get_session(attachment.session_id())?;
        let owner_removed_queued_prompt_count = self
            .prompt_state_owner
            .remove_queued_prompts_by_attachment(&session, attachment_id);
        self.mirror_prompt_owner_session_state(attachment.session_id())?;
        let removed_queued_prompt_count = effect
            .removed_queued_prompt_count
            .max(owner_removed_queued_prompt_count);
        let session_after_detach = self.session_store.get_session(attachment.session_id())?;

        if removed_queued_prompt_count > 0 {
            self.record_notice(
                attachment.session_id(),
                None,
                self.attachment_store
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed {} queued prompt(s) from detached attachment `{}`.",
                    removed_queued_prompt_count, attachment_id
                ),
            );
        }

        if effect.removed_active_prompt {
            self.record_notice(
                attachment.session_id(),
                None,
                self.attachment_store
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed the active prompt from detached attachment `{}` and advanced the queue.",
                    attachment_id
                ),
            );
            if let Some(agent_id) = session_after_detach.focused_agent_id() {
                let _ = self.activate_next_queued_prompt_for_agent(
                    attachment.session_id(),
                    agent_id,
                    None,
                )?;
            }
        }

        let remaining_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(attachment.session_id());
        let active_prompt_agent_id = self
            .prompt_state_owner
            .active_prompt_agent_id(&self.session_snapshot(attachment.session_id())?);
        if remaining_attachment_ids.is_empty() && active_prompt_agent_id.is_none() {
            if let Some(active_provider_run_id) = session_after_detach
                .active_provider_run_id()
                .map(str::to_string)
            {
                let run = self.provider_store.get_run(&active_provider_run_id)?;
                if run.state() != crate::provider::ProviderRunState::Ended {
                    let outcome = self
                        .provider_store
                        .park_run_provider_only(attachment.session_id(), &active_provider_run_id)?;
                    if self
                        .session_store
                        .get_session(attachment.session_id())?
                        .active_provider_run_id()
                        == Some(outcome.run().id())
                    {
                        self.session_store
                            .set_active_provider_run(attachment.session_id(), None)?;
                    }
                    self.provider_run_projection.update(outcome.into_run());
                }
            }
            for run in self.provider_store.list_runs() {
                if run.session_id() == attachment.session_id() {
                    self.clear_prompt_activity(run.id());
                }
            }
        }

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment left session",
            serde_json::json!({
                "session_id": attachment.session_id(),
                "attachment_id": attachment.id(),
                "removed_queued_prompts": removed_queued_prompt_count,
                "removed_active_prompt": effect.removed_active_prompt,
                "remaining_attachment_ids": remaining_attachment_ids,
            }),
        );
        self.session_snapshot(attachment.session_id())?;

        Ok(attachment)
    }

    pub(super) fn focus_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let mut sessions = self.session_store.write();
        let agent = self
            .agent_store
            .focus_agent(session_id, agent_id, &mut sessions)?;
        drop(sessions);
        if !self.should_defer_provider_run_sync_for_focus_change(session_id, agent_id)? {
            self.sync_active_provider_run_for_agent(session_id, agent_id)?;
        }
        Ok(agent)
    }

    pub(super) fn cycle_agent_focus(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        let mut sessions = self.session_store.write();
        let agent = self.agent_store.cycle_focus(session_id, &mut sessions)?;
        drop(sessions);
        if let Some(focused) = agent.as_ref() {
            if !self.should_defer_provider_run_sync_for_focus_change(session_id, focused.id())? {
                self.sync_active_provider_run_for_agent(session_id, focused.id())?;
            }
        }
        Ok(agent)
    }

    pub(super) fn resize_terminal(&self, session_id: &str) -> Result<Option<String>, DaemonError> {
        let provider_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        let _ = self.reconcile_provider_run_liveness_provider_phase(
            session_id,
            &provider_run_id,
            None,
        )?;
        let provider_run = self.ensure_provider_run_in_session(session_id, &provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id,
                state: provider_run.state(),
                operation: "resize terminal",
            });
        }
        if provider_run.endpoint_mode() == crate::provider::AgentEndpointMode::External {
            return Ok(None);
        }
        Ok(Some(provider_run.id().to_string()))
    }

    pub(super) fn end_session(
        &self,
        session_id: &str,
    ) -> Result<(crate::session::RuntimeSession, Vec<String>), DaemonError> {
        let session = self.session_store.get_session(session_id)?;

        if session.status() == crate::session::SessionStatus::Ended {
            self.prompt_state_owner.remove_session(session_id);
            let ended = self.session_store.end_session(session_id)?;
            return Ok((ended, Vec::new()));
        }

        let removed_attachments = self.attachment_store.remove_session_attachments(session_id);
        let terminated_runs = self
            .provider_store
            .terminate_session_runs_provider_only(session_id)?;
        let terminated_run_ids = terminated_runs
            .runs()
            .iter()
            .map(|outcome| outcome.run().id().to_string())
            .collect::<Vec<_>>();
        for outcome in terminated_runs.into_runs() {
            if self
                .session_store
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(outcome.run().id())
            {
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            self.provider_run_projection.update(outcome.into_run());
        }

        let removed_agents = self.agent_store.remove_session_agents(session_id);
        let removed_agent_ids: Vec<_> = removed_agents
            .iter()
            .map(|agent| format!("{} ({})", agent.agent_ref(), agent.id()))
            .collect();

        for run in self.provider_store.list_runs() {
            if run.session_id() == session_id {
                self.clear_prompt_activity(run.id());
            }
        }
        self.prompt_state_owner.remove_session(session_id);
        let mut ended = self.session_store.end_session(session_id)?;
        ended.set_agents(removed_agents);
        crate::logging::info_with_fields(
            "daemon.session",
            "session ended",
            serde_json::json!({
                "session_id": session_id,
                "removed_attachment_ids": removed_attachments
                    .iter()
                    .map(|attachment| attachment.id().to_string())
                    .collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "removed_agents": removed_agent_ids,
            }),
        );
        Ok((ended, terminated_run_ids))
    }

    pub(super) fn delete_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<(crate::session::RuntimeSession, Vec<String>), DaemonError> {
        let session = self
            .session_store
            .read()
            .resolve_session_ref(session_ref, workspace_id)?;
        let session_id = session.id().to_string();
        let (ended, terminated_run_ids) = self.end_session(&session_id)?;
        let mut deleted = self.session_store.delete_session(ended.id())?;
        deleted.set_agents(ended.agents().to_vec());
        self.history_projection.remove(deleted.id());
        self.session_projection.remove(deleted.id());
        crate::logging::info_with_fields(
            "daemon.session",
            "session deleted",
            serde_json::json!({
                "session_id": deleted.id(),
                "session_alias": deleted.alias(),
            }),
        );
        Ok((deleted, terminated_run_ids))
    }

    pub(super) fn should_defer_provider_run_sync_for_focus_change(
        &self,
        session_id: &str,
        target_agent_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.session_snapshot(session_id)?;
        let Some(active_provider_run_id) = session.active_provider_run_id().map(str::to_string)
        else {
            return Ok(false);
        };
        let active_run = self.provider_store.get_run(&active_provider_run_id)?;
        if active_run.agent_instance_id() == Some(target_agent_id)
            || active_run.state() != crate::provider::ProviderRunState::Running
        {
            return Ok(false);
        }

        Ok(self
            .prompt_state_owner
            .active_prompt_agent_id(&session)
            .is_some()
            || session.agents().iter().any(|agent| agent.is_processing()))
    }

    pub(super) fn sync_active_provider_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let current_active_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);

        if let Some(current_active_run_id) = current_active_run_id.as_deref() {
            let active_run = self.provider_store.get_run(current_active_run_id)?;
            if active_run.agent_instance_id() != Some(agent_id)
                && active_run.state() == crate::provider::ProviderRunState::Running
            {
                let outcome = self
                    .provider_store
                    .park_run_provider_only(session_id, current_active_run_id)?;
                self.clear_active_provider_run_session_pointer(session_id, outcome.run().id())?;
                self.provider_run_projection.update(outcome.into_run());
            }
        }

        if let Some(agent_run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
            match agent_run.state() {
                crate::provider::ProviderRunState::Running
                | crate::provider::ProviderRunState::Starting => {
                    self.session_store
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                crate::provider::ProviderRunState::Parked => {
                    let _ = self.resume_provider_run_for_session(session_id, agent_run.id())?;
                }
                crate::provider::ProviderRunState::Ended => {
                    self.session_store
                        .set_active_provider_run(session_id, None)?;
                }
            }
        } else {
            self.session_store
                .set_active_provider_run(session_id, None)?;
        }

        Ok(())
    }

    pub(super) fn sync_focused_provider_run_if_idle(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.session_snapshot(session_id)?;
        if session.agents().len() > 1 {
            let focused_agent_id = session.focused_agent_id().map(str::to_string);
            if let Some(focused_agent_id) = focused_agent_id {
                if self
                    .prompt_state_owner
                    .active_prompt_agent_id(&session)
                    .is_none()
                {
                    let current_active_run_id =
                        session.active_provider_run_id().map(str::to_string);
                    if let Some(current_active_run_id) = current_active_run_id.as_deref() {
                        let active_run = self.provider_store.get_run(current_active_run_id)?;
                        if active_run.agent_instance_id() != Some(focused_agent_id.as_str())
                            && active_run.state() == crate::provider::ProviderRunState::Running
                        {
                            let outcome = self
                                .provider_store
                                .park_run_provider_only(session_id, current_active_run_id)?;
                            self.clear_active_provider_run_session_pointer(
                                session_id,
                                outcome.run().id(),
                            )?;
                            self.provider_run_projection.update(outcome.into_run());
                        }
                    }
                }
                self.project_active_provider_run_for_agent(session_id, &focused_agent_id)?;
            } else {
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            return Ok(());
        }

        if self
            .prompt_state_owner
            .active_prompt_agent_id(&session)
            .is_some()
            || session.agents().iter().any(|agent| agent.is_processing())
        {
            return Ok(());
        }

        if let Some(focused_agent_id) = session.focused_agent_id() {
            self.sync_active_provider_run_for_agent(session_id, focused_agent_id)?;
        } else {
            self.session_store
                .set_active_provider_run(session_id, None)?;
        }
        Ok(())
    }

    pub(super) fn project_active_provider_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let projected_run_id = self
            .provider_store
            .get_run_for_agent(session_id, agent_id)
            .map(|run| run.id().to_string());
        self.session_store
            .set_active_provider_run(session_id, projected_run_id)?;
        Ok(())
    }

    pub(super) fn mirror_prompt_owner_session_state(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let mut agent_ids = self
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        let session = self.session_store.get_session(session_id)?;
        agent_ids.extend(session.prompt_states().keys().cloned());
        agent_ids.sort();
        agent_ids.dedup();
        for agent_id in agent_ids {
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, &agent_id);
            self.session_store.mirror_agent_prompt_state(
                session_id,
                &agent_id,
                active_prompt,
                queued_prompts,
            )?;
        }
        Ok(())
    }

    pub(super) fn activate_next_queued_prompt_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self.prompt_state_owner.activate_next_queued_prompt(
            &session,
            agent_id,
            expected_prompt_id,
        )?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        Ok(prompt)
    }

    pub(super) fn advance_next_queued_prompt_dispatch(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<crate::app::KernelPromptDispatch>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let Some(next_prompt) = self
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent_id)
        else {
            return Ok(None);
        };
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "advance queued prompt",
            });
        }
        self.acquire_provider_prompt_claim(
            session_id,
            provider_run_id,
            agent_id,
            Some(next_prompt.source_attachment_id()),
        )?;
        let started_next = self
            .prompt_state_owner
            .activate_next_queued_prompt(&session, agent_id, Some(next_prompt.id()))?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "advance queued prompt",
                message: format!(
                    "expected queued prompt `{}` but no queued prompt was available",
                    next_prompt.id()
                ),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
            started_next.workflow_run_id(),
            started_next.workflow_node_run_id(),
        ) {
            let _ = self.session_store.write().mark_workflow_turn_dispatched(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            let _ = self.workflow_start_prompt(session_id, &started_next)?;
        }
        let _ = self.session_snapshot(session_id)?;
        Ok(Some(crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.to_string(),
            source_attachment_id: started_next.source_attachment_id().to_string(),
            prompt: started_next.prompt().to_string(),
            attachments: started_next.attachments().to_vec(),
        }))
    }

    pub(super) fn start_provider_launch(
        &self,
        request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::app::StartedProviderLaunch, DaemonError> {
        let session_id = request.session_id.clone();
        let previous_active_run_id = self
            .session_store
            .get_session(&session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = previous_active_run_id.as_deref() {
            let active_run = self.provider_store.get_run(active_run_id)?;
            match active_run.state() {
                crate::provider::ProviderRunState::Ended => {
                    self.session_store
                        .set_active_provider_run(&session_id, None)?;
                    self.provider_store.clear_runtime(active_run_id);
                }
                crate::provider::ProviderRunState::Starting => {
                    let outcome = self
                        .provider_store
                        .terminate_run_provider_only(&session_id, active_run_id)?;
                    self.clear_active_provider_run_session_pointer(
                        &session_id,
                        outcome.run().id(),
                    )?;
                    self.provider_run_projection.update(outcome.into_run());
                }
                crate::provider::ProviderRunState::Running => {
                    let outcome = self
                        .provider_store
                        .park_run_provider_only(&session_id, active_run_id)?;
                    self.clear_active_provider_run_session_pointer(
                        &session_id,
                        outcome.run().id(),
                    )?;
                    self.provider_run_projection.update(outcome.into_run());
                }
                crate::provider::ProviderRunState::Parked => {
                    self.session_store
                        .set_active_provider_run(&session_id, None)?;
                }
            }
        }

        let outcome = self.provider_store.start_run_provider_only(request)?;
        self.session_store
            .set_active_provider_run(&session_id, Some(outcome.run().id().to_string()))?;
        Ok(crate::app::StartedProviderLaunch {
            run: outcome.into_run(),
            previous_active_run_id,
        })
    }

    pub(super) fn resume_provider_run_for_session(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let active_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            if active_run_id != run_id {
                let outcome = self
                    .provider_store
                    .park_run_provider_only(session_id, active_run_id)?;
                self.clear_active_provider_run_session_pointer(session_id, outcome.run().id())?;
                self.provider_run_projection.update(outcome.into_run());
            }
        }

        let outcome = self
            .provider_store
            .resume_run_provider_only(session_id, run_id)?;
        self.session_store
            .set_active_provider_run(session_id, Some(outcome.run().id().to_string()))?;
        let run = outcome.into_run();
        self.provider_run_projection.update(run.clone());
        Ok(run)
    }

    pub(super) fn clear_active_provider_run_session_pointer(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        if self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            == Some(provider_run_id)
        {
            self.session_store
                .set_active_provider_run(session_id, None)?;
        }
        Ok(())
    }

    pub(super) fn finish_provider_launch_success(
        &self,
        started: &crate::app::StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        if let Some(binding) = binding {
            self.provider_store
                .apply_runtime_binding(started.run.id(), binding)?;
        }
        let run = self.provider_store.mark_run_running(started.run.id())?;
        self.session_store
            .set_active_provider_run(run.session_id(), Some(run.id().to_string()))?;
        let _ = self.session_snapshot(run.session_id())?;
        crate::logging::info_with_fields(
            "daemon.app",
            "initializing provider runtime",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        crate::logging::info_with_fields(
            "daemon.app",
            "provider runtime initialized successfully",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        let _ = self.provider_store.record_run_activity(run.id());
        if let Some(agent_id) = run.agent_instance_id() {
            let _ = self.agent_store.set_agent_runtime_profile(
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                run.resume_state().clone(),
            )?;
        }
        self.provider_run_projection.update(run.clone());
        Ok(run)
    }

    pub(super) fn cancel_active_prompt_only(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let cancelled = self
            .prompt_state_owner
            .cancel_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        Ok(cancelled)
    }

    pub(super) fn complete_local_prompt_without_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<Option<OwnedPromptCompletion>, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        let _active = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;

        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;

        let completion_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        });
        let completion_record_key = provider_run_id.unwrap_or(agent_id);
        if !self.prompt_completion_recorded(completion_record_key) {
            let provider_run_id = completion_provider_run_id
                .as_deref()
                .unwrap_or("provider-run-completed");
            let recipient_attachment_ids = self
                .attachment_store
                .list_session_attachment_ids(session_id);
            self.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids,
                &format!("prompt-complete:{}", completed.id()),
                crate::session::unix_epoch_ms(),
            );
            self.mark_prompt_completion_recorded(provider_run_id);
        }
        let released_claim = completion_provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let _ = self.session_snapshot(session_id)?;

        Ok(Some(OwnedPromptCompletion {
            completion: crate::session::PromptCompletion {
                completed,
                started_next: None,
            },
            released_claim,
            dispatch: None,
        }))
    }

    pub(super) fn submit_local_prepared_prompt(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let session_id = prepared.session_id.clone();
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
            let _ = self.ensure_attachment_in_session(&session_id, &attachment_id)?;
        }
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        let target_agent = self.agent_store.get_agent(&target_agent_id)?;
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(&session_id)?;
        let queued_while_active = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &target_agent_id)
            .is_some();
        let provider_run_id = self
            .provider_store
            .get_run_for_agent(&session_id, &target_agent_id)
            .map(|run| run.id().to_string());
        if !queued_while_active && provider_run_id.is_none() {
            return Ok(None);
        }
        if !queued_while_active {
            if let Some(provider_run_id) = provider_run_id.as_deref() {
                let provider_run =
                    self.ensure_provider_run_in_session(&session_id, provider_run_id)?;
                if provider_run.state() == crate::provider::ProviderRunState::Parked {
                    let _ = self.resume_provider_run_for_session(&session_id, provider_run_id)?;
                }
            }
        }
        let provider_run_is_starting = provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.provider_store.get_run(provider_run_id).ok())
            .is_some_and(|run| run.state() == crate::provider::ProviderRunState::Starting);

        self.append_user_prompt_history(
            &session_id,
            &attachment_id,
            &target_agent_id,
            prepared.prompt.prompt(),
            prepared.prompt.attachments(),
        )?;
        let force_queue = prepared.force_queue || provider_run_is_starting;
        let outcome = self.prompt_state_owner.submit_prepared_prompt(
            &session,
            prepared.prompt.clone(),
            force_queue,
        );
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.session_store.mirror_agent_prompt_state(
            &session_id,
            &outcome_agent_id,
            active_prompt,
            queued_prompts,
        )?;

        let mut dispatch = None;
        match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => {
                let provider_run_id =
                    provider_run_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::NoActiveProviderRun {
                            session_id: session_id.clone(),
                        })?;
                self.echo_prompt_to_other_attachments(
                    &session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                if let Err(error) = self.acquire_provider_prompt_claim(
                    &session_id,
                    provider_run_id,
                    &target_agent_id,
                    Some(prompt.source_attachment_id()),
                ) {
                    let _ = self.cancel_active_prompt_only(&session_id, &target_agent_id);
                    let _ = self.clear_prompt_activity(provider_run_id);
                    return Err(error);
                }
                dispatch = Some(crate::app::KernelPromptDispatch {
                    session_id: session_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: target_agent_id.clone(),
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    attachments: prompt.attachments().to_vec(),
                });
            }
            crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                let queue_depth = self
                    .prompt_state_owner
                    .queued_prompt_count_for_agent(&session, &target_agent_id);
                if let Some(provider_run_id) = provider_run_id.as_deref() {
                    self.echo_prompt_to_other_attachments(
                        &session_id,
                        provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                }
                self.record_notice(
                    &session_id,
                    provider_run_id.as_deref(),
                    self.other_attachment_ids(&session_id, &attachment_id),
                    format!(
                        "A queued message from attachment `{}` was added to agent `{}` in session `{}` as `{}`. Queue depth is now {}.",
                        attachment_id,
                        target_agent_id,
                        session_id,
                        prompt.id(),
                        queue_depth
                    ),
                );
            }
        }
        let session = self.session_snapshot(&session_id)?;
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome,
            session,
            dispatch,
            remote_dispatch: None,
        }))
    }

    pub(super) fn complete_local_prompt_with_queued_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
        next_queued_prompt: &crate::session::PromptQueueItem,
    ) -> Result<Option<OwnedPromptCompletion>, DaemonError> {
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(
            next_queued_prompt.source_attachment_id(),
        ) {
            let _ = self.ensure_attachment_in_session(
                session_id,
                next_queued_prompt.source_attachment_id(),
            )?;
        }
        let provider_run_id = provider_run_id
            .map(str::to_string)
            .or_else(|| {
                self.provider_store
                    .get_run_for_agent(session_id, agent_id)
                    .map(|run| run.id().to_string())
            })
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self.ensure_provider_run_in_session(session_id, &provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Ok(None);
        }
        self.acquire_provider_prompt_claim(
            session_id,
            &provider_run_id,
            agent_id,
            Some(next_queued_prompt.source_attachment_id()),
        )?;

        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let started_next = self
            .prompt_state_owner
            .activate_next_queued_prompt(&session, agent_id, Some(next_queued_prompt.id()))?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "advance queued prompt",
                message: format!(
                    "expected queued prompt `{}` but no queued prompt was available",
                    next_queued_prompt.id()
                ),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        if self
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            if let Err(error) = self.provider_store.enqueue_structured_prompt_submit(
                session_id.to_string(),
                provider_run_id.clone(),
                agent_id.to_string(),
                &provider_run,
                started_next.prompt(),
                started_next.attachments(),
            ) {
                let _ = self.cancel_active_prompt_only(session_id, agent_id);
                let _ = self.clear_prompt_activity(&provider_run_id);
                return Err(error);
            }
            self.note_prompt_started(&provider_run_id);
            let _ = self.session_snapshot(session_id)?;
            return Ok(Some(OwnedPromptCompletion {
                completion: crate::session::PromptCompletion {
                    completed,
                    started_next: Some(started_next),
                },
                released_claim: false,
                dispatch: None,
            }));
        }
        let _ = self.session_snapshot(session_id)?;
        Ok(Some(OwnedPromptCompletion {
            completion: crate::session::PromptCompletion {
                completed,
                started_next: Some(started_next.clone()),
            },
            released_claim: false,
            dispatch: Some(crate::app::KernelPromptDispatch {
                session_id: session_id.to_string(),
                provider_run_id,
                agent_id: agent_id.to_string(),
                source_attachment_id: started_next.source_attachment_id().to_string(),
                prompt: started_next.prompt().to_string(),
                attachments: started_next.attachments().to_vec(),
            }),
        }))
    }

    pub(super) fn finalize_local_prompt_cancellation_with_queued_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<OwnedPromptCancellation, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .finalize_active_prompt_cancellation(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        });
        let released_claim = provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let started_next = if self
            .prompt_state_owner
            .active_prompt_for_agent(&self.session_store.get_session(session_id)?, agent_id)
            .is_none()
        {
            let next_prompt = self
                .prompt_state_owner
                .peek_next_queued_prompt(&self.session_store.get_session(session_id)?, agent_id);
            if let (Some(provider_run_id), Some(next_prompt)) =
                (provider_run_id.as_deref(), next_prompt.as_ref())
            {
                let provider_run =
                    self.ensure_provider_run_in_session(session_id, provider_run_id)?;
                if provider_run.state() == crate::provider::ProviderRunState::Running {
                    self.acquire_provider_prompt_claim(
                        session_id,
                        provider_run_id,
                        agent_id,
                        Some(next_prompt.source_attachment_id()),
                    )?;
                    self.prompt_state_owner.activate_next_queued_prompt(
                        &self.session_store.get_session(session_id)?,
                        agent_id,
                        Some(next_prompt.id()),
                    )?
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&self.session_store.get_session(session_id)?, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        if started_next.is_none() {
            self.sync_focused_provider_run_if_idle(session_id)?;
        }
        let dispatch = if let (Some(provider_run_id), Some(started_next)) =
            (provider_run_id.as_deref(), started_next.as_ref())
        {
            let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
            if self
                .provider_store
                .run_uses_structured_prompt_io(&provider_run)
            {
                self.provider_store.enqueue_structured_prompt_submit(
                    session_id.to_string(),
                    provider_run_id.to_string(),
                    agent_id.to_string(),
                    &provider_run,
                    started_next.prompt(),
                    started_next.attachments(),
                )?;
                self.note_prompt_started(provider_run_id);
                None
            } else {
                Some(crate::app::KernelPromptDispatch {
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: agent_id.to_string(),
                    source_attachment_id: started_next.source_attachment_id().to_string(),
                    prompt: started_next.prompt().to_string(),
                    attachments: started_next.attachments().to_vec(),
                })
            }
        } else {
            None
        };
        let _ = self.session_snapshot(session_id)?;
        Ok(OwnedPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next,
            },
            released_claim,
            dispatch,
        })
    }

    pub(super) fn cancel_local_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<Option<crate::app::KernelPromptCancellation>, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(target_agent_id)?;
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        let active_prompt = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            let session = self.session_snapshot(session_id)?;
            return Ok(Some(crate::app::KernelPromptCancellation {
                cancellation: crate::session::PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                session,
                dispatch: None,
            }));
        }

        let provider_run = self
            .provider_run_projection
            .get_for_agent(session_id, target_agent_id)
            .or_else(|| {
                self.provider_store
                    .get_run_for_agent(session_id, target_agent_id)
            })
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run.id())?;

        let prompt = self
            .prompt_state_owner
            .begin_cancelling_active_prompt(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, target_agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            target_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        self.note_prompt_settlement_requested(provider_run.id());
        let recipients = self.other_attachment_ids(session_id, attachment_id);
        self.record_notice(
            session_id,
            Some(provider_run.id()),
            recipients,
            format!(
                "Attachment `{}` requested cancellation of active prompt `{}` on provider run `{}`.",
                attachment_id,
                prompt.id(),
                provider_run.id()
            ),
        );
        let session = self.session_snapshot(session_id)?;

        Ok(Some(crate::app::KernelPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next: None,
            },
            session,
            dispatch: Some(crate::app::KernelPromptAbortDispatch {
                session_id: session_id.to_string(),
                provider_run_id: provider_run.id().to_string(),
                source_attachment_id: attachment_id.to_string(),
            }),
        }))
    }

    pub(super) fn submit_remote_prepared_prompt(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let session_id = prepared.session_id.clone();
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
            let _ = self.ensure_attachment_in_session(&session_id, &attachment_id)?;
        }
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        let target_agent = self.agent_store.get_agent(&target_agent_id)?;
        let Some(remote_execution) = target_agent.remote_execution().cloned() else {
            return Ok(None);
        };
        self.append_user_prompt_history(
            &session_id,
            &attachment_id,
            &target_agent_id,
            prepared.prompt.prompt(),
            prepared.prompt.attachments(),
        )?;
        let session = self.session_store.get_session(&session_id)?;
        let outcome = self.prompt_state_owner.submit_prepared_prompt(
            &session,
            prepared.prompt.clone(),
            prepared.force_queue,
        );
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.session_store.mirror_agent_prompt_state(
            &session_id,
            &outcome_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let remote_dispatch =
            if let crate::session::PromptSubmissionOutcome::Started { prompt } = &outcome {
                Some(crate::app::KernelRemotePromptDispatch {
                    session_id: session_id.clone(),
                    agent_id: target_agent_id,
                    worker_kernel_id: remote_execution.worker_kernel_id,
                    leased_agent_id: remote_execution.leased_agent_id,
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    attachments: prompt.attachments().to_vec(),
                    workflow_context: None,
                })
            } else {
                None
            };
        let session = self.session_snapshot(&session_id)?;
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome,
            session,
            dispatch: None,
            remote_dispatch,
        }))
    }

    pub(super) fn complete_remote_prompt_owner(
        &self,
        session_id: &str,
        agent_id: &str,
        remote_provider_run_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        self.record_assistant_message_completion(
            session_id,
            remote_provider_run_id,
            recipient_attachment_ids,
            &format!("prompt-complete:{}", completed.id()),
            crate::session::unix_epoch_ms(),
        );
        let started_next = if self
            .prompt_state_owner
            .active_prompt_for_agent(&self.session_store.get_session(session_id)?, agent_id)
            .is_none()
        {
            if let Some(expected_next) = next_queued_prompt {
                let session = self.session_store.get_session(session_id)?;
                let active = self.prompt_state_owner.activate_next_queued_prompt(
                    &session,
                    agent_id,
                    Some(expected_next.id()),
                )?;
                let (active_prompt, queued_prompts) =
                    self.prompt_state_owner.state_parts(&session, agent_id);
                self.session_store.mirror_agent_prompt_state(
                    session_id,
                    agent_id,
                    active_prompt,
                    queued_prompts,
                )?;
                active
            } else {
                None
            }
        } else {
            None
        };
        let _ = self.session_snapshot(session_id)?;
        Ok(crate::session::PromptCompletion {
            completed,
            started_next,
        })
    }

    pub(super) fn begin_remote_prompt_cancellation(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let session = self.session_store.get_session(session_id)?;
        let active_prompt = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            let session = self.session_snapshot(session_id)?;
            return Ok(crate::app::KernelPromptCancellation {
                cancellation: crate::session::PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                session,
                dispatch: None,
            });
        }
        let prompt = self
            .prompt_state_owner
            .begin_cancelling_active_prompt(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, target_agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            target_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let worker_kernel_id = self
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .map(|remote| remote.worker_kernel_id.clone())
            .unwrap_or_else(|| "remote".to_string());
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{attachment_id}` requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                prompt.id(),
                worker_kernel_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next: None,
            },
            session,
            dispatch: None,
        })
    }

    pub(super) fn other_attachment_ids(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<String> {
        self.attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|id| id != attachment_id)
            .collect()
    }

    pub(super) fn prompt_completion_recorded(&self, provider_run_id: &str) -> bool {
        self.prompt_activity
            .read()
            .get(provider_run_id)
            .map(|state| state.completion_recorded)
            .unwrap_or(false)
    }

    pub(super) fn mark_prompt_completion_recorded(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.completion_recorded = true;
        }
    }

    pub(super) fn record_assistant_message_completion(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) {
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        self.terminal_stream.record_assistant_message_completion(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
    }

    pub(super) fn reap_structured_prompt_jobs(&self) {
        self.provider_store
            .apply_finished_provider_run_selection_sync_jobs();
        for finished in self
            .provider_store
            .drain_finished_structured_prompt_submit_jobs()
        {
            if let Err(error) = finished.result {
                let _ = self.cancel_active_prompt_only(&finished.session_id, &finished.agent_id);
                let _ = self.session_snapshot(&finished.session_id);
                let recipients = self
                    .attachment_store
                    .list_session_attachment_ids(&finished.session_id);
                self.record_notice(
                    &finished.session_id,
                    Some(&finished.provider_run_id),
                    recipients,
                    format!("Prompt dispatch failed after acknowledgement: {error}"),
                );
            }
        }
        for finished in self
            .provider_store
            .drain_finished_structured_prompt_abort_jobs()
        {
            if let Err(error) = finished.result {
                let recipients = self
                    .attachment_store
                    .list_session_attachment_ids(&finished.session_id);
                self.record_notice(
                    &finished.session_id,
                    Some(&finished.provider_run_id),
                    recipients,
                    format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
                );
            }
        }
    }

    pub(super) fn fan_out_terminal_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        kind: crate::terminal::TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> crate::terminal::TerminalOutputRecord {
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        let record = self.terminal_stream.fan_out_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            kind.clone(),
            merge_key.clone(),
            recipient_attachment_ids,
            bytes,
        );
        if kind != crate::terminal::TerminalOutputKind::PromptEcho {
            self.append_history_entry(
                session_id,
                SessionHistoryEntry::provider_output(
                    session_id,
                    provider_run_id,
                    agent_id.as_deref(),
                    kind,
                    merge_key,
                    String::from_utf8_lossy(bytes).into_owned(),
                ),
            );
        }
        record
    }

    pub(super) fn append_history_entry(&self, session_id: &str, entry: SessionHistoryEntry) {
        let session = match self.session_store.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping provider-output history append because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if let Err(error) = self.history_store.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append provider-output session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        } else {
            self.history_projection.append(entry);
        }
    }

    pub(super) fn clear_prompt_activity(&self, provider_run_id: &str) -> bool {
        self.prompt_activity.write().remove(provider_run_id);
        self.prompt_workspace_claims.remove(provider_run_id)
    }

    pub(super) fn release_workflow_node_workspace_claim(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> bool {
        let owner = format!("{workflow_run_id}:{workflow_node_run_id}");
        self.prompt_workspace_claims.remove_matching(|claim| {
            claim.session_id == session_id
                && claim.attachment_id.as_deref() == Some(owner.as_str())
                && claim.operation == "workflow_node_dispatch"
        }) > 0
    }

    pub(super) fn note_prompt_started(&self, provider_run_id: &str) {
        self.prompt_activity.write().insert(
            provider_run_id.to_string(),
            crate::app::ActivePromptState {
                last_output_at: None,
                saw_response_content: false,
                completion_recorded: false,
            },
        );
    }

    pub(super) fn note_prompt_output(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
        }
    }

    pub(super) fn note_prompt_response_content(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
            state.saw_response_content = true;
        }
    }

    pub(super) fn note_prompt_settlement_requested(&self, provider_run_id: &str) {
        self.prompt_activity
            .write()
            .entry(provider_run_id.to_string())
            .and_modify(|state| {
                state.last_output_at = Some(Instant::now());
                state.saw_response_content = true;
            })
            .or_insert(crate::app::ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: false,
            });
    }

    pub(super) fn prompt_should_settle(&self, provider_run_id: &str) -> bool {
        self.prompt_activity
            .read()
            .get(provider_run_id)
            .map(|state| {
                (state.saw_response_content || state.completion_recorded)
                    && state
                        .last_output_at
                        .map(|last_output_at| last_output_at.elapsed() >= self.prompt_idle_timeout)
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    pub(super) fn acquire_provider_prompt_claim(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        if self.prompt_workspace_claims.contains(provider_run_id) {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agent_store
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_provider_prompt_claim(
            workspace_id,
            worktree_id,
            session_id,
            attachment_id.map(str::to_string),
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }

    pub(super) fn acquire_workflow_node_workspace_claim(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<(), DaemonError> {
        if self.prompt_workspace_claims.contains(provider_run_id) {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agent_store
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_worktree_write_claim(
            workspace_id,
            worktree_id,
            session_id,
            Some(format!("{workflow_run_id}:{workflow_node_run_id}")),
            "workflow_node_dispatch",
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }

    pub(super) fn append_user_prompt_history(
        &self,
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
    ) -> Result<(), DaemonError> {
        let session = self.session_snapshot(session_id)?;
        let entry = crate::history::SessionHistoryEntry::user_prompt(
            session_id,
            source_attachment_id,
            agent_id,
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments),
        );
        self.history_store.append(&session, &entry)?;
        self.history_projection.append(entry);
        Ok(())
    }

    pub(super) fn echo_prompt_to_other_attachments(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
    ) {
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|attachment_id| attachment_id != source_attachment_id)
            .collect::<Vec<_>>();
        if recipient_attachment_ids.is_empty() {
            return;
        }
        let mut bytes =
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments).into_bytes();
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        self.terminal_stream.fan_out_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            crate::terminal::TerminalOutputKind::PromptEcho,
            None,
            recipient_attachment_ids,
            &bytes,
        );
    }

    pub(super) fn ensure_provider_run_in_session(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let provider_run = self.provider_store.get_run(provider_run_id)?;
        if provider_run.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.to_string(),
            });
        }
        Ok(provider_run)
    }

    pub(super) fn remove_provider_process_tracking_for_run(
        &self,
        provider_run_id: &str,
        pty_process_key: Option<String>,
    ) {
        self.workspace_identity_monitor
            .remove_provider_run(provider_run_id);
        let process_key = self
            .provider_process_tracking
            .read()
            .run_processes
            .get(provider_run_id)
            .cloned()
            .or(pty_process_key);
        let Some(process_key) = process_key else {
            return;
        };
        let mut tracking = self.provider_process_tracking.write();
        tracking.run_processes.remove(provider_run_id);
        let should_remove_entry = if let Some(entry) = tracking.processes.get_mut(&process_key) {
            entry
                .owner_provider_run_ids
                .retain(|id| id != provider_run_id);
            entry.owner_provider_run_ids.is_empty()
        } else {
            false
        };
        if should_remove_entry {
            tracking.processes.remove(&process_key);
        }
    }

    pub(super) fn reconcile_provider_run_liveness_provider_phase(
        &self,
        session_id: &str,
        provider_run_id: &str,
        process_running: Option<bool>,
    ) -> Result<Option<OwnedProviderRunExit>, DaemonError> {
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let _ = provider_run
            .agent_instance_id()
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        let reconciliation = self.provider_store.reconcile_run_liveness_provider_only(
            session_id,
            provider_run_id,
            process_running,
        )?;
        match reconciliation {
            crate::provider::ProviderRunLivenessReconciliation::AlreadyEnded(run) => {
                self.clear_active_provider_run_session_pointer(session_id, provider_run_id)?;
                self.provider_run_projection.update(run.clone());
                Ok(Some(OwnedProviderRunExit {
                    ended_run: run,
                    already_ended: true,
                }))
            }
            crate::provider::ProviderRunLivenessReconciliation::NewlyEnded(run) => {
                self.clear_active_provider_run_session_pointer(session_id, provider_run_id)?;
                self.provider_run_projection.update(run.clone());
                Ok(Some(OwnedProviderRunExit {
                    ended_run: run,
                    already_ended: false,
                }))
            }
            crate::provider::ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | crate::provider::ProviderRunLivenessReconciliation::StillRunning(_) => Ok(None),
        }
    }

    pub(super) fn record_notice(
        &self,
        session_id: &str,
        provider_run_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) {
        let message = message.into();
        let agent_id = provider_run_id.and_then(|run_id| {
            self.provider_store
                .get_run(run_id)
                .ok()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        self.terminal_stream.record_notice(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message.clone(),
        );
        let session = match self.session_store.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping history append because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        let entry =
            SessionHistoryEntry::notice(session_id, provider_run_id, agent_id.as_deref(), message);
        if let Err(error) = self.history_store.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        } else {
            self.history_projection.append(entry);
        }
    }

    pub(super) fn workflow_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.session_snapshot(session_id)
    }

    pub(super) fn workflow_create_workflow(
        &self,
        request: crate::local::CreateWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .create_workflow(&request.session_id, request.alias)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
    }

    pub(super) fn workflow_alias_workflow(
        &self,
        request: crate::local::AliasWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self.session_store.write().assign_workflow_alias(
            &request.session_id,
            &request.workflow_ref,
            request.alias,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
    }

    pub(super) fn workflow_list_workflows(
        &self,
        request: crate::local::ListWorkflowsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowsListed {
            workflows: self
                .session_store
                .read()
                .list_workflows(&request.session_id)?,
        })
    }

    pub(super) fn workflow_resolve_workflow(
        &self,
        request: crate::local::ResolveWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowResolved {
            workflow: self
                .session_store
                .read()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
        })
    }

    pub(super) fn workflow_create_endpoint(
        &self,
        request: crate::local::CreateWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.session_store.write().create_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            request.alias,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_alias_endpoint(
        &self,
        request: crate::local::AliasWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.session_store.write().assign_workflow_endpoint_alias(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.alias,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_bind_endpoint(
        &self,
        request: crate::local::BindWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.session_store.write().bind_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            &request.entry_node_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_add_node(
        &self,
        request: crate::local::AddWorkflowNodeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if self
            .agent_store
            .get_session_agents(&request.session_id)
            .into_iter()
            .all(|agent| agent.id() != request.agent_id)
        {
            return Err(DaemonError::AgentNotFound {
                agent_id: request.agent_id,
            });
        }
        let node = self.session_store.write().add_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.agent_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeAdded {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_remove_node(
        &self,
        request: crate::local::RemoveWorkflowNodeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self.session_store.write().remove_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeRemoved {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_update_node_instructions(
        &self,
        request: crate::local::UpdateWorkflowNodeInstructionsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .session_store
            .write()
            .update_workflow_node_instructions(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.instructions,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_node_can_complete_run(
        &self,
        request: crate::local::SetWorkflowNodeCanCompleteRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .session_store
            .write()
            .set_workflow_node_can_complete_run(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_complete_workflow_run,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_node_can_emit_intermediate_output(
        &self,
        request: crate::local::SetWorkflowNodeCanEmitIntermediateOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .session_store
            .write()
            .set_workflow_node_can_emit_intermediate_output(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_emit_intermediate_workflow_run_output,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(super) fn workflow_set_node_intermediate_output_schema(
        &self,
        request: crate::local::SetWorkflowNodeIntermediateOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .session_store
            .write()
            .set_workflow_node_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.intermediate_output_schema_ref,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    pub(super) fn workflow_set_node_max_turns(
        &self,
        request: crate::local::SetWorkflowNodeMaxTurnsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self.session_store.write().set_workflow_node_max_turns(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            request.max_turns,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_add_edge(
        &self,
        request: crate::local::AddWorkflowEdgeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let edge = self.session_store.write().add_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.from_node_id,
            &request.to_node_id,
            request.output_schema_ref,
            request.validation_policy,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_remove_edge(
        &self,
        request: crate::local::RemoveWorkflowEdgeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let edge = self.session_store.write().remove_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.edge_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow,
            session,
        })
    }

    pub(super) fn workflow_set_flush_context(
        &self,
        request: crate::local::SetWorkflowFlushContextRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .set_workflow_flush_agent_context_before_run(
                &request.session_id,
                &request.workflow_ref,
                request.flush_agent_context_before_run,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
    }

    pub(super) fn workflow_set_run_output_schema(
        &self,
        request: crate::local::SetWorkflowRunOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .set_workflow_run_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.run_output_schema_ref,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
    }

    pub(super) fn workflow_set_intermediate_output_schema(
        &self,
        request: crate::local::SetWorkflowIntermediateOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .set_workflow_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.intermediate_output_schema_ref,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { workflow, session })
    }

    pub(super) fn workflow_set_launch_policy(
        &self,
        request: crate::local::SetWorkflowLaunchPolicyRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self
            .session_store
            .write()
            .set_workflow_launch_policy(&request.session_id, request.policy)?;
        let mut session = session;
        session.set_agents(self.agent_store.get_session_agents(&request.session_id));
        self.project_session_runtime_view(&mut session);
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session })
    }

    pub(super) fn workflow_list_runs(
        &self,
        request: crate::local::ListWorkflowRunsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs: self
                .session_store
                .read()
                .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(super) fn workflow_get_run(
        &self,
        request: crate::local::GetWorkflowRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRun {
            workflow_run: self
                .session_store
                .read()
                .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
        })
    }

    pub(super) fn workflow_create_watchdog(
        &self,
        request: crate::local::CreateWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.session_store.write().create_workflow_watchdog(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.interval_seconds,
            request.invocation_prompt,
            request.policy,
            if request.max_wakeups_configured {
                Some(request.max_wakeups)
            } else {
                None
            },
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogCreated {
            watchdog,
            workflow,
            endpoint,
            session,
        })
    }

    pub(super) fn workflow_list_watchdogs(
        &self,
        request: crate::local::ListWorkflowWatchdogsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
            watchdogs: self
                .session_store
                .read()
                .list_workflow_watchdogs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(super) fn workflow_set_watchdog_enabled(
        &self,
        request: crate::local::SetWorkflowWatchdogEnabledRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.session_store.write().set_workflow_watchdog_enabled(
            &request.session_id,
            &request.watchdog_ref,
            request.enabled,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
    }

    pub(super) fn workflow_remove_watchdog(
        &self,
        request: crate::local::RemoveWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self
            .session_store
            .write()
            .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
    }

    pub(super) fn workflow_list_queued_launches(
        &self,
        request: crate::local::ListQueuedWorkflowLaunchesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
            queued_launches: self
                .session_store
                .read()
                .list_queued_workflow_launches(&request.session_id)?,
        })
    }

    pub(super) fn workflow_remove_queued_launch(
        &self,
        request: crate::local::RemoveQueuedWorkflowLaunchRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_launch = self
            .session_store
            .write()
            .remove_queued_workflow_launch(&request.session_id, &request.queue_item_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
            queued_launch,
            session,
        })
    }

    pub(super) fn workflow_clear_queued_launches(
        &self,
        request: crate::local::ClearQueuedWorkflowLaunchesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_launches = self
            .session_store
            .write()
            .clear_queued_workflow_launches(&request.session_id)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
            queued_launches,
            session,
        })
    }

    pub(super) fn workflow_validate_output(
        &self,
        request: crate::local::ValidateWorkflowOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let warning = crate::transport::runtime_tools::validate_workflow_output_schema(
            &request.output_schema_ref,
            &request.output_json,
        )
        .err();
        Ok(LocalDaemonResponse::WorkflowOutputValidated {
            valid: warning.is_none(),
            warning,
        })
    }

    pub(super) fn workflow_ack_turn(
        &self,
        request: crate::local::AckWorkflowTurnRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_run_id = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
            .id()
            .to_string();
        let workflow_run = self.session_store.write().ack_workflow_turn(
            &request.session_id,
            &workflow_run_id,
            &request.workflow_node_run_id,
            &request.delivery_token,
        )?;
        let event = crate::session::WorkflowRuntimeToolCallEvent::new(
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL.to_string(),
            serde_json::json!({"delivery_token": request.delivery_token}).to_string(),
            Some(
                serde_json::json!({
                    "workflow_run_id": workflow_run.id(),
                    "workflow_node_run_id": request.workflow_node_run_id,
                    "state": "acknowledged",
                    "next_action": "Continue this same workflow turn. This acknowledgement is not the final answer. If this turn requires final workflow run output, call validate_and_submit_workflow_run_output before stopping; otherwise emit the required final fenced json block before stopping.",
                })
                .to_string(),
            ),
            true,
        );
        let _ = self
            .session_store
            .write()
            .record_workflow_runtime_tool_call(
                &request.session_id,
                &request.workflow_node_run_id,
                event,
            );
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &workflow_run_id)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowTurnAcknowledged {
            workflow_run,
            session,
        })
    }

    pub(super) fn workflow_start_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.session_store.write().start_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let recipients = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        let active_provider_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);
        self.record_notice(
            session_id,
            active_provider_run_id.as_deref(),
            recipients,
            format!(
                "Workflow run `{}` started on agent `{}`.",
                workflow_run.id(),
                prompt.target_agent_id()
            ),
        );
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    pub(super) fn workflow_ensure_provider_run(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
            if run.state() == crate::provider::ProviderRunState::Parked {
                let resumed = self.resume_provider_run_for_session(session_id, run.id())?;
                self.session_store
                    .set_active_provider_run(session_id, Some(resumed.id().to_string()))?;
                return Ok(resumed.id().to_string());
            }
            if run.state() != crate::provider::ProviderRunState::Ended {
                self.session_store
                    .set_active_provider_run(session_id, Some(run.id().to_string()))?;
                return Ok(run.id().to_string());
            }
        }
        let agent = self.agent_store.get_agent(agent_id)?;
        let adapter_key = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let provider = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let mut request = crate::provider::LaunchProviderRequest::new(
            session_id,
            adapter_key,
            provider,
            "default",
            agent.model().unwrap_or("default"),
        )
        .with_agent_id(agent.id().to_string())
        .with_variant(agent.effort().map(str::to_string));
        if crate::provider::provider_requires_managed_io_by_default(provider) {
            request = request.with_managed_io_required();
        }
        if let Some(worktree_id) = agent.worktree_id() {
            request = request.with_working_directory(std::path::PathBuf::from(worktree_id));
        }
        let run = self.provider_store.launch_run_detached(request)?;
        self.session_store
            .set_active_provider_run(session_id, Some(run.id().to_string()))?;
        self.provider_run_projection.update(run.clone());
        Ok(run.id().to_string())
    }

    pub(super) fn workflow_dispatch_claim_id(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if let Some(remote_execution) = agent.remote_execution() {
            return Ok(format!(
                "remote-workflow:{}:{}",
                remote_execution.worker_kernel_id, remote_execution.leased_agent_id
            ));
        }
        self.workflow_ensure_provider_run(session_id, agent_id)
    }

    pub(super) fn workflow_submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let mut dispatches = WorkflowPromptDispatches::default();
        let mut submission = match self.submit_local_prepared_prompt(&prepared)? {
            Some(submission) => submission,
            None => match self.submit_remote_prepared_prompt(&prepared)? {
                Some(submission) => submission,
                None => return Ok(dispatches),
            },
        };
        if let crate::session::PromptSubmissionOutcome::Started { prompt } = &submission.outcome {
            let _ = self.session_store.write().mark_workflow_turn_dispatched(
                &prepared.session_id,
                workflow_run_id,
                workflow_node_run_id,
            );
            let _ = self.workflow_start_prompt(&prepared.session_id, prompt);
        }
        if let Some(dispatch) = submission.dispatch.take() {
            dispatches.local.push(dispatch);
        }
        if let Some(mut dispatch) = submission.remote_dispatch.take() {
            if dispatch.workflow_context.is_none() {
                let prompt = match &submission.outcome {
                    crate::session::PromptSubmissionOutcome::Started { prompt }
                    | crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
                };
                dispatch.workflow_context = Some(self.remote_workflow_turn_context_for_prompt(
                    &prepared.session_id,
                    prompt.target_agent_id(),
                    prompt,
                )?);
            }
            dispatches.remote.push(dispatch);
        }
        Ok(dispatches)
    }

    pub(super) fn remote_workflow_turn_context_for_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<crate::execution_lease::RemoteWorkflowTurnContext, DaemonError> {
        let workflow_run_id =
            prompt
                .workflow_run_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: "remote workflow prompt is missing workflow run id".to_string(),
                })?;
        let workflow_node_run_id =
            prompt
                .workflow_node_run_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: "remote workflow prompt is missing workflow node run id".to_string(),
                })?;
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let delivery_token = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .and_then(|node_run| node_run.turn_envelope())
            .map(|envelope| envelope.delivery_token().to_string())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "dispatch remote workflow prompt",
                message: format!(
                    "workflow node run `{workflow_node_run_id}` has no prepared turn envelope"
                ),
            })?;
        Ok(crate::execution_lease::RemoteWorkflowTurnContext {
            home_kernel_id: self.config_projection.snapshot().daemon_id,
            home_session_id: session_id.to_string(),
            home_agent_id: target_agent_id.to_string(),
            workflow_run_id: workflow_run.id().to_string(),
            workflow_node_run_id: workflow_node_run_id.to_string(),
            delivery_token,
        })
    }

    pub(super) fn workflow_validate_agents(
        &self,
        session_id: &str,
        workflow: &crate::session::WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        let agents = self.agent_store.get_session_agents(session_id);
        let agent_ids = agents
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        for node in workflow.nodes() {
            if !agent_ids.contains(node.agent_id()) {
                return Err(DaemonError::WorkflowNodeAgentMissing {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                });
            }
            let Some(agent) = agents.iter().find(|agent| agent.id() == node.agent_id()) else {
                continue;
            };
            let capabilities = self
                .provider_store
                .get_run_for_agent(session_id, node.agent_id())
                .unwrap_or_else(|| {
                    crate::provider::RuntimeProviderRun::from_control_capability_inference(
                        format!("inferred-{session_id}-{}", node.agent_id()),
                        session_id.to_string(),
                        Some(node.agent_id().to_string()),
                        agent.provider().to_string(),
                    )
                });
            if !capabilities
                .supports_control_operation(crate::provider::ControlOperation::AckWorkflowTurn)
            {
                return Err(DaemonError::WorkflowNodeControlUnsupported {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                    operation: "ack_workflow_turn",
                });
            }
            let requires_validation = workflow
                .edges()
                .iter()
                .any(|edge| edge.from_node_id() == node.id() && edge.output_schema_ref().is_some());
            if requires_validation
                && !capabilities.supports_control_operation(
                    crate::provider::ControlOperation::ValidateWorkflowOutput,
                )
            {
                return Err(DaemonError::WorkflowNodeControlUnsupported {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                    operation: "validate_workflow_output",
                });
            }
        }
        Ok(())
    }

    pub(super) fn workflow_flush_agent_context_if_needed(
        &self,
        session_id: &str,
        workflow: &crate::session::WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        if !workflow.flush_agent_context_before_run() {
            return Ok(());
        }
        let workflow_agent_ids = workflow
            .nodes()
            .iter()
            .map(|node| node.agent_id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        if workflow_agent_ids.is_empty() {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        for agent_id in &workflow_agent_ids {
            if self
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
                .is_some()
            {
                let _ = self.cancel_active_prompt_only(session_id, agent_id);
            }
        }
        for agent_id in workflow_agent_ids {
            let Some(run) = self.provider_store.get_run_for_agent(session_id, &agent_id) else {
                continue;
            };
            if run.state() == crate::provider::ProviderRunState::Ended {
                continue;
            }
            let ended = self
                .provider_store
                .terminate_run_provider_only(session_id, run.id())?
                .into_run();
            if self
                .session_store
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(ended.id())
            {
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            self.provider_run_projection.update(ended.clone());
            self.remove_provider_process_tracking_for_run(ended.id(), None);
        }
        Ok(())
    }

    pub(super) fn workflow_schedule_entry_node(
        &self,
        session_id: &str,
        workflow_run: &crate::session::WorkflowRun,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let endpoint_prompt = workflow_run
            .invocation_prompt()
            .map(str::trim)
            .unwrap_or("");
        let node_run = workflow_run.node_runs().first().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_run.workflow_id().to_string(),
                reference: workflow_run.id().to_string(),
                message: "workflow run has no entry node run",
            }
        })?;
        let prompt_text = self.workflow_turn_prompt_text(
            session_id,
            workflow_run.id(),
            node_run.id(),
            node_run.node_id(),
            endpoint_prompt,
            None,
            None,
        )?;
        let _ = self.session_store.write().prepare_workflow_turn(
            session_id,
            workflow_run.id(),
            node_run.id(),
            format!("workflow-ack:{}", node_run.id()),
            prompt_text.clone(),
            None,
            None,
        )?;
        let claim_id = self.workflow_dispatch_claim_id(session_id, node_run.agent_id())?;
        match self.acquire_workflow_node_workspace_claim(
            session_id,
            &claim_id,
            node_run.agent_id(),
            workflow_run.id(),
            node_run.id(),
        ) {
            Ok(()) => {}
            Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                let _ = self
                    .session_store
                    .write()
                    .block_workflow_node_on_workspace_claim(
                        session_id,
                        workflow_run.id(),
                        node_run.id(),
                    );
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{}` blocked node `{}` on a workspace claim: {error}",
                        workflow_run.id(),
                        node_run.node_id()
                    ),
                );
                let _ = self.session_snapshot(session_id)?;
                return Ok(WorkflowPromptDispatches::default());
            }
            Err(error) => return Err(error),
        }
        let _ = self
            .session_store
            .write()
            .ready_workflow_node_after_workspace_claim(
                session_id,
                workflow_run.id(),
                node_run.id(),
            );
        let prompt = crate::session::PromptQueueItem::new(
            self.session_store.reserve_prompt_id(),
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            node_run.agent_id(),
            prompt_text,
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run.id(), node_run.id());
        self.workflow_submit_prepared_prompt(
            crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
            },
            workflow_run.id(),
            node_run.id(),
        )
    }

    pub(super) fn workflow_invoke_queued_launch(
        &self,
        session_id: &str,
        queued_launch: crate::session::QueuedWorkflowLaunch,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, queued_launch.workflow_id())?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            queued_launch.workflow_id(),
            queued_launch.endpoint_id(),
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        self.workflow_flush_agent_context_if_needed(session_id, &workflow)?;
        let workflow_run = self.session_store.write().invoke_workflow_endpoint(
            session_id,
            workflow.id(),
            endpoint.id(),
            queued_launch.invocation_prompt().map(str::to_string),
        )?;
        let dispatches = self.workflow_schedule_entry_node(session_id, &workflow_run)?;
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self.session_store.write().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        Ok((
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            },
            dispatches,
        ))
    }

    pub(super) fn workflow_invoke_endpoint_with_admission(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        let admission = self.session_store.write().admit_manual_workflow_launch(
            session_id,
            workflow.id(),
            endpoint.id(),
            prompt.clone(),
        )?;
        match admission {
            crate::session::WorkflowLaunchAdmission::StartNow => {
                self.workflow_flush_agent_context_if_needed(session_id, &workflow)?;
                let workflow_run = self.session_store.write().invoke_workflow_endpoint(
                    session_id,
                    workflow.id(),
                    endpoint.id(),
                    prompt,
                )?;
                let dispatches = self.workflow_schedule_entry_node(session_id, &workflow_run)?;
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(session_id, workflow_run.id())?;
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                        workflow_run,
                        workflow,
                        endpoint,
                    },
                    dispatches,
                ))
            }
            crate::session::WorkflowLaunchAdmission::Queued(queued_launch) => Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued {
                    queued_launch,
                    workflow,
                    endpoint,
                },
                WorkflowPromptDispatches::default(),
            )),
        }
    }

    pub(super) fn workflow_cancel_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.session_store.write().stop_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let _ = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::RunStopped,
                workflow_node_run_id,
                Vec::new(),
                "workflow node run was stopped before validated completion",
            ),
        );
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!("Workflow run `{}` was stopped.", workflow_run.id()),
        );
        self.workflow_maybe_start_next_queued_launch(session_id);
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub(super) fn workflow_complete_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(WorkflowPromptDispatches::default());
        };
        let completion_snapshot = self.workflow_completion_snapshot(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            provider_run_id,
        );
        let max_turns = self.workflow_max_turns(session_id);
        let completion_result = self.session_store.write().complete_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            completion_snapshot.clone(),
            max_turns,
        );
        let update = match completion_result {
            Ok(update) => update,
            Err(crate::error::DaemonError::WorkflowOutputValidationFailed {
                edge_id,
                message,
                ..
            }) => {
                self.workflow_record_failure(
                    session_id,
                    workflow_run_id,
                    &crate::session::WorkflowFailureEvent::new(
                        crate::session::WorkflowFailureKind::OutputValidationFailed,
                        workflow_node_run_id,
                        vec![edge_id.clone()],
                        message.clone(),
                    ),
                );
                self.session_store.write().stop_workflow_node_run(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
                let _ = self.release_workflow_node_workspace_claim(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                );
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store.list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{workflow_run_id}` stopped after validation failed on edge `{edge_id}`: {message}"
                    ),
                );
                self.workflow_maybe_start_next_queued_launch(session_id);
                let _ = self.session_snapshot(session_id)?;
                return Ok(WorkflowPromptDispatches::default());
            }
            Err(error) => return Err(error),
        };
        for warning in &update.validation_warnings {
            let failure = crate::session::WorkflowFailureEvent::new(
                crate::session::classify_workflow_failure_kind(
                    &completion_snapshot,
                    &warning.message,
                ),
                workflow_node_run_id,
                vec![warning.edge_id.clone()],
                warning.message.clone(),
            );
            self.workflow_record_failure(session_id, workflow_run_id, &failure);
            self.record_notice(
                session_id,
                None,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Workflow output validation warning on edge `{}`: {}",
                    warning.edge_id, warning.message
                ),
            );
        }
        if update.workflow_run.status() == crate::session::WorkflowRunStatus::Stopped
            && update.workflow_run.final_output().is_none()
            && update.workflow_run.failure_events().iter().all(|event| {
                event.kind() != crate::session::WorkflowFailureKind::NodeTurnBudgetExhausted
            })
        {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::NodeTurnBudgetExhausted,
                    workflow_node_run_id,
                    Vec::new(),
                    "workflow run stopped after a node exhausted its turn budget",
                ),
            );
        }
        if update.workflow_run.final_output_valid() == Some(false) {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::WorkflowRunOutputValidationFailed,
                    workflow_node_run_id,
                    Vec::new(),
                    update
                        .workflow_run
                        .final_output_warning()
                        .unwrap_or("workflow run output validation failed"),
                ),
            );
        }
        if update.validation_warnings.is_empty() {
            let _ = self
                .session_store
                .write()
                .mark_workflow_turn_validated_completed(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
        }
        let claim_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, prompt.target_agent_id())
                .map(|run| run.id().to_string())
        });
        let released_claim = claim_provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let released_workflow_claim = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        let mut dispatches =
            self.workflow_prepare_dispatches(session_id, workflow_run_id, &update.dispatches);
        if released_claim || released_workflow_claim {
            dispatches.extend(self.workflow_retry_blocked_claims());
        }
        let state_suffix = match update.workflow_run.status() {
            crate::session::WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
            crate::session::WorkflowRunStatus::Completing => "is completing",
            crate::session::WorkflowRunStatus::Completed => "completed",
            crate::session::WorkflowRunStatus::Stopped => "stopped",
            _ => "updated",
        };
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!(
                "Workflow run `{}` {state_suffix}.",
                update.workflow_run.id()
            ),
        );
        if matches!(
            update.workflow_run.status(),
            crate::session::WorkflowRunStatus::Completed
                | crate::session::WorkflowRunStatus::Failed
                | crate::session::WorkflowRunStatus::Stopped
        ) {
            self.workflow_maybe_start_next_queued_launch(session_id);
        }
        let _ = self.session_snapshot(session_id)?;
        Ok(dispatches)
    }

    #[allow(dead_code)]
    pub(super) fn workflow_completion_snapshot(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: Option<&str>,
    ) -> Option<crate::session::WorkflowCompletionSnapshot> {
        let provider_run_id = provider_run_id
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let session = self.session_store.get_session(session_id).ok()?;
        let history = match self.history_store.load(&session) {
            Ok(history) => history,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.workflow",
                    "failed to load session history for workflow completion snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "workflow_run_id": workflow_run_id,
                        "workflow_node_run_id": workflow_node_run_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
                return None;
            }
        };
        self.history_projection
            .update_entries(session_id, history.clone());
        crate::scheduler::runtime::build_workflow_completion_snapshot_from_history(
            &session,
            history,
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            provider_run_id,
        )
    }

    pub(super) fn workflow_prompt_has_completion_output(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: &str,
    ) -> bool {
        self.workflow_completion_snapshot(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            Some(provider_run_id),
        )
        .and_then(|snapshot| snapshot.output().cloned())
        .is_some()
    }

    #[allow(dead_code)]
    pub(super) fn workflow_max_turns(&self, session_id: &str) -> Option<usize> {
        self.session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                session
                    .config_state()
                    .values()
                    .get("workflow.max_turns")
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .filter(|value| *value > 0)
            })
            .or(Some(
                crate::session::DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT,
            ))
    }

    pub(super) fn workflow_record_failure(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        failure: &crate::session::WorkflowFailureEvent,
    ) {
        let _ = self.session_store.write().record_workflow_failure_event(
            session_id,
            workflow_run_id,
            failure.clone(),
        );
    }

    #[allow(dead_code)]
    pub(super) fn workflow_control_mailbox_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        _workflow_node_run_id: &str,
    ) -> Option<String> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .ok()?;
        let lines = workflow_run
            .failure_events()
            .iter()
            .map(|failure| format!("- {:?}: {}", failure.kind(), failure.message()))
            .collect::<Vec<_>>();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    #[allow(dead_code)]
    pub(super) fn workflow_outgoing_edge_contracts_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        node_id: &str,
    ) -> String {
        let workflow_id = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run.workflow_id().to_string(),
            Err(_) => return String::new(),
        };
        let Ok(workflow) = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, &workflow_id)
        else {
            return String::new();
        };
        let lines = workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == node_id)
            .map(|edge| {
                let mut line = format!("- edge {} -> {}", edge.id(), edge.to_node_id());
                if let Some(schema_ref) = edge.output_schema_ref() {
                    line.push_str(&format!(", output_schema_ref: {schema_ref}"));
                }
                if let Some(validation_policy) = edge.validation_policy() {
                    line.push_str(&format!(", validation_policy: {validation_policy:?}"));
                }
                line
            })
            .collect::<Vec<_>>();
        if lines.is_empty() {
            String::new()
        } else {
            format!("Outgoing edge contracts:\n{}\n\n", lines.join("\n"))
        }
    }

    #[allow(dead_code)]
    pub(super) fn workflow_prepare_dispatches(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        dispatches: &[crate::session::WorkflowDispatch],
    ) -> WorkflowPromptDispatches {
        let mut prepared = WorkflowPromptDispatches::default();
        for dispatch in dispatches {
            if !self.workflow_dispatch_has_all_inputs(session_id, workflow_run_id, &dispatch) {
                continue;
            }
            self.record_notice(
                session_id,
                None,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` routed {} upstream message(s) to node `{}`.",
                    dispatch.messages.len(),
                    dispatch.node_run.node_id()
                ),
            );
            let handoff_payloads_json =
                serde_json::to_string(&dispatch.messages).unwrap_or_else(|_| "[]".to_string());
            let control_mailbox = self.workflow_control_mailbox_text(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
            );
            let prompt_text = match self.workflow_turn_prompt_text(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                dispatch.node_run.node_id(),
                "",
                Some(&handoff_payloads_json),
                control_mailbox.as_deref(),
            ) {
                Ok(prompt_text) => prompt_text,
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not prepare downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                    continue;
                }
            };
            let _ = self.session_store.write().prepare_workflow_turn(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                format!("workflow-ack:{}", dispatch.node_run.id()),
                prompt_text.clone(),
                control_mailbox,
                Some(handoff_payloads_json),
            );
            let claim_id = match self
                .workflow_dispatch_claim_id(session_id, dispatch.node_run.agent_id())
            {
                Ok(claim_id) => claim_id,
                Err(error) => {
                    self.record_notice(
                            session_id,
                            None,
                            self.attachment_store.list_session_attachment_ids(session_id),
                            format!(
                                "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                                dispatch.node_run.node_id(),
                                error
                            ),
                        );
                    continue;
                }
            };
            match self.acquire_workflow_node_workspace_claim(
                session_id,
                &claim_id,
                dispatch.node_run.agent_id(),
                workflow_run_id,
                dispatch.node_run.id(),
            ) {
                Ok(()) => {
                    let _ = self
                        .session_store
                        .write()
                        .ready_workflow_node_after_workspace_claim(
                            session_id,
                            workflow_run_id,
                            dispatch.node_run.id(),
                        );
                }
                Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                    let _ = self
                        .session_store
                        .write()
                        .block_workflow_node_on_workspace_claim(
                            session_id,
                            workflow_run_id,
                            dispatch.node_run.id(),
                        );
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` blocked node `{}` on a workspace claim: {error}",
                            dispatch.node_run.node_id()
                        ),
                    );
                    continue;
                }
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                    continue;
                }
            }
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run_id),
                dispatch.node_run.agent_id(),
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run_id, dispatch.node_run.id());
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                },
                workflow_run_id,
                dispatch.node_run.id(),
            ) {
                Ok(dispatches) => prepared.extend(dispatches),
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                }
            }
        }
        prepared
    }

    pub(super) fn workflow_turn_prompt_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        node_id: &str,
        endpoint_prompt: &str,
        handoff_payloads_json: Option<&str>,
        control_mailbox: Option<&str>,
    ) -> Result<String, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_run.workflow_id())?;
        let node = workflow.node(node_id);
        let base_directory =
            self.workflow_runtime_base_directory(session_id, workflow_run_id, workflow_node_run_id);
        let instruction_ref = self.workflow_node_instruction_reference(
            base_directory.as_ref(),
            workflow_run_id,
            node_id,
            node.and_then(|node| node.instructions()),
        );
        let turn_index = workflow_run
            .node_runs()
            .iter()
            .filter(|node_run| node_run.node_id() == node_id)
            .count() as u32;
        Ok(
            crate::scheduler::prompt_injection::build_workflow_turn_prompt(
                crate::scheduler::prompt_injection::WorkflowPromptInjectionContext {
                    endpoint_prompt: endpoint_prompt.to_string(),
                    workflow_prompt: workflow_run
                        .invocation_prompt()
                        .map(str::to_string)
                        .unwrap_or_default(),
                    node_instructions: node
                        .and_then(|node| node.instructions())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or("No node-specific instructions were configured.")
                        .to_string(),
                    instruction_ref,
                    handoff_payloads_json: handoff_payloads_json.map(str::to_string),
                    outgoing_edge_contracts: self.workflow_outgoing_edge_contracts_text(
                        session_id,
                        workflow_run_id,
                        node_id,
                    ),
                    control_mailbox: control_mailbox.map(str::to_string),
                    delivery_token: format!("workflow-ack:{workflow_node_run_id}"),
                    node_turn: node.map(|node| {
                        crate::scheduler::prompt_injection::WorkflowNodeTurnPromptContext {
                            turn_index,
                            max_turns: node.max_turns(),
                            can_complete_workflow_run: node.can_complete_workflow_run(),
                            can_emit_intermediate_output: node.can_emit_intermediate_run_output(),
                        }
                    }),
                    base_directory,
                },
            ),
        )
    }

    pub(super) fn workflow_runtime_base_directory(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Option<PathBuf> {
        let session = self.session_store.get_session(session_id).ok()?;
        let workflow_run = session.workflow_run(workflow_run_id)?;
        let node_run = workflow_run
            .node_runs()
            .iter()
            .find(|candidate| candidate.id() == workflow_node_run_id)?;
        self.provider_store
            .get_latest_run_for_agent(session_id, node_run.agent_id())
            .and_then(|run| run.working_directory().cloned())
            .or_else(|| {
                let worktree = PathBuf::from(session.worktree_id());
                if worktree.is_absolute() {
                    Some(worktree)
                } else {
                    std::env::current_dir().ok().map(|cwd| cwd.join(worktree))
                }
            })
    }

    pub(super) fn workflow_node_instruction_reference(
        &self,
        base_directory: Option<&PathBuf>,
        workflow_run_id: &str,
        node_id: &str,
        node_instructions: Option<&str>,
    ) -> Option<String> {
        let root = base_directory?
            .join(".arroba")
            .join("workflow-runtime")
            .join("kernel")
            .join(workflow_run_id)
            .join("workflow-instructions");
        let path = root.join(format!("node-{node_id}.md"));
        if !path.exists() || node_instructions.is_some() {
            if let Err(error) = std::fs::create_dir_all(&root) {
                tracing::debug!(
                    ?error,
                    "Failed to create workflow instruction directory at {:?}",
                    root
                );
                return None;
            }
            let content = node_instructions
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!(
                        "# Workflow Node Instructions\n\nThis file is daemon-managed. Update node instructions through workflow configuration tooling.\n\nNode: {node_id}\n"
                    )
                });
            if let Err(error) = std::fs::write(&path, content) {
                tracing::debug!(
                    ?error,
                    "Failed to write workflow instruction file at {:?}",
                    path
                );
                return None;
            }
        }
        Some(path.to_string_lossy().to_string())
    }

    pub(super) fn workflow_retry_blocked_claims(&self) -> WorkflowPromptDispatches {
        let mut blocked = Vec::new();
        for session in self.session_store.read().list_sessions() {
            for workflow_run in session.workflow_runs() {
                for node_run in workflow_run.node_runs() {
                    if node_run.status()
                        != crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
                    {
                        continue;
                    }
                    let Some(prompt) = node_run
                        .turn_envelope()
                        .and_then(|envelope| envelope.rendered_prompt())
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    blocked.push((
                        session.id().to_string(),
                        workflow_run.id().to_string(),
                        node_run.id().to_string(),
                        node_run.agent_id().to_string(),
                        node_run.node_id().to_string(),
                        prompt,
                    ));
                }
            }
        }
        let mut dispatches = WorkflowPromptDispatches::default();
        for (session_id, workflow_run_id, workflow_node_run_id, agent_id, node_id, prompt_text) in
            blocked
        {
            let claim_id = match self.workflow_dispatch_claim_id(&session_id, &agent_id) {
                Ok(claim_id) => claim_id,
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                    continue;
                }
            };
            match self.acquire_workflow_node_workspace_claim(
                &session_id,
                &claim_id,
                &agent_id,
                &workflow_run_id,
                &workflow_node_run_id,
            ) {
                Ok(()) => {
                    let _ = self
                        .session_store
                        .write()
                        .ready_workflow_node_after_workspace_claim(
                            &session_id,
                            &workflow_run_id,
                            &workflow_node_run_id,
                        );
                }
                Err(DaemonError::WorkspaceClaimConflict { .. }) => continue,
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                    continue;
                }
            }
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(&workflow_run_id),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(&workflow_run_id, &workflow_node_run_id);
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.clone(),
                    prompt,
                    force_queue: false,
                },
                &workflow_run_id,
                &workflow_node_run_id,
            ) {
                Ok(prepared) => dispatches.extend(prepared),
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                }
            }
        }
        dispatches
    }

    pub(super) fn workflow_dispatch_has_all_inputs(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        dispatch: &crate::session::WorkflowDispatch,
    ) -> bool {
        let workflow_id = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run.workflow_id().to_string(),
            Err(_) => return true,
        };
        let workflow = match self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, &workflow_id)
        {
            Ok(workflow) => workflow,
            Err(_) => return true,
        };
        let expected = workflow
            .edges()
            .iter()
            .filter(|edge| edge.to_node_id() == dispatch.node_run.node_id())
            .map(|edge| edge.from_node_id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        if expected.len() <= 1 {
            return true;
        }
        let run = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run,
            Err(_) => return true,
        };
        let run_node_by_id = run
            .node_runs()
            .iter()
            .map(|node_run| (node_run.id().to_string(), node_run.node_id().to_string()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let delivered = dispatch
            .messages
            .iter()
            .filter_map(|message| message.source_node_run_id())
            .filter_map(|node_run_id| run_node_by_id.get(node_run_id).cloned())
            .collect::<std::collections::BTreeSet<_>>();
        expected.is_subset(&delivered)
    }

    pub(super) fn workflow_maybe_start_next_queued_launch(&self, session_id: &str) {
        let queued_launch = match self
            .session_store
            .write()
            .dequeue_next_workflow_launch(session_id)
        {
            Ok(Some(queued_launch)) => queued_launch,
            Ok(None) => return,
            Err(error) => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!("Failed to start queued workflow launch: {error}"),
                );
                return;
            }
        };
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self
                .session_store
                .write()
                .mark_workflow_watchdog_pending_started(session_id, watchdog_id);
        }
        match self.workflow_invoke_queued_launch(session_id, queued_launch.clone()) {
            Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                    workflow_run,
                    workflow,
                    endpoint,
                },
                _dispatches,
            )) => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Started queued workflow run `{}` for workflow `{}` endpoint `{}`.",
                        workflow_run.id(),
                        workflow.id(),
                        endpoint.id()
                    ),
                );
            }
            Ok((crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued { .. }, _)) => {}
            Err(error) => {
                if let Some(watchdog_id) = queued_launch.watchdog_id() {
                    let _ = self.session_store.write().mark_workflow_watchdog_failed(
                        session_id,
                        watchdog_id,
                        error.to_string(),
                    );
                }
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Queued workflow launch `{}` failed: {error}",
                        queued_launch.id()
                    ),
                );
            }
        }
    }

    pub(super) fn workflow_resume_run(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<(crate::session::WorkflowRun, WorkflowPromptDispatches), DaemonError> {
        let workflow_run = self
            .session_store
            .write()
            .resume_workflow_run(session_id, workflow_run_ref)?;
        let resumable = workflow_run
            .node_runs()
            .iter()
            .filter_map(|node_run| {
                let prompt = node_run.turn_envelope()?.rendered_prompt()?.to_string();
                Some((
                    node_run.id().to_string(),
                    node_run.agent_id().to_string(),
                    prompt,
                ))
            })
            .collect::<Vec<_>>();
        let mut dispatches = WorkflowPromptDispatches::default();
        for (workflow_node_run_id, agent_id, prompt_text) in resumable {
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run.id(), &workflow_node_run_id);
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                },
                workflow_run.id(),
                &workflow_node_run_id,
            ) {
                Ok(prepared) => dispatches.extend(prepared),
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{}` could not resume node prompt: {}",
                            workflow_run.id(),
                            error
                        ),
                    );
                }
            }
        }
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((workflow_run, dispatches))
    }

    pub(super) fn dispatch_workflow_runtime_tool_call(
        &self,
        tool_name: String,
        arguments: serde_json::Value,
        context: crate::transport::runtime_tools::WorkflowRuntimeToolContext,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let canonical_tool_name = tool_name
            .strip_prefix("arroba_")
            .unwrap_or(tool_name.as_str())
            .to_string();
        let arguments_json = serde_json::to_string(&arguments)
            .unwrap_or_else(|_| String::from("<unserializable runtime tool arguments>"));
        let mut dispatches = WorkflowPromptDispatches::default();
        let result = match canonical_tool_name.as_str() {
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::AckWorkflowTurnArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_ack_workflow_turn",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let workflow_run_id = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?
                    .id()
                    .to_string();
                let workflow_run = self.session_store.write().ack_workflow_turn(
                    &context.session_id,
                    &workflow_run_id,
                    &context.workflow_node_run_id,
                    &args.delivery_token,
                )?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "workflow_run_id": workflow_run.id(),
                        "workflow_node_run_id": context.workflow_node_run_id,
                        "state": "acknowledged",
                        "next_action": "Continue this same workflow turn. This acknowledgement is not the final answer. If this turn requires final workflow run output, call validate_and_submit_workflow_run_output before stopping; otherwise emit the required final fenced json block before stopping.",
                    }),
                })
            }
            crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ValidateWorkflowOutputArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_validate_workflow_output",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                if !context.allowed_output_schema_refs.is_empty()
                    && !context
                        .allowed_output_schema_refs
                        .iter()
                        .any(|schema_ref| schema_ref == &args.output_schema_ref)
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_validate_workflow_output",
                        message: format!(
                            "schema ref `{}` is not allowed for workflow node run `{}`",
                            args.output_schema_ref, context.workflow_node_run_id
                        ),
                    });
                }
                let warning = crate::transport::runtime_tools::validate_workflow_output_schema(
                    &args.output_schema_ref,
                    &args.output_json,
                )
                .err();
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "valid": warning.is_none(),
                        "warning": warning,
                        "next_action": if warning.is_none() {
                            "Validation passed. Now finish this same workflow turn by emitting exactly one final fenced json block and then stop."
                        } else {
                            "Validation failed or warned. Revise the output and call validate_workflow_output again before finalizing."
                        },
                    }),
                })
            }
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
            | crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL =>
            {
                let is_final = canonical_tool_name
                    == crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL;
                if is_final && !context.can_complete_workflow_run {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_validate_and_submit_workflow_run_output",
                        message:
                            "current workflow node run is not allowed to complete the workflow run"
                                .to_string(),
                    });
                }
                if !is_final && !context.can_emit_intermediate_workflow_run_output {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_validate_and_submit_intermediate_workflow_run_output",
                        message:
                            "current workflow node run is not allowed to emit intermediate workflow run output"
                                .to_string(),
                    });
                }
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ValidateAndSubmitWorkflowRunOutputArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: if is_final {
                        "runtime_tool_validate_and_submit_workflow_run_output"
                    } else {
                        "runtime_tool_validate_and_submit_intermediate_workflow_run_output"
                    },
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let workflow_run_id = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?
                    .id()
                    .to_string();
                let schema_ref = if is_final {
                    context.workflow_run_output_schema_ref.as_deref()
                } else {
                    context.workflow_intermediate_output_schema_ref.as_deref()
                };
                let warning = schema_ref.and_then(|schema_ref| {
                    crate::transport::runtime_tools::validate_workflow_output_schema(
                        schema_ref,
                        &args.workflow_output_json,
                    )
                    .err()
                });
                let output = crate::session::WorkflowOutputPayload::new(
                    args.workflow_output_json,
                    Vec::<crate::session::WorkflowArtifactRef>::new(),
                );
                let workflow_run = if is_final {
                    self.session_store.write().submit_workflow_run_final_output(
                        &context.session_id,
                        &workflow_run_id,
                        &context.workflow_node_run_id,
                        output,
                        warning.is_none(),
                        warning.clone(),
                    )?
                } else {
                    self.session_store.write().submit_workflow_run_intermediate_output(
                        &context.session_id,
                        &workflow_run_id,
                        &context.workflow_node_run_id,
                        output,
                        warning.is_none(),
                        warning.clone(),
                    )?
                };
                if !is_final && warning.is_none() {
                    let update = self
                        .session_store
                        .write()
                        .release_workflow_intermediate_output_downstream(
                            &context.session_id,
                            &workflow_run_id,
                            &context.workflow_node_run_id,
                        )?;
                    for warning in &update.validation_warnings {
                        self.workflow_record_failure(
                            &context.session_id,
                            &workflow_run_id,
                            &crate::session::WorkflowFailureEvent::new(
                                crate::session::WorkflowFailureKind::OutputValidationFailed,
                                &context.workflow_node_run_id,
                                vec![warning.edge_id.clone()],
                                warning.message.clone(),
                            ),
                        );
                        self.record_notice(
                            &context.session_id,
                            None,
                            self.attachment_store
                                .list_session_attachment_ids(&context.session_id),
                            format!(
                                "Workflow output validation warning on edge `{}`: {}",
                                warning.edge_id, warning.message
                            ),
                        );
                    }
                    dispatches.extend(self.workflow_prepare_dispatches(
                        &context.session_id,
                        &workflow_run_id,
                        &update.dispatches,
                    ));
                    let _ = self.session_snapshot(&context.session_id);
                }
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "submitted": true,
                        "valid": warning.is_none(),
                        "warning": warning,
                        "workflow_run_id": workflow_run.id(),
                        "workflow_node_run_id": context.workflow_node_run_id,
                        "next_action": if is_final {
                            "Final workflow run output was submitted. If it is valid with no warning, finish this same workflow turn now."
                        } else {
                            "Intermediate workflow run output was submitted. Continue this same workflow turn and emit the required final fenced json block before stopping."
                        },
                    }),
                })
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_READ_TOOL => {
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
                let console = self
                    .session_store
                    .read()
                    .read_workflow_console(&context.session_id, workflow_run.workflow_id())?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "workflow_id": console.workflow_id(),
                        "entries": console.entries().iter().map(|entry| serde_json::json!({
                            "timestamp_ms": entry.timestamp_ms(),
                            "source_node_run_id": entry.source_node_run_id(),
                            "source_agent_id": entry.source_agent_id(),
                            "text": entry.text(),
                        })).collect::<Vec<_>>(),
                    }),
                })
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_WRITE_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::WorkflowConsoleWriteArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_workflow_console_write",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
                let source_agent_id = self.workflow_node_agent_id(
                    &context.session_id,
                    &context.workflow_run_ref,
                    &context.workflow_node_run_id,
                );
                let entry = self.session_store.write().append_workflow_console_entry(
                    &context.session_id,
                    workflow_run.workflow_id(),
                    Some(context.workflow_node_run_id.clone()),
                    source_agent_id,
                    &args.text,
                )?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "timestamp_ms": entry.timestamp_ms(),
                        "source_node_run_id": entry.source_node_run_id(),
                        "source_agent_id": entry.source_agent_id(),
                        "text": entry.text(),
                    }),
                })
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_CLEAR_TOOL => {
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
                let console = self
                    .session_store
                    .write()
                    .clear_workflow_console(&context.session_id, workflow_run.workflow_id())?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "cleared": true,
                        "workflow_id": console.workflow_id(),
                    }),
                })
            }
            other => Err(DaemonError::LocalTransport {
                operation: "dispatch_runtime_tool_call",
                message: format!("unsupported runtime tool `{other}`"),
            }),
        };
        let result_json = match &result {
            Ok(result) => Some(
                serde_json::to_string(&result.payload)
                    .unwrap_or_else(|_| String::from("<unserializable runtime tool result>")),
            ),
            Err(error) => Some(serde_json::json!({"error": error.to_string()}).to_string()),
        };
        let ok = result.as_ref().map(|entry| entry.ok).unwrap_or(false);
        let _ = self
            .session_store
            .write()
            .record_workflow_runtime_tool_call(
                &context.session_id,
                &context.workflow_node_run_id,
                crate::session::WorkflowRuntimeToolCallEvent::new(
                    canonical_tool_name,
                    arguments_json,
                    result_json,
                    ok,
                ),
            );
        let _ = self.session_snapshot(&context.session_id);
        result.map(|result| (result, dispatches))
    }

    pub(super) fn workflow_node_agent_id(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
        workflow_node_run_id: &str,
    ) -> Option<String> {
        self.session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_ref)
            .ok()
            .and_then(|workflow_run| {
                workflow_run
                    .node_runs()
                    .iter()
                    .find(|node_run| node_run.id() == workflow_node_run_id)
                    .map(|node_run| node_run.agent_id().to_string())
            })
    }

    pub(super) fn workflow_tool_context(
        &self,
        session_id: String,
        workflow_run_ref: String,
        workflow_node_run_id: String,
        delivery_token: Option<String>,
    ) -> Result<crate::transport::runtime_tools::WorkflowRuntimeToolContext, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&session_id, &workflow_run_ref)?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&session_id, workflow_run.workflow_id())?;
        let node_id = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .map(|node_run| node_run.node_id().to_string())
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.clone(),
                workflow_id: workflow.id().to_string(),
                reference: workflow_node_run_id.clone(),
                message: "workflow node run was not found while resolving runtime tool scope",
            })?;
        let allowed_output_schema_refs = workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == node_id)
            .filter_map(|edge| edge.output_schema_ref().map(str::to_string))
            .collect();
        let node = workflow.node(&node_id);
        let can_complete_workflow_run = node.is_some_and(|node| node.can_complete_workflow_run());
        let can_emit_intermediate_workflow_run_output =
            node.is_some_and(|node| node.can_emit_intermediate_run_output());
        let workflow_intermediate_output_schema_ref = node
            .and_then(|node| node.intermediate_output_schema_ref())
            .map(str::to_string)
            .or_else(|| {
                workflow
                    .intermediate_output_schema_ref()
                    .map(str::to_string)
            });
        Ok(
            crate::transport::runtime_tools::WorkflowRuntimeToolContext {
                session_id,
                workflow_run_ref,
                workflow_node_run_id,
                delivery_token,
                allowed_output_schema_refs,
                workflow_run_output_schema_ref: workflow
                    .run_output_schema_ref()
                    .map(str::to_string),
                workflow_intermediate_output_schema_ref,
                can_complete_workflow_run,
                can_emit_intermediate_workflow_run_output,
            },
        )
    }

    pub(super) fn resolve_owned_authenticated_workflow_turn(
        &self,
        session_id: &str,
        candidate_agent_ids: &[String],
        delivery_token: Option<&str>,
    ) -> Result<(String, String), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let agent_matches = |agent_id: &str| {
            candidate_agent_ids.is_empty()
                || candidate_agent_ids
                    .iter()
                    .any(|candidate| candidate == agent_id)
        };
        for agent_id in candidate_agent_ids {
            if let Some(prompt) = self
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
            {
                let (Some(workflow_run_ref), Some(workflow_node_run_id)) =
                    (prompt.workflow_run_id(), prompt.workflow_node_run_id())
                else {
                    continue;
                };
                let matches_token = delivery_token.is_none_or(|requested| {
                    session
                        .workflow_runs()
                        .iter()
                        .find(|workflow_run| workflow_run.id() == workflow_run_ref)
                        .and_then(|workflow_run| {
                            workflow_run
                                .node_runs()
                                .iter()
                                .find(|node_run| node_run.id() == workflow_node_run_id)
                        })
                        .and_then(|node_run| node_run.turn_envelope())
                        .is_some_and(|envelope| envelope.delivery_token() == requested)
                });
                if matches_token {
                    return Ok((
                        workflow_run_ref.to_string(),
                        workflow_node_run_id.to_string(),
                    ));
                }
            }
        }
        let mut running_turns = session
            .workflow_runs()
            .iter()
            .flat_map(|workflow_run| {
                workflow_run.node_runs().iter().filter_map(|node_run| {
                    let envelope = node_run.turn_envelope()?;
                    if node_run.status() != crate::session::WorkflowNodeRunStatus::Running
                        || !matches!(
                            envelope.state(),
                            crate::session::WorkflowTurnRuntimeState::Prepared
                                | crate::session::WorkflowTurnRuntimeState::Dispatched
                                | crate::session::WorkflowTurnRuntimeState::Acknowledged
                        )
                    {
                        return None;
                    }
                    if !agent_matches(node_run.agent_id()) {
                        return None;
                    }
                    if delivery_token
                        .is_some_and(|requested| envelope.delivery_token() != requested)
                    {
                        return None;
                    }
                    Some((workflow_run.id().to_string(), node_run.id().to_string()))
                })
            })
            .collect::<Vec<_>>();
        match running_turns.len() {
            1 => Ok(running_turns.remove(0)),
            0 => Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: "no active workflow turn for authenticated provider run".to_string(),
            }),
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: "multiple workflow turns matched the authenticated provider run"
                    .to_string(),
            }),
        }
    }
}

pub(super) struct OwnedProviderRunExit {
    pub(super) ended_run: crate::provider::RuntimeProviderRun,
    pub(super) already_ended: bool,
}

pub(super) struct OwnedPromptCompletion {
    pub(super) completion: crate::session::PromptCompletion,
    pub(super) released_claim: bool,
    pub(super) dispatch: Option<crate::app::KernelPromptDispatch>,
}

pub(super) struct OwnedPromptCancellation {
    pub(super) cancellation: crate::session::PromptCancellation,
    pub(super) released_claim: bool,
    pub(super) dispatch: Option<crate::app::KernelPromptDispatch>,
}
