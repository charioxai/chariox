//! Prompt queue mirror and advancement mutations.
//!
//! This module owns synchronizing prompt-owner state back into sessions and advancing queued
//! prompts onto an existing provider run.

use super::*;

impl KernelRuntimeOwnedState {
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
            prompt_id: started_next.id().to_string(),
            source_attachment_id: started_next.source_attachment_id().to_string(),
            prompt: started_next.prompt().to_string(),
            hidden_system_context: started_next.hidden_system_context().to_string(),
            attachments: started_next.attachments().to_vec(),
        }))
    }
}
