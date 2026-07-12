use super::DaemonApp;
use crate::error::DaemonError;
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome, RuntimeSession};

impl DaemonApp {
    pub(crate) fn promoted_prompt_source_attachment_id(
        &self,
        session_id: &str,
        source_attachment_id: &str,
    ) -> Result<String, DaemonError> {
        if crate::scheduler::runtime::is_workflow_prompt_attachment(source_attachment_id) {
            return Ok(source_attachment_id.to_string());
        }
        let session = self.sessions.get_session(session_id)?;
        if session.has_attachment(source_attachment_id) {
            return Ok(source_attachment_id.to_string());
        }
        self.attachments
            .list_session_attachment_ids(session_id)
            .into_iter()
            .next()
            .ok_or_else(|| DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: source_attachment_id.to_string(),
            })
    }

    pub(crate) fn prompt_owner_active_prompt_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        Ok(self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id))
    }

    #[cfg(test)]
    pub(crate) fn prompt_owner_active_prompt_for_agent_snapshot(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        Ok(self
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, agent_id))
    }

    pub(crate) fn prompt_owner_active_prompt_agent_id(
        &mut self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        Ok(self.prompt_state_owner.active_prompt_agent_id(&session))
    }

    pub(crate) fn prompt_owner_has_any_active_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        Ok(self.prompt_state_owner.has_any_active_prompt(&session))
    }

    #[cfg(test)]
    pub(crate) fn prompt_owner_queued_prompt_count_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<usize, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        Ok(self
            .prompt_state_owner
            .queued_prompt_count_for_agent(&session, agent_id))
    }

    pub(crate) fn prompt_owner_submit_prepared_prompt(
        &mut self,
        session_id: &str,
        prompt: PromptQueueItem,
        force_queue: bool,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        let source_attachment_id = prompt.source_attachment_id().to_string();
        let session = self.sessions.get_session(session_id)?;
        if let Some(outcome) = self
            .prompt_state_owner
            .replay_durable_submission(&session, &prompt)?
        {
            return Ok(outcome);
        }
        if !session.has_attachment(&source_attachment_id)
            && !crate::scheduler::runtime::is_workflow_prompt_attachment(&source_attachment_id)
        {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: source_attachment_id,
            });
        }
        let agent_id = prompt.target_agent_id().to_string();
        let will_queue = force_queue
            || self
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id)
                .is_some();
        let prompt = if will_queue {
            prompt
        } else {
            prompt.with_id(self.sessions.reserve_prompt_id())
        };
        let outcome =
            self.prompt_state_owner
                .submit_prepared_prompt(&session, prompt, force_queue)?;
        let agent_id = match &outcome {
            PromptSubmissionOutcome::Started { prompt }
            | PromptSubmissionOutcome::Queued { prompt } => prompt.target_agent_id().to_string(),
        };
        if matches!(outcome, PromptSubmissionOutcome::Started { .. }) {
            let prompt_sent_at_ms = crate::session::unix_epoch_ms();
            self.agents
                .note_prompt_sent_at(&agent_id, prompt_sent_at_ms)?;
            self.sessions
                .note_prompt_sent(session_id, &agent_id, prompt_sent_at_ms)?;
        }
        self.mirror_prompt_owner_agent_state(session_id, &agent_id)?;
        Ok(outcome)
    }

    pub(crate) fn prompt_owner_complete_active_prompt_only(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<PromptQueueItem, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        self.mirror_prompt_owner_agent_state(session_id, agent_id)?;
        Ok(completed)
    }

    pub(crate) fn prompt_owner_cancel_active_prompt_only(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<PromptQueueItem, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let cancelled = self
            .prompt_state_owner
            .cancel_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        self.mirror_prompt_owner_agent_state(session_id, agent_id)?;
        Ok(cancelled)
    }

    pub(crate) fn prompt_owner_begin_cancelling_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<PromptQueueItem, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .begin_cancelling_active_prompt(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        self.mirror_prompt_owner_agent_state(session_id, agent_id)?;
        Ok(prompt)
    }

    pub(crate) fn prompt_owner_mark_active_prompt_running(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<PromptQueueItem, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .mark_active_prompt_running(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        self.mirror_prompt_owner_agent_state(session_id, agent_id)?;
        Ok(prompt)
    }

    pub(crate) fn prompt_owner_finalize_active_prompt_cancellation(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<PromptQueueItem, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .finalize_active_prompt_cancellation(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        self.mirror_prompt_owner_agent_state(session_id, agent_id)?;
        Ok(prompt)
    }

    pub(crate) fn prompt_owner_peek_next_queued_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        Ok(self
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent_id))
    }

    pub(crate) fn prompt_owner_activate_next_queued_prompt_with_prompt_id(
        &mut self,
        session_id: &str,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
        prompt_id: String,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let next = self
            .prompt_state_owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                agent_id,
                expected_prompt_id,
                prompt_id,
            )?;
        self.mirror_prompt_owner_agent_state(session_id, agent_id)?;
        Ok(next)
    }

    #[cfg(test)]
    pub(crate) fn prompt_owner_activate_prompt(
        &mut self,
        session_id: &str,
        prompt: PromptQueueItem,
    ) -> Result<PromptQueueItem, DaemonError> {
        let agent_id = prompt.target_agent_id().to_string();
        let session = self.sessions.get_session(session_id)?;
        let active = self.prompt_state_owner.activate_prompt(&session, prompt)?;
        self.mirror_prompt_owner_agent_state(session_id, &agent_id)?;
        Ok(active)
    }

    pub(crate) fn prompt_owner_sync_external_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        active_prompt: Option<PromptQueueItem>,
    ) -> Result<bool, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let changed =
            self.prompt_state_owner
                .sync_external_active_prompt(&session, agent_id, active_prompt);
        if changed {
            self.mirror_prompt_owner_agent_state(session_id, agent_id)?;
        }
        Ok(changed)
    }

    pub(crate) fn prompt_owner_submit_workflow_prompt(
        &mut self,
        session_id: &str,
        source_attachment_id: &str,
        target_agent_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        prompt: impl Into<String>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        let (visible_user_prompt, hidden_system_context) =
            split_workflow_prompt_for_hidden_context(prompt.into());
        let prompt = PromptQueueItem::new(
            self.sessions.reserve_prompt_id(),
            source_attachment_id,
            target_agent_id,
            visible_user_prompt,
            PromptStatus::Queued,
        )
        .with_hidden_system_context(hidden_system_context)
        .with_workflow_context(workflow_run_id, workflow_node_run_id);
        self.prompt_owner_submit_prepared_prompt(session_id, prompt, false)
    }

    pub(crate) fn prompt_owner_remove_session(&mut self, session_id: &str) {
        self.prompt_state_owner.remove_session(session_id);
    }

    fn mirror_prompt_owner_agent_state(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        let session = self.sessions.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        crate::durable_prompt_state::append_durable_prompt_state_event(
            &self.durable_state,
            &session,
            agent_id,
        )?;
        self.provider_process_projection.invalidate();
        self.refresh_prompt_owner_session_projection(session_id)?;
        Ok(session)
    }

    fn refresh_prompt_owner_session_projection(&self, session_id: &str) -> Result<(), DaemonError> {
        let mut session = self.sessions.get_session(session_id)?;
        let agents = self.agents.get_session_agents(session_id);
        session.set_agents(agents);
        self.project_session_runtime_view(&mut session);
        self.update_session_projection(session);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::KernelSessionService;
    use crate::session::{CreateSessionRequest, PromptOrigin};

    #[test]
    fn external_active_prompt_sync_refreshes_projected_session_state() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let external_prompt = PromptQueueItem::external_observed_running(
            "codex",
            "thread-1",
            "user-1",
            agent.id(),
            "external prompt",
        );

        let changed = app
            .prompt_owner_sync_external_active_prompt(
                session.id(),
                agent.id(),
                Some(external_prompt),
            )
            .expect("external prompt should sync");

        assert!(changed);
        let projected = app
            .session_state_projection_store()
            .get(session.id())
            .expect("session projection should refresh");
        assert_eq!(projected.agents().len(), 1);
        let active_prompt = projected
            .active_prompt_for_agent(agent.id())
            .expect("external active prompt should be projected");
        assert_eq!(active_prompt.prompt_origin(), PromptOrigin::External);
        assert_eq!(active_prompt.prompt(), "external prompt");
        let activity = app
            .agent_runtime_projection_store()
            .get(agent.id())
            .expect("agent runtime projection should refresh");
        assert!(activity.active_prompt.is_some());
    }

    #[test]
    fn update_session_projection_projects_prompt_owner_when_session_mirror_is_stale() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let external_prompt = PromptQueueItem::external_observed_running(
            "codex",
            "thread-stale-projection",
            "user-stale-projection",
            agent.id(),
            "external prompt from prompt owner",
        );
        app.prompt_owner_sync_external_active_prompt(
            session.id(),
            agent.id(),
            Some(external_prompt),
        )
        .expect("external prompt should sync");
        app.sessions_mut()
            .mirror_agent_prompt_state(
                session.id(),
                agent.id(),
                None,
                std::collections::VecDeque::new(),
            )
            .expect("test drift should clear the session prompt mirror");
        let mut stale_session = app
            .sessions()
            .get_session(session.id())
            .expect("session should load");
        stale_session.set_agents(app.agents().get_session_agents(session.id()));
        assert!(
            stale_session.active_prompt_for_agent(agent.id()).is_none(),
            "stale input session should not expose the active prompt"
        );

        app.update_session_projection(stale_session);

        let projected = app
            .session_state_projection_store()
            .get(session.id())
            .expect("session projection should exist");
        let projected_prompt = projected
            .active_prompt_for_agent(agent.id())
            .expect("session projection should use prompt owner state");
        assert_eq!(projected_prompt.prompt_origin(), PromptOrigin::External);
        assert_eq!(
            projected_prompt.prompt(),
            "external prompt from prompt owner"
        );
        let activity = app
            .agent_runtime_projection_store()
            .get(agent.id())
            .expect("agent runtime projection should refresh");
        assert_eq!(
            activity
                .active_prompt
                .as_ref()
                .map(|prompt| prompt.prompt()),
            Some("external prompt from prompt owner")
        );
    }

    #[test]
    fn prompt_submission_refreshes_projected_prompt_timestamp() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-1",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let prompt = PromptQueueItem::new(
            app.sessions.reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "projected prompt timestamp",
            PromptStatus::Queued,
        );

        app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should submit");

        let projected = app
            .session_state_projection_store()
            .get(session.id())
            .expect("session projection should refresh");
        assert!(projected.last_prompt_sent_at_ms().is_some());
        assert!(projected.active_prompt_for_agent(agent.id()).is_some());
        let projected_agent = projected
            .agents()
            .iter()
            .find(|projected_agent| projected_agent.id() == agent.id())
            .expect("projected session should include agent");
        assert!(projected_agent.last_prompt_sent_at_ms().is_some());
    }
}

fn split_workflow_prompt_for_hidden_context(prompt: String) -> (String, String) {
    const WORKFLOW_MARKER: &str = "Workflow-level prompt:\n";
    if let Some(index) = prompt.find(WORKFLOW_MARKER) {
        let visible = prompt[..index].to_string();
        let hidden = prompt[index..].to_string();
        return (visible, strip_native_hidden_markers(hidden));
    }
    (prompt, String::new())
}

fn strip_native_hidden_markers(value: String) -> String {
    value
        .replace(crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_START, "")
        .replace(crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_END, "")
        .trim()
        .to_string()
}
