use super::DaemonApp;
use crate::error::DaemonError;
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome, RuntimeSession};

impl DaemonApp {
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
        let agent_id = prompt.target_agent_id().to_string();
        let session = self.sessions.get_session(session_id)?;
        if !session.has_attachment(&source_attachment_id)
            && !crate::scheduler::runtime::is_workflow_prompt_attachment(&source_attachment_id)
        {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: source_attachment_id,
            });
        }
        let outcome =
            self.prompt_state_owner
                .submit_prepared_prompt(&session, prompt, force_queue)?;
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

    pub(crate) fn prompt_owner_activate_next_queued_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let next = self.prompt_state_owner.activate_next_queued_prompt(
            &session,
            agent_id,
            expected_prompt_id,
        )?;
        self.mirror_prompt_owner_agent_state(session_id, agent_id)?;
        Ok(next)
    }

    pub(crate) fn prompt_owner_activate_prompt(
        &mut self,
        session_id: &str,
        prompt: PromptQueueItem,
    ) -> Result<PromptQueueItem, DaemonError> {
        let agent_id = prompt.target_agent_id().to_string();
        let session = self.sessions.get_session(session_id)?;
        let active = self.prompt_state_owner.activate_prompt(&session, prompt);
        self.mirror_prompt_owner_agent_state(session_id, &agent_id)?;
        Ok(active)
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
        let prompt = PromptQueueItem::new(
            self.sessions.reserve_prompt_id(),
            source_attachment_id,
            target_agent_id,
            prompt,
            PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run_id, workflow_node_run_id);
        self.prompt_owner_submit_prepared_prompt(session_id, prompt, false)
    }

    pub(crate) fn prompt_owner_remove_session(&mut self, session_id: &str) {
        self.prompt_state_owner.remove_session(session_id);
    }

    pub(crate) fn prompt_owner_remove_queued_prompts_by_attachment(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<usize, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let removed = self
            .prompt_state_owner
            .remove_queued_prompts_by_attachment(&session, attachment_id);
        self.mirror_prompt_owner_session_state(session_id)?;
        Ok(removed)
    }

    fn mirror_prompt_owner_agent_state(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.sessions
            .mirror_agent_prompt_state(session_id, agent_id, active_prompt, queued_prompts)
    }

    fn mirror_prompt_owner_session_state(
        &mut self,
        session_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let mut agent_ids = session
            .agents()
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        agent_ids.extend(session.prompt_states().keys().cloned());
        agent_ids.sort();
        agent_ids.dedup();
        let mut mirrored_session = session;
        for agent_id in agent_ids {
            mirrored_session = self.mirror_prompt_owner_agent_state(session_id, &agent_id)?;
        }
        Ok(mirrored_session)
    }
}
