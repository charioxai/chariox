//! Local prompt admission and queueing mutations.
//!
//! This module owns admitting a prepared prompt onto a local agent/provider run and producing the
//! prompt dispatch envelope when the prompt starts immediately.

use super::*;

impl KernelRuntimeOwnedState {
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
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id,
                agent_id: target_agent_id,
            });
        }
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
            Some(prepared.prompt.id()),
            prepared.prompt.workflow_run_id(),
            prepared.prompt.workflow_node_run_id(),
        )?;
        let force_queue = prepared.force_queue || provider_run_is_starting;
        let outcome = self.prompt_state_owner.submit_prepared_prompt(
            &session,
            prepared.prompt.clone(),
            force_queue,
        )?;
        self.session_store.note_prompt_sent(&session_id)?;
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
                dispatch = Some(crate::app::KernelPromptDispatch {
                    session_id: session_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: target_agent_id.clone(),
                    prompt_id: prompt.id().to_string(),
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    hidden_system_context: prompt.hidden_system_context().to_string(),
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
}
