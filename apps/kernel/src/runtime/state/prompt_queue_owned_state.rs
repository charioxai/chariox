//! Prompt queue mirror and advancement mutations.
//!
//! This module owns synchronizing prompt-owner state back into sessions and advancing queued
//! prompts onto an existing provider run.

use super::*;

#[derive(Clone)]
pub(super) struct RemoteQueuedPromptSteerPreparation {
    pub(super) agent: crate::agent::AgentInstance,
    pub(super) remote_execution: crate::agent::RemoteAgentBinding,
    pub(super) target_active_prompt_id: String,
    pub(super) provider_run_id: String,
    pub(super) prompt: crate::session::PromptQueueItem,
}

pub(super) enum RemoteQueuedPromptSteerReceiptSettlement {
    Pending,
    Accepted,
    Rejected,
    AlreadySettled,
}

pub(super) struct RemoteQueuedPromptSteerReceiptQuery {
    pub(super) target_home_prompt_id: String,
    pub(super) worker_provider_run_id: String,
    pub(super) worker_kernel_id: String,
    pub(super) worker_machine_id: String,
    pub(super) execution_lease_id: String,
    pub(super) leased_agent_id: String,
}

struct QueuedPromptSteerContext {
    session: crate::session::RuntimeSession,
    agent: crate::agent::AgentInstance,
    active_prompt: crate::session::PromptQueueItem,
    provider_run_id: String,
    queued_prompt: crate::session::PromptQueueItem,
}

impl KernelRuntimeOwnedState {
    pub(super) fn remote_queued_prompt_steer_receipt_query(
        &self,
        session_id: &str,
        agent_id: &str,
        queued_prompt_id: &str,
    ) -> Result<Option<RemoteQueuedPromptSteerReceiptQuery>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let (_, queued_prompts) = self.prompt_state_owner.state_parts(&session, agent_id);
        let Some(prompt) = queued_prompts
            .iter()
            .find(|prompt| prompt.id() == queued_prompt_id)
        else {
            return Ok(None);
        };
        let Some((
            target_home_prompt_id,
            worker_provider_run_id,
            worker_kernel_id,
            worker_machine_id,
            execution_lease_id,
            leased_agent_id,
        )) = prompt.remote_steer_outcome_uncertainty()
        else {
            return Ok(None);
        };
        let Some(worker_provider_run_id) = worker_provider_run_id.filter(|run| !run.is_empty())
        else {
            return Err(DaemonError::LocalTransport {
                operation: "query queued steer receipt",
                message: format!(
                    "queued prompt `{queued_prompt_id}` has no exact worker provider-run identity"
                ),
            });
        };
        let agent = self.agent_store.get_agent(agent_id)?;
        if !agent.remote_execution().is_some_and(|binding| {
            binding.worker_kernel_id == worker_kernel_id
                && binding.worker_machine_id == worker_machine_id
                && binding.execution_lease_id == execution_lease_id
                && binding.leased_agent_id == leased_agent_id
        }) {
            return Err(DaemonError::LocalTransport {
                operation: "query queued steer receipt",
                message: "current remote worker binding no longer matches the durable queued steer".to_string(),
            });
        }
        Ok(Some(RemoteQueuedPromptSteerReceiptQuery {
            target_home_prompt_id,
            worker_provider_run_id,
            worker_kernel_id,
            worker_machine_id,
            execution_lease_id,
            leased_agent_id,
        }))
    }

    pub(super) fn provider_account_allows_queued_prompt_advance(
        &self,
        session_id: &str,
        agent: &crate::agent::AgentInstance,
        operation: &'static str,
    ) -> bool {
        let error = self.provider_account_profiles.require_agent_authenticated(
            &self.config_projection.snapshot(),
            agent,
            operation,
        );
        let Err(error) = error else {
            return true;
        };
        crate::logging::warn_with_fields(
            "daemon.prompt_queue",
            "deferred queued prompt because its provider account is unavailable",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent.id(),
                "error": error.to_string(),
            }),
        );
        false
    }

    pub(super) fn prompt_source_attribution(
        &self,
        prompt: &crate::session::PromptQueueItem,
    ) -> (Option<String>, Option<String>) {
        let source_attachment = self
            .attachment_store
            .get_attachment(prompt.source_attachment_id())
            .ok();
        let source_client_id = prompt.source_client_id().map(str::to_string).or_else(|| {
            source_attachment
                .as_ref()
                .map(|attachment| attachment.client_id().to_string())
        });
        let source_user_id = prompt.source_user_id().map(str::to_string).or_else(|| {
            source_attachment
                .as_ref()
                .map(|attachment| attachment.owner_user_id().to_string())
        });
        (source_client_id, source_user_id)
    }

    pub(super) fn active_prompt_source_attribution(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(Option<String>, Option<String>), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        Ok(self.prompt_source_attribution(&prompt))
    }

    fn ensure_queued_prompt_manually_mutable(
        &self,
        session: &crate::session::RuntimeSession,
        agent_id: &str,
        prompt_id: &str,
        operation: &'static str,
        action: &str,
    ) -> Result<(), DaemonError> {
        let (_, queued_prompts) = self.prompt_state_owner.state_parts(session, agent_id);
        let prompt = queued_prompts
            .iter()
            .find(|prompt| prompt.id() == prompt_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        if prompt.workflow_run_id().is_some() {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!("workflow queued prompts cannot be {action} manually"),
            });
        }
        if prompt.remote_steer_reserved() {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!("queued prompt `{prompt_id}` already has an in-flight remote steer"),
            });
        }
        Ok(())
    }

    pub(super) fn promoted_prompt_source_attachment_id(
        &self,
        session_id: &str,
        source_attachment_id: &str,
    ) -> Result<String, DaemonError> {
        let _ = self.session_store.get_session(session_id)?;
        Ok(source_attachment_id.to_string())
    }

    pub(super) fn mirror_prompt_owner_agent_state(
        &self,
        session_id: &str,
        agent_id: &str,
        active_prompt: Option<crate::session::PromptQueueItem>,
        queued_prompts: std::collections::VecDeque<crate::session::PromptQueueItem>,
    ) -> Result<(), DaemonError> {
        let activity_mutation = self.begin_managed_activity_mutation();
        let session = self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        self.persist_prompt_session_state(&session, agent_id)?;
        activity_mutation.record();
        self.provider_process_projection.invalidate();
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    pub(super) fn persist_prompt_session_state(
        &self,
        session: &crate::session::RuntimeSession,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        crate::durable_prompt_state::append_durable_prompt_state_event(
            &self.durable_state_store,
            session,
            agent_id,
        )
    }

    pub(super) fn mark_active_prompt_delivery(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        phase: crate::session::DurablePromptDeliveryPhase,
        provider_run_id: Option<String>,
        provider_session_id: Option<String>,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let previous = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let prompt = self.prompt_state_owner.mark_active_prompt_delivery(
            &session,
            agent_id,
            prompt_id,
            phase,
            provider_run_id,
            provider_session_id,
        )?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        if let Err(error) = self.mirror_prompt_owner_agent_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        ) {
            let _ = self
                .prompt_state_owner
                .replace_active_prompt_if_matches(&session, agent_id, &prompt, previous);
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            self.session_store.mirror_agent_prompt_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            )?;
            return Err(error);
        }
        if let Some(provider_run_id) = prompt.durable_delivery_provider_run_id() {
            let owns_prompt_run = self
                .provider_store
                .get_run(provider_run_id)
                .is_ok_and(|run| {
                    run.session_id() == session_id && run.agent_instance_id() == Some(agent_id)
                });
            if owns_prompt_run {
                self.structured_output_records
                    .reset_poll_failures_if_prompt_changed(
                        provider_run_id,
                        prompt.id(),
                        crate::session::unix_epoch_ms(),
                    );
            }
        }
        Ok(prompt)
    }

    pub(super) fn compare_and_mark_active_prompt_delivery_failure(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        provider_session_id: &str,
        status_transition: (crate::session::PromptStatus, crate::session::PromptStatus),
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .compare_and_mark_active_prompt_delivery_failure(
                &session,
                agent_id,
                prompt_id,
                provider_run_id,
                provider_session_id,
                status_transition,
            );
        if prompt.is_none() {
            return Ok(None);
        }
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        if let Err(error) = self.mirror_prompt_owner_agent_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        ) {
            let _ = self
                .prompt_state_owner
                .compare_and_mark_active_prompt_delivery_failure(
                    &session,
                    agent_id,
                    prompt_id,
                    provider_run_id,
                    provider_session_id,
                    (status_transition.1, status_transition.0),
                );
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            self.session_store.mirror_agent_prompt_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            )?;
            return Err(error);
        }
        Ok(prompt)
    }

    pub(super) fn restore_active_prompt_after_resume_superseded(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
        failed_provider_session_id: &str,
        current_provider_session_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let Some((previous, restored)) = self
            .prompt_state_owner
            .compare_and_restore_active_prompt_after_resume_superseded(
                &session,
                agent_id,
                prompt_id,
                provider_run_id,
                failed_provider_session_id,
                current_provider_session_id,
            )
        else {
            return Ok(None);
        };
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        if let Err(error) = self.mirror_prompt_owner_agent_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        ) {
            let _ = self
                .prompt_state_owner
                .replace_active_prompt_if_matches(&session, agent_id, &restored, previous);
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            self.session_store.mirror_agent_prompt_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            )?;
            return Err(error);
        }
        Ok(Some(restored))
    }

    pub(super) fn begin_active_prompt_recovery(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .begin_active_prompt_recovery(&session, agent_id, prompt_id)?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(prompt)
    }

    pub(super) fn mark_active_prompt_recovery_phase(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        operation_id: &str,
        phase: crate::session::DurablePromptDeliveryPhase,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self.prompt_state_owner.mark_active_prompt_recovery_phase(
            &session,
            agent_id,
            prompt_id,
            operation_id,
            phase,
        )?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(prompt)
    }

    pub(super) fn compare_and_mark_active_prompt_recovery_phase(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        operation_id: &str,
        expected_phase: crate::session::DurablePromptDeliveryPhase,
        next_phase: crate::session::DurablePromptDeliveryPhase,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .compare_and_mark_active_prompt_recovery_phase(
                &session,
                agent_id,
                prompt_id,
                operation_id,
                expected_phase,
                next_phase,
            )?;
        if prompt.is_some() {
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            if let Err(error) = self.mirror_prompt_owner_agent_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            ) {
                // The durable append is the commit boundary for this transition. Restore the
                // owner state only while it still contains our Accepted phase; a concurrent
                // delivery acknowledgement must win. Then repair the in-memory session mirror
                // from whichever owner state is current without attempting a second durable
                // write. The failed transaction left the durable phase at `expected_phase`.
                let _ = self
                    .prompt_state_owner
                    .compare_and_mark_active_prompt_recovery_phase(
                        &session,
                        agent_id,
                        prompt_id,
                        operation_id,
                        next_phase,
                        expected_phase,
                    );
                let (active_prompt, queued_prompts) =
                    self.prompt_state_owner.state_parts(&session, agent_id);
                self.session_store.mirror_agent_prompt_state(
                    session_id,
                    agent_id,
                    active_prompt,
                    queued_prompts,
                )?;
                return Err(error);
            }
        }
        Ok(prompt)
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
            self.mirror_prompt_owner_agent_state(
                session_id,
                &agent_id,
                active_prompt,
                queued_prompts,
            )?;
        }
        Ok(())
    }

    pub(super) fn remove_queued_prompts_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<usize, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let removed = self
            .prompt_state_owner
            .remove_queued_prompts_for_agent(&session, agent_id);
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(removed)
    }

    pub(super) fn take_queued_workflow_prompts_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let (_, queued_prompts) = self.prompt_state_owner.state_parts(&session, agent_id);
        let workflow_prompts = queued_prompts
            .into_iter()
            .filter(|prompt| prompt.workflow_run_id().is_some())
            .collect::<Vec<_>>();
        if workflow_prompts.is_empty() {
            return Ok(workflow_prompts);
        }
        for prompt in &workflow_prompts {
            let _ = self
                .prompt_state_owner
                .remove_queued_prompt(&session, agent_id, prompt.id());
        }
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(workflow_prompts)
    }

    pub(super) fn remove_queued_metaagent_event_prompts_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        source_attachment_id: &str,
    ) -> Result<usize, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let removed = self
            .prompt_state_owner
            .remove_queued_metaagent_event_prompts_for_agent(
                &session,
                agent_id,
                source_attachment_id,
            );
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(removed)
    }

    pub(super) fn activate_next_queued_prompt_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent_id)
            .is_none_or(|prompt| prompt.remote_steer_reserved())
        {
            return Ok(None);
        }
        let agent = self.agent_store.get_agent(agent_id)?;
        if !self.provider_account_allows_queued_prompt_advance(
            session_id,
            &agent,
            "activate queued prompt",
        ) {
            return Ok(None);
        }
        let prompt = self
            .prompt_state_owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                agent_id,
                expected_prompt_id,
                self.session_store.reserve_prompt_id(),
            )?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(prompt)
    }

    pub(super) fn advance_next_queued_prompt_dispatch(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<crate::app::KernelPromptDispatch>, DaemonError> {
        let idle_check_session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&idle_check_session, agent_id)
            .is_some()
        {
            return Ok(None);
        }
        let (session, next_prompt) = loop {
            let session = self.session_store.get_session(session_id)?;
            let Some(next_prompt) = self
                .prompt_state_owner
                .peek_next_queued_prompt(&session, agent_id)
            else {
                return Ok(None);
            };
            if next_prompt.remote_steer_reserved() {
                return Ok(None);
            }
            let workflow_is_runnable =
                next_prompt.workflow_run_id().is_none_or(|workflow_run_id| {
                    session
                        .workflow_runs()
                        .iter()
                        .find(|workflow_run| workflow_run.id() == workflow_run_id)
                        .is_some_and(|workflow_run| {
                            matches!(
                                workflow_run.status(),
                                crate::session::WorkflowRunStatus::Created
                                    | crate::session::WorkflowRunStatus::Running
                                    | crate::session::WorkflowRunStatus::Waiting
                            )
                        })
                });
            if workflow_is_runnable {
                break (session, next_prompt);
            }
            let removed =
                self.prompt_state_owner
                    .remove_queued_prompt(&session, agent_id, next_prompt.id());
            if removed.is_none() {
                continue;
            }
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            self.mirror_prompt_owner_agent_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            )?;
        };
        let agent = self.agent_store.get_agent(agent_id)?;
        if !self.provider_account_allows_queued_prompt_advance(
            session_id,
            &agent,
            "advance queued prompt",
        ) {
            return Ok(None);
        }
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "advance queued prompt",
            });
        }
        let acquired_workflow_claim =
            match self.ensure_workflow_prompt_workspace_claim(session_id, &next_prompt) {
                Ok(acquired) => acquired,
                Err(DaemonError::WorkspaceClaimConflict { .. }) => return Ok(None),
                Err(error) => return Err(error),
            };
        let started_next = self
            .prompt_state_owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                agent_id,
                Some(next_prompt.id()),
                self.session_store.reserve_prompt_id(),
            );
        let started_next = match started_next {
            Ok(Some(prompt)) => prompt,
            Ok(None) => {
                if acquired_workflow_claim == Some(true) {
                    self.release_workflow_node_workspace_claim(
                        session_id,
                        next_prompt.workflow_run_id().unwrap_or_default(),
                        next_prompt.workflow_node_run_id().unwrap_or_default(),
                    );
                }
                return Err(DaemonError::LocalTransport {
                    operation: "advance queued prompt",
                    message: format!(
                        "expected queued prompt `{}` but no queued prompt was available",
                        next_prompt.id()
                    ),
                });
            }
            Err(error) => {
                if acquired_workflow_claim == Some(true) {
                    self.release_workflow_node_workspace_claim(
                        session_id,
                        next_prompt.workflow_run_id().unwrap_or_default(),
                        next_prompt.workflow_node_run_id().unwrap_or_default(),
                    );
                }
                // Another queue-advancement path may have promoted a prompt
                // after our initial idle check. That path now owns the active
                // turn; leave any later queued work for its completion instead
                // of treating this provider run as a launch failure.
                if self
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent_id)
                    .is_some()
                {
                    return Ok(None);
                }
                return Err(error);
            }
        };
        let source_attachment_id = self.promoted_prompt_source_attachment_id(
            session_id,
            started_next.source_attachment_id(),
        )?;
        let prompt_sent_at_ms =
            self.record_started_user_prompt(session_id, &source_attachment_id, &started_next)?;
        self.echo_promoted_queued_prompt_to_attachments(
            session_id,
            provider_run_id,
            started_next.id(),
            &source_attachment_id,
            started_next.prompt(),
            started_next.attachments(),
        );
        self.capture_git_turn_snapshot_for_started_prompt(
            &session,
            agent_id,
            &provider_run,
            &started_next,
            Some(prompt_sent_at_ms),
        );
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.workflow_mark_prompt_started(session_id, &started_next)?;
        let _ = self.session_snapshot(session_id)?;
        Ok(Some(crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.to_string(),
            prompt_id: started_next.id().to_string(),
            target_active_prompt_id: None,
            source_attachment_id,
            prompt: started_next.prompt().to_string(),
            hidden_system_context: started_next.hidden_system_context().to_string(),
            attachments: started_next.attachments().to_vec(),
            prompt_origin: started_next.prompt_origin(),
            external_provider: started_next.external_provider().map(str::to_string),
            external_provider_session_id: started_next
                .external_provider_session_id()
                .map(str::to_string),
            external_provider_turn_id: started_next.external_provider_turn_id().map(str::to_string),
            steering: false,
        }))
    }

    fn queued_prompt_steer_context(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        expected_reservation: Option<u64>,
    ) -> Result<QueuedPromptSteerContext, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let (_, queued_prompts) = self.prompt_state_owner.state_parts(&session, agent_id);
        if let Some(prompt) = queued_prompts
            .iter()
            .find(|prompt| prompt.id() == prompt_id)
        {
            if let Some((target_home_prompt_id, worker_provider_run_id, worker_kernel_id, _, _, _)) =
                prompt.remote_steer_outcome_uncertainty()
            {
                let held_by_expected_reservation = expected_reservation
                    .is_some_and(|reservation| prompt.remote_steer_reservation_matches(reservation));
                if !held_by_expected_reservation {
                    return Err(DaemonError::LocalTransport {
                        operation: "steer queued prompt",
                        message: format!(
                            "queued prompt `{prompt_id}` has an uncertain remote steer outcome for home prompt `{}` on worker `{}` run `{}`; it remains blocked until an exact worker receipt is reconciled",
                            target_home_prompt_id,
                            worker_kernel_id,
                            worker_provider_run_id
                                .as_deref()
                                .unwrap_or("unknown"),
                        ),
                    });
                }
            }
        }
        let Some(active_prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Err(DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            });
        };
        if active_prompt.is_external() {
            return Err(DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: "queued prompts cannot be steered into externally started provider turns"
                    .to_string(),
            });
        }
        let provider_run_id = if let Some(remote_execution) = target_agent.remote_execution() {
            if let Some(provider_run) = self
                .provider_run_projection
                .get_for_agent(session_id, agent_id)
            {
                if provider_run.session_id() != session_id {
                    return Err(DaemonError::ProviderRunNotInSession {
                        session_id: session_id.to_string(),
                        provider_run_id: provider_run.id().to_string(),
                    });
                }
                if provider_run.state() != crate::provider::ProviderRunState::Running {
                    return Err(DaemonError::InvalidProviderRunState {
                        provider_run_id: provider_run.id().to_string(),
                        state: provider_run.state(),
                        operation: "steer queued prompt",
                    });
                }
            }
            remote_execution
                .active_worker_provider_run_id
                .clone()
                .ok_or_else(|| DaemonError::NoActiveProviderRun {
                    session_id: session_id.to_string(),
                })?
        } else {
            let provider_run = self
                .provider_store
                .get_run_for_agent(session_id, agent_id)
                .ok_or_else(|| DaemonError::NoActiveProviderRun {
                    session_id: session_id.to_string(),
                })?;
            let provider_run =
                self.ensure_provider_run_in_session(session_id, provider_run.id())?;
            if provider_run.state() != crate::provider::ProviderRunState::Running {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: provider_run.id().to_string(),
                    state: provider_run.state(),
                    operation: "steer queued prompt",
                });
            }
            provider_run.id().to_string()
        };
        let (_, queued_prompts) = self.prompt_state_owner.state_parts(&session, agent_id);
        let queued_prompt = queued_prompts
            .iter()
            .find(|prompt| prompt.id() == prompt_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        if queued_prompt.workflow_run_id().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: "workflow queued prompts cannot be steered manually".to_string(),
            });
        }
        Ok(QueuedPromptSteerContext {
            session,
            agent: target_agent,
            active_prompt,
            provider_run_id,
            queued_prompt,
        })
    }

    pub(super) fn prepare_remote_queued_prompt_steer(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        expected_reservation: Option<u64>,
    ) -> Result<Option<RemoteQueuedPromptSteerPreparation>, DaemonError> {
        let context = self.queued_prompt_steer_context(
            session_id,
            agent_id,
            attachment_id,
            prompt_id,
            expected_reservation,
        )?;
        let Some(remote_execution) = context.agent.remote_execution().cloned() else {
            return Ok(None);
        };
        Ok(Some(RemoteQueuedPromptSteerPreparation {
            agent: context.agent,
            remote_execution,
            target_active_prompt_id: context.active_prompt.id().to_string(),
            provider_run_id: context.provider_run_id,
            prompt: context.queued_prompt,
        }))
    }

    pub(super) fn reserve_remote_queued_prompt_steer(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        prepared: &RemoteQueuedPromptSteerPreparation,
    ) -> Result<u64, DaemonError> {
        let context = self.queued_prompt_steer_context(
            session_id,
            agent_id,
            attachment_id,
            prompt_id,
            None,
        )?;
        if context.active_prompt.id() != prepared.target_active_prompt_id
            || context.provider_run_id != prepared.provider_run_id
            || context.agent.remote_execution() != Some(&prepared.remote_execution)
            || context.queued_prompt != prepared.prompt
        {
            return Err(DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: "queued prompt, active turn, or remote binding changed before reservation".to_string(),
            });
        }
        self.prompt_state_owner.reserve_queued_prompt_remote_steer(
            &context.session,
            agent_id,
            context.active_prompt.id(),
            &context.queued_prompt,
        )
    }

    pub(super) fn mark_remote_queued_prompt_steer_uncertain(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_prompt: &crate::session::PromptQueueItem,
        reservation: u64,
        target_home_prompt_id: String,
        worker_provider_run_id: Option<String>,
        worker_kernel_id: String,
        worker_machine_id: String,
        execution_lease_id: String,
        leased_agent_id: String,
    ) -> Result<(), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let (active_prompt, mut queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        let Some(current_prompt) = queued_prompts
            .iter_mut()
            .find(|prompt| prompt.id() == expected_prompt.id())
        else {
            return Err(DaemonError::LocalTransport {
                operation: "persist uncertain remote queued prompt steer",
                message: format!(
                    "queued prompt `{}` disappeared before its uncertain steer could be persisted",
                    expected_prompt.id()
                ),
            });
        };
        if current_prompt != expected_prompt
            || !current_prompt.remote_steer_reservation_matches(reservation)
            || !current_prompt.mark_remote_steer_outcome_uncertain(
                reservation,
                target_home_prompt_id,
                worker_provider_run_id,
                worker_kernel_id,
                worker_machine_id,
                execution_lease_id,
                leased_agent_id,
            )
        {
            return Err(DaemonError::LocalTransport {
                operation: "persist uncertain remote queued prompt steer",
                message: format!(
                    "queued prompt `{}` no longer matches the in-flight steer reservation",
                    expected_prompt.id()
                ),
            });
        }
        self.mirror_prompt_owner_agent_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )
    }

    pub(super) fn clear_remote_queued_prompt_steer_uncertainty(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_prompt: &crate::session::PromptQueueItem,
        reservation: u64,
    ) -> Result<(), DaemonError> {
        let prompt_id = expected_prompt.id();
        let session = self.session_store.get_session(session_id)?;
        let (active_prompt, mut queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        let Some(current_prompt) = queued_prompts
            .iter_mut()
            .find(|prompt| prompt.id() == prompt_id)
        else {
            return Err(DaemonError::LocalTransport {
                operation: "clear rejected remote queued prompt steer",
                message: format!(
                    "queued prompt `{prompt_id}` disappeared before rejection was persisted"
                ),
            });
        };
        if current_prompt != expected_prompt {
            return Err(DaemonError::LocalTransport {
                operation: "clear rejected remote queued prompt steer",
                message: format!(
                    "queued prompt `{prompt_id}` changed before rejection was persisted"
                ),
            });
        }
        let restore_prompt = current_prompt.clone();
        let Some(uncertainty) = current_prompt
            .clear_remote_steer_outcome_uncertainty(reservation)
        else {
            return Err(DaemonError::LocalTransport {
                operation: "clear rejected remote queued prompt steer",
                message: format!(
                    "queued prompt `{prompt_id}` no longer matches the uncertain steer reservation"
                ),
            });
        };
        let persisted = self.mirror_prompt_owner_agent_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        );
        if persisted.is_err() {
            let _ = restore_prompt.mark_remote_steer_outcome_uncertain(
                reservation,
                uncertainty.0,
                uncertainty.1,
                uncertainty.2,
                uncertainty.3,
                uncertainty.4,
                uncertainty.5,
            );
        }
        persisted
    }

    fn commit_queued_prompt_steer(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        context: QueuedPromptSteerContext,
        dispatch_locally: bool,
    ) -> Result<crate::app::KernelQueuedPromptSteer, DaemonError> {
        let target_active_prompt_id = context.active_prompt.id().to_string();
        let source_attachment_id = self.promoted_prompt_source_attachment_id(
            session_id,
            context.queued_prompt.source_attachment_id(),
        )?;
        self.append_steering_prompt_history(
            session_id,
            &context.provider_run_id,
            &target_active_prompt_id,
            &source_attachment_id,
            agent_id,
            context.queued_prompt.id(),
            context.queued_prompt.prompt(),
            context.queued_prompt.attachments(),
        )?;
        let prompt = self
            .prompt_state_owner
            .remove_queued_prompt(&context.session, agent_id, prompt_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&context.session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.echo_steering_prompt_to_other_attachments(
            session_id,
            &context.provider_run_id,
            agent_id,
            prompt.id(),
            &source_attachment_id,
            attachment_id,
            prompt.prompt(),
            prompt.attachments(),
            prompt.prompt_origin(),
        );
        self.record_notice(
            session_id,
            Some(&context.provider_run_id),
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{}` steered queued prompt `{}` to agent `{}`.",
                attachment_id,
                prompt.id(),
                agent_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        let dispatch = dispatch_locally.then(|| crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: context.provider_run_id,
            agent_id: agent_id.to_string(),
            prompt_id: prompt.id().to_string(),
            target_active_prompt_id: Some(target_active_prompt_id),
            source_attachment_id,
            prompt: prompt.prompt().to_string(),
            hidden_system_context: prompt.hidden_system_context().to_string(),
            attachments: prompt.attachments().to_vec(),
            prompt_origin: prompt.prompt_origin(),
            external_provider: prompt.external_provider().map(str::to_string),
            external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
            external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
            steering: true,
        });
        Ok(crate::app::KernelQueuedPromptSteer {
            dispatch,
            prompt,
            session,
        })
    }

    pub(super) fn steer_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<Option<crate::app::KernelQueuedPromptSteer>, DaemonError> {
        let context = self.queued_prompt_steer_context(
            session_id,
            agent_id,
            attachment_id,
            prompt_id,
            None,
        )?;
        if context.agent.remote_execution().is_some() {
            return Ok(None);
        }
        self.commit_queued_prompt_steer(
            session_id,
            agent_id,
            attachment_id,
            prompt_id,
            context,
            true,
        )
        .map(Some)
    }

    pub(super) fn finish_remote_queued_prompt_steer(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prepared: &RemoteQueuedPromptSteerPreparation,
        expected_binding: &crate::agent::RemoteAgentBinding,
        reservation: u64,
        provider_run_id: &str,
    ) -> Result<crate::app::KernelQueuedPromptSteer, DaemonError> {
        if !prepared.prompt.remote_steer_reservation_matches(reservation) {
            return Err(DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: "remote queued prompt steer no longer owns its reservation".to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let current_agent = self.agent_store.get_agent(agent_id)?;
        if !remote_steer_binding_identity_matches(
            current_agent.remote_execution(),
            Some(expected_binding),
        ) {
            return Err(DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: "remote binding changed while the queued steer was in flight".to_string(),
            });
        }
        let (_, queued_prompts) = self.prompt_state_owner.state_parts(&session, agent_id);
        let Some(current_prompt) = queued_prompts.iter().find(|prompt| {
            prompt.id() == prepared.prompt.id()
                && prompt.remote_steer_reservation_matches(reservation)
        }) else {
            return Err(DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: format!("queued prompt `{}` no longer owns its reservation", prepared.prompt.id()),
            });
        };
        if current_prompt != &prepared.prompt {
            return Err(DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: format!("queued prompt `{}` changed while its remote steer was in flight", prepared.prompt.id()),
            });
        }
        let source_attachment_id = self.promoted_prompt_source_attachment_id(
            session_id,
            prepared.prompt.source_attachment_id(),
        )?;
        self.append_steering_prompt_history(
            session_id,
            provider_run_id,
            &prepared.target_active_prompt_id,
            &source_attachment_id,
            agent_id,
            prepared.prompt.id(),
            prepared.prompt.prompt(),
            prepared.prompt.attachments(),
        )?;
        let prompt = self
            .prompt_state_owner
            .remove_queued_prompt(&session, agent_id, prepared.prompt.id())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: format!("queued prompt `{}` disappeared before commit", prepared.prompt.id()),
            })?;
        let _ = prompt.release_remote_steer(reservation);
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        let projected_provider_run_id = crate::provider::projected_leased_provider_run_id(
            &prepared.remote_execution.leased_agent_id,
            provider_run_id,
        );
        self.echo_steering_prompt_to_other_attachments(
            session_id,
            &projected_provider_run_id,
            agent_id,
            prompt.id(),
            &source_attachment_id,
            attachment_id,
            prompt.prompt(),
            prompt.attachments(),
            prompt.prompt_origin(),
        );
        self.record_notice(
            session_id,
            Some(&projected_provider_run_id),
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{attachment_id}` steered queued prompt `{}` to agent `{agent_id}`.",
                prompt.id()
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelQueuedPromptSteer {
            dispatch: None,
            prompt,
            session,
        })
    }

    pub(super) fn reconcile_remote_queued_prompt_steer_receipt(
        &self,
        session_id: &str,
        agent_id: &str,
        queued_prompt_id: &str,
        receipt: &crate::transport::relay_peer::LeasedPromptReceipt,
    ) -> Result<RemoteQueuedPromptSteerReceiptSettlement, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        let Some(queued_prompt) = queued_prompts
            .iter()
            .find(|prompt| prompt.id() == queued_prompt_id)
            .cloned()
        else {
            return Ok(RemoteQueuedPromptSteerReceiptSettlement::AlreadySettled);
        };
        if receipt.home_prompt_id != queued_prompt_id {
            return Err(DaemonError::LocalTransport {
                operation: "reconcile remote queued prompt steer",
                message: "worker receipt names a different queued prompt".to_string(),
            });
        }
        let Some(target_home_prompt_id) = receipt.target_home_prompt_id.as_deref() else {
            return Err(DaemonError::LocalTransport {
                operation: "reconcile remote queued prompt steer",
                message: "worker receipt omitted the target home prompt identity".to_string(),
            });
        };
        let Some(execution_lease_id) = receipt.execution_lease_id.as_deref() else {
            return Err(DaemonError::LocalTransport {
                operation: "reconcile remote queued prompt steer",
                message: "worker receipt omitted the execution lease identity".to_string(),
            });
        };
        let Some((
            expected_target_home_prompt_id,
            expected_worker_run_id,
            worker_kernel_id,
            worker_machine_id,
            expected_execution_lease_id,
            leased_agent_id,
        )) = queued_prompt.remote_steer_outcome_uncertainty()
        else {
            return Err(DaemonError::LocalTransport {
                operation: "reconcile remote queued prompt steer",
                message: "queued prompt has no durable uncertain-steer identity".to_string(),
            });
        };
        if expected_worker_run_id.as_deref() != Some(receipt.worker_provider_run_id.as_str())
            || expected_target_home_prompt_id != target_home_prompt_id
            || expected_execution_lease_id != execution_lease_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "reconcile remote queued prompt steer",
                message: "worker receipt conflicts with the durable queued-steer target, run, or lease".to_string(),
            });
        }
        let current_agent = self.agent_store.get_agent(agent_id)?;
        let binding = current_agent
            .remote_execution()
            .filter(|binding| {
                binding.worker_kernel_id == worker_kernel_id
                    && binding.worker_machine_id == worker_machine_id
                    && binding.execution_lease_id == expected_execution_lease_id
                    && binding.leased_agent_id == leased_agent_id
            })
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "reconcile remote queued prompt steer",
                message: "current worker binding no longer matches the durable steer receipt".to_string(),
            })?;
        if active_prompt
            .as_ref()
            .is_some_and(|active| active.id() != target_home_prompt_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "reconcile remote queued prompt steer",
                message: "a different home prompt is active; the uncertain steer remains held".to_string(),
            });
        }
        if receipt.phase == crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerDispatching
        {
            return Ok(RemoteQueuedPromptSteerReceiptSettlement::Pending);
        }
        match receipt.phase {
            crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerAccepted => {
                let source_attachment_id = self.promoted_prompt_source_attachment_id(
                    session_id,
                    queued_prompt.source_attachment_id(),
                )?;
                let projected_provider_run_id = crate::provider::projected_leased_provider_run_id(
                    &binding.leased_agent_id,
                    &receipt.worker_provider_run_id,
                );
                // The stable prompt merge key makes this idempotent if a crash
                // happens after history append but before the queue commit.
                self.append_steering_prompt_history(
                    session_id,
                    &receipt.worker_provider_run_id,
                    target_home_prompt_id,
                    &source_attachment_id,
                    agent_id,
                    queued_prompt_id,
                    queued_prompt.prompt(),
                    queued_prompt.attachments(),
                )?;
                self.prompt_state_owner
                    .remove_queued_prompt(&session, agent_id, queued_prompt_id)
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "reconcile remote queued prompt steer",
                        message: format!(
                            "queued prompt `{queued_prompt_id}` disappeared before receipt settlement"
                        ),
                    })?;
                let (active_prompt, queued_prompts) =
                    self.prompt_state_owner.state_parts(&session, agent_id);
                self.mirror_prompt_owner_agent_state(
                    session_id,
                    agent_id,
                    active_prompt,
                    queued_prompts,
                )?;
                self.record_notice(
                    session_id,
                    Some(&projected_provider_run_id),
                    self.other_attachment_ids(session_id, &source_attachment_id),
                    format!(
                        "Recovered accepted queued steer `{queued_prompt_id}` from its exact worker receipt."
                    ),
                );
                Ok(RemoteQueuedPromptSteerReceiptSettlement::Accepted)
            }
            crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerRejected => {
                let (active_prompt, mut queued_prompts) =
                    self.prompt_state_owner.state_parts(&session, agent_id);
                let Some(current_prompt) = queued_prompts
                    .iter_mut()
                    .find(|prompt| prompt.id() == queued_prompt_id)
                else {
                    return Ok(RemoteQueuedPromptSteerReceiptSettlement::AlreadySettled);
                };
                if current_prompt != &queued_prompt
                    || !current_prompt.clear_remote_steer_outcome_uncertainty_for_receipt(
                        target_home_prompt_id,
                        &receipt.worker_provider_run_id,
                        &worker_kernel_id,
                        &worker_machine_id,
                        execution_lease_id,
                        &leased_agent_id,
                    )
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "reconcile remote queued prompt steer",
                        message: "exact rejection receipt could not release the durable queue hold".to_string(),
                    });
                }
                self.mirror_prompt_owner_agent_state(
                    session_id,
                    agent_id,
                    active_prompt,
                    queued_prompts,
                )?;
                Ok(RemoteQueuedPromptSteerReceiptSettlement::Rejected)
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "reconcile remote queued prompt steer",
                message: "worker returned a prompt receipt without a queued-steer outcome".to_string(),
            }),
        }
    }

    pub(super) fn cancel_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<crate::app::KernelQueuedPromptCancellation, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        self.ensure_queued_prompt_manually_mutable(
            &session,
            agent_id,
            prompt_id,
            "cancel queued prompt",
            "cancelled",
        )?;
        let mut prompt = self
            .prompt_state_owner
            .remove_queued_prompt(&session, agent_id, prompt_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "cancel queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        prompt.set_status(crate::session::PromptStatus::Cancelled);
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{}` cancelled queued prompt `{}` for agent `{}`.",
                attachment_id,
                prompt.id(),
                agent_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelQueuedPromptCancellation { prompt, session })
    }

    pub(super) fn update_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        prompt_text: &str,
    ) -> Result<crate::app::KernelQueuedPromptUpdate, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        self.ensure_queued_prompt_manually_mutable(
            &session,
            agent_id,
            prompt_id,
            "update queued prompt",
            "updated",
        )?;
        let prompt = self
            .prompt_state_owner
            .update_queued_prompt(&session, agent_id, prompt_id, prompt_text)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "update queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{}` updated queued prompt `{}` for agent `{}`.",
                attachment_id,
                prompt.id(),
                agent_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelQueuedPromptUpdate { prompt, session })
    }
}

fn remote_steer_binding_identity_matches(
    current: Option<&crate::agent::RemoteAgentBinding>,
    expected: Option<&crate::agent::RemoteAgentBinding>,
) -> bool {
    let (Some(current), Some(expected)) = (current, expected) else {
        return false;
    };
    current.worker_kernel_id == expected.worker_kernel_id
        && current.worker_machine_id == expected.worker_machine_id
        && current.execution_lease_id == expected.execution_lease_id
        && current.leased_agent_id == expected.leased_agent_id
        && current.relay_url == expected.relay_url
        && current.relay_token == expected.relay_token
        && current.relay_peer_protocol_version == expected.relay_peer_protocol_version
}
