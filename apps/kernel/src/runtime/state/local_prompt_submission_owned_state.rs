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

        let force_queue = prepared.force_queue || provider_run_is_starting;
        let will_queue = force_queue || queued_while_active;
        let prompt = if will_queue {
            prepared.prompt.clone()
        } else {
            prepared
                .prompt
                .clone()
                .with_id(self.session_store.reserve_prompt_id())
        };
        let outcome =
            self.prompt_state_owner
                .submit_prepared_prompt(&session, prompt, force_queue)?;
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let prompt_sent_at_ms = crate::session::unix_epoch_ms();
        if let crate::session::PromptSubmissionOutcome::Started { prompt } = &outcome {
            self.append_user_prompt_history(
                &session_id,
                prompt.source_attachment_id(),
                prompt.target_agent_id(),
                prompt.prompt(),
                prompt.attachments(),
                Some(prompt.id()),
                prompt.workflow_run_id(),
                prompt.workflow_node_run_id(),
            )?;
            let provider_run_id =
                provider_run_id
                    .as_deref()
                    .ok_or_else(|| DaemonError::NoActiveProviderRun {
                        session_id: session_id.clone(),
                    })?;
            if let Ok(provider_run) = self.provider_store.get_run(provider_run_id) {
                self.capture_git_turn_snapshot_for_started_prompt(
                    &session,
                    &target_agent_id,
                    &provider_run,
                    prompt,
                    Some(prompt_sent_at_ms),
                );
            }
        }
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.mirror_prompt_owner_agent_state(
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
                    prompt.id(),
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                dispatch = Some(crate::app::KernelPromptDispatch {
                    session_id: session_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: target_agent_id.clone(),
                    prompt_id: prompt.id().to_string(),
                    target_active_prompt_id: None,
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    hidden_system_context: prompt.hidden_system_context().to_string(),
                    attachments: prompt.attachments().to_vec(),
                    steering: false,
                });
                self.agent_store
                    .note_prompt_sent_at(&outcome_agent_id, prompt_sent_at_ms)?;
                self.session_store.note_prompt_sent(
                    &session_id,
                    &outcome_agent_id,
                    prompt_sent_at_ms,
                )?;
            }
            crate::session::PromptSubmissionOutcome::Queued { .. } => {}
        }
        let session = if prepared.refresh_projection {
            self.session_snapshot(&session_id)?
        } else {
            self.session_snapshot_without_projection_update(&session_id)?
        };
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome,
            session,
            dispatch,
            remote_dispatch: None,
        }))
    }

    pub(super) fn capture_git_turn_snapshot_for_started_prompt(
        &self,
        session: &crate::session::RuntimeSession,
        agent_id: &str,
        provider_run: &crate::provider::RuntimeProviderRun,
        prompt: &crate::session::PromptQueueItem,
        started_at_ms: Option<u64>,
    ) {
        if self
            .git_turn_snapshots
            .get(provider_run.id(), prompt.id())
            .is_some()
        {
            return;
        }
        let worktree_path = provider_run
            .working_directory()
            .cloned()
            .unwrap_or_else(|| std::path::PathBuf::from(session.worktree_id()));
        let context = crate::git_observer::GitTurnContext {
            session_id: session.id().to_string(),
            agent_id: agent_id.to_string(),
            provider: provider_run.provider().to_string(),
            model: provider_run.model().to_string(),
            provider_run_id: provider_run.id().to_string(),
            provider_session_id: provider_run.provider_session_id().map(str::to_string),
            prompt_id: prompt.id().to_string(),
            turn_id: prompt.id().to_string(),
            started_at_ms,
            worktree_path,
            workspace_live_sync_tracked: provider_run.tracks_workspace_live_sync(),
            machine_id: None,
            prompt_summary: crate::prompt_transcript::render_prompt_transcript(
                prompt.prompt(),
                prompt.attachments(),
            ),
        };
        if let Some(snapshot) = crate::git_observer::capture_turn_snapshot(context) {
            self.git_turn_snapshots.insert(snapshot);
        }
    }
}
