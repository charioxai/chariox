//! Provider-run state reconciliation and focus/parking mutations.
//!
//! This module keeps the owned provider registry coherent with sessions, including active-run
//! focus, park/unpark transitions, liveness reconciliation, and provider-output bookkeeping.

use super::owned::OwnedProviderRunExit;
use super::*;

impl KernelRuntimeOwnedState {
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

        Ok(self.prompt_state_owner.has_any_active_prompt(&session)
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
                && !self.provider_run_has_active_prompt(session_id, &active_run)?
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
                let active_prompt_agent_id =
                    self.prompt_state_owner.active_prompt_agent_id(&session);
                let has_active_prompt = self.prompt_state_owner.has_any_active_prompt(&session);
                let has_processing_agent =
                    session.agents().iter().any(|agent| agent.is_processing());
                if !has_active_prompt {
                    let current_active_run_id =
                        session.active_provider_run_id().map(str::to_string);
                    if let Some(current_active_run_id) = current_active_run_id.as_deref() {
                        let active_run = self.provider_store.get_run(current_active_run_id)?;
                        if active_run.agent_instance_id() != Some(focused_agent_id.as_str())
                            && active_run.state() == crate::provider::ProviderRunState::Running
                            && !self.provider_run_has_active_prompt(session_id, &active_run)?
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
                if has_active_prompt {
                    if let Some(projected_agent_id) = active_prompt_agent_id.as_deref() {
                        self.project_active_provider_run_for_agent(session_id, projected_agent_id)?;
                    }
                } else if has_processing_agent {
                    self.project_active_provider_run_for_agent(session_id, &focused_agent_id)?;
                } else {
                    self.sync_active_provider_run_for_agent(session_id, &focused_agent_id)?;
                }
            } else {
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            return Ok(());
        }

        if self.prompt_state_owner.has_any_active_prompt(&session)
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
            .and_then(|run| match run.state() {
                crate::provider::ProviderRunState::Running
                | crate::provider::ProviderRunState::Starting => Some(run.id().to_string()),
                crate::provider::ProviderRunState::Parked
                | crate::provider::ProviderRunState::Ended => None,
            });
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
                    if !self.provider_run_has_active_prompt(&session_id, &active_run)? {
                        let outcome = self
                            .provider_store
                            .park_run_provider_only(&session_id, active_run_id)?;
                        self.clear_active_provider_run_session_pointer(
                            &session_id,
                            outcome.run().id(),
                        )?;
                        self.provider_run_projection.update(outcome.into_run());
                    }
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
                let active_run = self.provider_store.get_run(active_run_id)?;
                match active_run.state() {
                    crate::provider::ProviderRunState::Running => {
                        if !self.provider_run_has_active_prompt(session_id, &active_run)? {
                            let outcome = self
                                .provider_store
                                .park_run_provider_only(session_id, active_run_id)?;
                            self.clear_active_provider_run_session_pointer(
                                session_id,
                                outcome.run().id(),
                            )?;
                            self.provider_run_projection.update(outcome.into_run());
                        }
                    }
                    crate::provider::ProviderRunState::Starting => {
                        let outcome = self
                            .provider_store
                            .terminate_run_provider_only(session_id, active_run_id)?;
                        self.clear_active_provider_run_session_pointer(
                            session_id,
                            outcome.run().id(),
                        )?;
                        self.provider_run_projection.update(outcome.into_run());
                    }
                    crate::provider::ProviderRunState::Parked
                    | crate::provider::ProviderRunState::Ended => {
                        self.session_store
                            .set_active_provider_run(session_id, None)?;
                    }
                }
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

    pub(super) fn provider_run_has_active_prompt(
        &self,
        session_id: &str,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) -> Result<bool, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(false);
        };
        let session = self.session_store.get_session(session_id)?;
        Ok(self
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, agent_id)
            .is_some())
    }

    pub(super) fn finish_provider_launch_success(
        &self,
        started: &crate::app::StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let previous_active_run = started
            .previous_active_run_id
            .as_deref()
            .and_then(|run_id| self.provider_store.get_run(run_id).ok());
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
        if let Some(previous_active_run) = previous_active_run.as_ref() {
            self.prepare_provider_switch_context_handoff(previous_active_run, &run);
        }
        self.provider_run_projection.update(run.clone());
        Ok(run)
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
}
