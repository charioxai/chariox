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
}
