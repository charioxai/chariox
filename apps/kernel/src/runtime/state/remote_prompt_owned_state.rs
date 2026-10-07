//! Remote-agent prompt ownership transitions.
//!
//! This module owns prompt queue state for agents leased to remote kernels. Local provider prompt
//! lifecycle remains in `prompt`.

use super::*;

pub(super) enum RemotePromptDispatchSettlement {
    Settled(crate::session::PromptQueueItem),
    Superseded,
    BindingChanged(crate::session::PromptQueueItem),
}

impl KernelRuntimeOwnedState {
    pub(super) fn settle_remote_dispatch_if_current(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        delivered_run_id: Option<&str>,
    ) -> Result<RemotePromptDispatchSettlement, DaemonError> {
        let activity_mutation = self.begin_managed_activity_mutation();
        let mut sessions = self.session_store.write();
        let session = sessions.get_session(&dispatch.session_id)?;
        let mut binding_changed = None;
        let settled = self
            .prompt_state_owner
            .settle_active_remote_dispatch_if_matches(
                &session,
                &dispatch.agent_id,
                &dispatch.prompt_id,
                delivered_run_id,
                |previous, active, queued| {
                    // Lock order: activity -> session -> prompt owner -> agent.
                    // No projection/history helper may reenter prompt ownership here.
                    let mut agents = self.agent_store.write();
                    let agent = agents.get_agent(&dispatch.agent_id)?;
                    if !agent.remote_execution().is_some_and(|binding| {
                        binding.worker_kernel_id == dispatch.worker_kernel_id
                            && binding.leased_agent_id == dispatch.leased_agent_id
                    }) {
                        binding_changed = Some(previous.clone());
                        return Ok(false);
                    }
                    let mirrored = sessions.mirror_agent_prompt_state(
                        &dispatch.session_id,
                        &dispatch.agent_id,
                        active,
                        queued.clone(),
                    )?;
                    // Persist under the ownership locks so a successor cannot
                    // race the durable commit. This briefly blocks readers.
                    if let Err(error) =
                        self.persist_prompt_session_state(&mirrored, &dispatch.agent_id)
                    {
                        sessions.mirror_agent_prompt_state(
                            &dispatch.session_id,
                            &dispatch.agent_id,
                            Some(previous.clone()),
                            queued,
                        )?;
                        return Err(error);
                    }
                    agents.set_remote_execution_active_worker_provider_run_id(
                        &dispatch.agent_id,
                        delivered_run_id.map(str::to_string),
                    )?;
                    if delivered_run_id.is_some() {
                        if agent.state() == crate::agent::AgentState::Error {
                            agents.set_agent_state(
                                &dispatch.agent_id,
                                crate::agent::AgentState::Idle,
                            )?;
                        }
                    } else {
                        agents.set_agent_processing(&dispatch.agent_id, false)?;
                        agents
                            .set_agent_state(&dispatch.agent_id, crate::agent::AgentState::Error)?;
                    }
                    Ok(true)
                },
            )?;
        // Recording activity samples session state; never do it while holding
        // the session write lock or the nested ownership locks.
        drop(sessions);
        if settled.is_some() {
            activity_mutation.record();
        }
        Ok(match (settled, binding_changed) {
            (Some(prompt), _) => RemotePromptDispatchSettlement::Settled(prompt),
            (None, Some(prompt)) => RemotePromptDispatchSettlement::BindingChanged(prompt),
            (None, None) => RemotePromptDispatchSettlement::Superseded,
        })
    }

    pub(super) fn advance_next_queued_remote_prompt_dispatch(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        self.prepare_queued_remote_prompt_dispatch(session_id, agent_id, None)
    }

    pub(super) fn finish_remote_profile_transition(
        &self,
        session_id: &str,
        agent_id: &str,
        claim: crate::runtime::prompt_state::AgentProfileTransitionClaim,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        self.prepare_queued_remote_prompt_dispatch(session_id, agent_id, Some(claim))
    }

    fn prepare_queued_remote_prompt_dispatch(
        &self,
        session_id: &str,
        agent_id: &str,
        profile_transition: Option<crate::runtime::prompt_state::AgentProfileTransitionClaim>,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(None);
        };
        let session = self.session_store.get_session(session_id)?;
        let next_prompt = if profile_transition.is_none() {
            let Some(prompt) = self
                .prompt_state_owner
                .peek_next_queued_prompt(&session, agent_id)
            else {
                return Ok(None);
            };
            if prompt.remote_steer_reserved() {
                return Ok(None);
            }
            Some(prompt)
        } else {
            None
        };
        if !self.provider_account_allows_queued_prompt_advance(
            session_id,
            &agent,
            "advance remote queued prompt",
        ) {
            return Ok(None);
        }
        let _admission = match self.begin_managed_activity_admission() {
            Ok(admission) => admission,
            Err(_) => return Ok(None),
        };
        let activity_mutation = self.begin_managed_activity_mutation();
        let started = if let Some(claim) = profile_transition {
            claim.finish_and_activate_next(
                &session,
                agent_id,
                self.session_store.reserve_prompt_id(),
            )?
        } else {
            let next_prompt = next_prompt.expect("ordinary queue advance checked its queue front");
            self.prompt_state_owner
                .activate_next_queued_prompt_with_prompt_id(
                    &session,
                    agent_id,
                    Some(next_prompt.id()),
                    self.session_store.reserve_prompt_id(),
                )?
        };
        let Some(started) = started else {
            return Ok(None);
        };
        let _ =
            self.record_started_user_prompt(session_id, started.source_attachment_id(), &started)?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state_with_activity_mutation(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
            activity_mutation,
        )?;
        self.persist_prompt_session_state(&self.session_store.get_session(session_id)?, agent_id)?;
        let remote_dispatch = crate::app::KernelRemotePromptDispatch {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            prompt_id: started.id().to_string(),
            worker_kernel_id: remote_execution.worker_kernel_id,
            leased_agent_id: remote_execution.leased_agent_id,
            relay_url: remote_execution.relay_url,
            relay_token: remote_execution.relay_token,
            source_attachment_id: started.source_attachment_id().to_string(),
            prompt: started.prompt().to_string(),
            hidden_system_context: started.hidden_system_context().to_string(),
            attachments: started.attachments().to_vec(),
            workspace_live_sync_mode: Some(
                crate::provider::provider_workspace_live_sync_mode_for_session(
                    agent.provider(),
                    &self.config_projection.snapshot(),
                    Some(&session),
                ),
            ),
            prompt_origin: started.prompt_origin(),
            external_provider: started.external_provider().map(str::to_string),
            external_provider_session_id: started
                .external_provider_session_id()
                .map(str::to_string),
            external_provider_turn_id: started.external_provider_turn_id().map(str::to_string),
            workflow_context: None,
        };
        let session = self.session_snapshot(session_id)?;
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome: crate::session::PromptSubmissionOutcome::Started { prompt: started },
            session,
            dispatch: None,
            remote_dispatch: Some(remote_dispatch),
        }))
    }

    pub(super) fn submit_remote_prepared_prompt(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        self.submit_remote_prepared_prompt_with_queue_policy(prepared, true)
    }

    pub(super) fn submit_remote_prepared_prompt_with_queue_policy(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
        allow_queue: bool,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let session_id = prepared.session_id.clone();
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        let source_attachment =
            if crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
                None
            } else {
                Some(self.ensure_attachment_in_session(&session_id, &attachment_id)?)
            };
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        let target_agent = self.agent_store.get_agent(&target_agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id,
                agent_id: target_agent_id,
            });
        }
        let Some(remote_execution) = target_agent.remote_execution().cloned() else {
            return Ok(None);
        };
        self.provider_account_profiles.require_agent_authenticated(
            &self.config_projection.snapshot(),
            &target_agent,
            "submit remote prompt",
        )?;
        if target_agent.state() == crate::agent::AgentState::Error {
            let _ = self
                .agent_store
                .set_agent_state(&target_agent_id, crate::agent::AgentState::Idle)?;
        }
        let session = self.session_store.get_session(&session_id)?;
        let queued_while_active = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &target_agent_id)
            .is_some();
        let will_queue = prepared.force_queue || queued_while_active;
        let prompt = if let Some(source_attachment) = source_attachment.as_ref() {
            prepared.prompt.clone().with_source_attribution(
                source_attachment.client_id(),
                source_attachment.owner_user_id(),
            )
        } else {
            prepared.prompt.clone()
        };
        let prompt = if will_queue {
            prompt
        } else {
            prompt.with_id(self.session_store.reserve_prompt_id())
        };
        let _admission = self.begin_managed_activity_admission()?;
        let outcome = self
            .prompt_state_owner
            .submit_prepared_prompt_with_queue_policy(
                &session,
                prompt,
                prepared.force_queue,
                allow_queue,
            )?;
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.mirror_prompt_owner_agent_state(
            &session_id,
            &outcome_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let remote_dispatch = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => {
                let _ = self.record_started_user_prompt(
                    &session_id,
                    prompt.source_attachment_id(),
                    prompt,
                )?;
                self.persist_prompt_session_state(
                    &self.session_store.get_session(&session_id)?,
                    &outcome_agent_id,
                )?;
                Some(crate::app::KernelRemotePromptDispatch {
                    session_id: session_id.clone(),
                    agent_id: target_agent_id.clone(),
                    prompt_id: prompt.id().to_string(),
                    worker_kernel_id: remote_execution.worker_kernel_id,
                    leased_agent_id: remote_execution.leased_agent_id,
                    relay_url: remote_execution.relay_url,
                    relay_token: remote_execution.relay_token,
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    hidden_system_context: prompt.hidden_system_context().to_string(),
                    attachments: prompt.attachments().to_vec(),
                    workspace_live_sync_mode: Some(
                        crate::provider::provider_workspace_live_sync_mode_for_session(
                            target_agent.provider(),
                            &self.config_projection.snapshot(),
                            Some(&session),
                        ),
                    ),
                    prompt_origin: prompt.prompt_origin(),
                    external_provider: prompt.external_provider().map(str::to_string),
                    external_provider_session_id: prompt
                        .external_provider_session_id()
                        .map(str::to_string),
                    external_provider_turn_id: prompt
                        .external_provider_turn_id()
                        .map(str::to_string),
                    workflow_context: None,
                })
            }
            crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                self.record_notice_for_agent(
                    &session_id,
                    None,
                    Some(&target_agent_id),
                    self.other_attachment_ids(&session_id, prompt.source_attachment_id()),
                    format!(
                        "Attachment `{}` queued prompt `{}` for agent `{}`.",
                        prompt.source_attachment_id(),
                        prompt.id(),
                        target_agent_id
                    ),
                );
                None
            }
        };
        let session = if prepared.refresh_projection {
            self.session_snapshot(&session_id)?
        } else {
            self.session_snapshot_without_projection_update(&session_id)?
        };
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
        self.complete_remote_prompt_owner_with_termination(
            session_id,
            agent_id,
            remote_provider_run_id,
            next_queued_prompt,
            None,
        )
    }

    pub(super) fn complete_remote_prompt_owner_with_termination(
        &self,
        session_id: &str,
        agent_id: &str,
        remote_provider_run_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
        provider_termination: Option<crate::provider::ProviderRunTermination>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        self.complete_remote_prompt_owner_for_receipt(
            session_id,
            agent_id,
            remote_provider_run_id,
            next_queued_prompt,
            provider_termination,
            None,
        )
    }

    pub(super) fn complete_remote_prompt_owner_for_receipt(
        &self,
        session_id: &str,
        agent_id: &str,
        remote_provider_run_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
        provider_termination: Option<crate::provider::ProviderRunTermination>,
        receipt: Option<(&str, &crate::agent::RemoteAgentBinding)>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let sessions = self.session_store.read();
        let mut agents = self.agent_store.write();
        let agent = agents.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = sessions.get_session(session_id)?;
        if receipt.is_some_and(|(_, expected)| {
            agent.remote_execution() != Some(expected)
                || expected
                    .active_worker_provider_run_id
                    .as_deref()
                    .is_some_and(|run| run != remote_provider_run_id)
        }) {
            return Err(DaemonError::LocalTransport {
                operation: "settle remote prompt receipt",
                message: "remote binding changed before completion settled".into(),
            });
        }
        let completed = self
            .prompt_state_owner
            .complete_active_prompt_if_matches(
                &session,
                agent_id,
                receipt.map(|(prompt, _)| prompt),
            )
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        agents.set_remote_execution_active_worker_provider_run_id(agent_id, None)?;
        drop(agents);
        drop(sessions);
        let settled_at_ms = crate::session::unix_epoch_ms();
        let archive_enabled = self
            .config_projection
            .snapshot()
            .user_config
            .history
            .archive
            .mode
            == crate::config::HistoryArchiveMode::External;
        let settlement_status = if provider_termination.is_some() {
            crate::git_observer::CompletedTurnSettlementStatus::Failed
        } else {
            crate::git_observer::CompletedTurnSettlementStatus::Completed
        };
        self.operational_history_store.record_prompt_settlement(
            archive_enabled,
            session_id,
            agent_id,
            completed.id(),
            Some(remote_provider_run_id),
            settled_at_ms,
            settlement_status.as_str(),
        );
        self.completed_git_turn_snapshots
            .record_prompt_settlement_with_termination(
                session_id,
                agent_id,
                remote_provider_run_id,
                &completed,
                settled_at_ms,
                Some(completed.created_at_ms()),
                settlement_status,
                provider_termination.clone(),
            );
        if provider_termination.is_some() {
            let _ = self
                .agent_store
                .mark_unexpected_provider_exit_error(agent_id, true);
        }
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        self.record_assistant_message_completion(
            session_id,
            remote_provider_run_id,
            recipient_attachment_ids,
            &format!("prompt-complete:{}", completed.id()),
            settled_at_ms,
        );
        let started_next = if let Some(expected_next) = next_queued_prompt {
            match self.begin_managed_activity_admission() {
                Err(_) => None,
                Ok(_admission) => {
                    let activity_mutation = self.begin_managed_activity_mutation();
                    let active = self
                        .prompt_state_owner
                        .activate_next_queued_prompt_with_prompt_id(
                            &session,
                            agent_id,
                            Some(expected_next.id()),
                            self.session_store.reserve_prompt_id(),
                        )?;
                    if active.is_some() {
                        let (active_prompt, queued_prompts) =
                            self.prompt_state_owner.state_parts(&session, agent_id);
                        self.mirror_prompt_owner_agent_state_with_activity_mutation(
                            session_id,
                            agent_id,
                            active_prompt,
                            queued_prompts,
                            activity_mutation,
                        )?;
                    }
                    if let Some(active_prompt) = active.as_ref() {
                        let _ = self.record_started_user_prompt(
                            session_id,
                            active_prompt.source_attachment_id(),
                            active_prompt,
                        )?;
                    }
                    active
                }
            }
        } else {
            None
        };
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
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
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(attachment_id) {
            let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        }
        let target_agent = self.agent_store.get_agent(target_agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: target_agent_id.to_string(),
            });
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
        self.mirror_prompt_owner_agent_state(
            session_id,
            target_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let worker_kernel_id = target_agent
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

    pub(super) fn finalize_remote_prompt_cancellation_after_worker_settled(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        let _ =
            self.begin_remote_prompt_cancellation(session_id, target_agent_id, attachment_id)?;
        let cancellation = self.finalize_local_prompt_cancellation_with_queued_advance(
            session_id,
            target_agent_id,
            None,
        )?;
        if cancellation.cancellation.prompt.workflow_run_id().is_some() {
            self.workflow_cancel_prompt(session_id, &cancellation.cancellation.prompt)?;
        }
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelPromptCancellation {
            cancellation: cancellation.cancellation,
            session,
            dispatch: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;
    use crate::agent::RemoteAgentBinding;
    use crate::app::{DaemonApp, KernelPreparedPromptSubmission, KernelSessionService};
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    async fn mp11_fixture(
        config: crate::DaemonConfig,
    ) -> (
        KernelRuntimeState,
        String,
        String,
        String,
        RemoteAgentBinding,
    ) {
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("mp11-home", "mp11-home"))
            .unwrap();
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "mp11-client",
                ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let binding = RemoteAgentBinding {
            worker_kernel_id: "worker-kernel-1".into(),
            worker_machine_id: "worker-machine-1".into(),
            execution_lease_id: "mp11-lease".into(),
            leased_agent_id: "mp11-leased".into(),
            active_worker_provider_run_id: Some("worker-run-1".into()),
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };
        app.agents
            .bind_remote_execution(agent.id(), binding.clone())
            .unwrap();
        let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
        (
            runtime,
            session.id().into(),
            agent.id().into(),
            attachment.id().into(),
            binding,
        )
    }

    fn mp11_submit(
        runtime: &KernelRuntimeState,
        session: &str,
        agent: &str,
        attachment: &str,
        text: &str,
    ) -> PromptQueueItem {
        let submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session.into(),
                prompt: PromptQueueItem::new(text, attachment, agent, text, PromptStatus::Queued),
                force_queue: false,
                refresh_projection: true,
            })
            .unwrap()
            .unwrap();
        match submission.outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
        }
    }

    #[tokio::test]
    async fn mp11_projection_drain_rebind_barrier_rejects_old_lease_without_notices() {
        let (runtime, session, agent, attachment, binding) =
            mp11_fixture(crate::DaemonConfig::for_tests()).await;
        let prompt = mp11_submit(&runtime, &session, &agent, &attachment, "drained prompt");
        let mut authority =
            crate::runtime::relay_peer_authority::test_projection_authority("worker-kernel-1");
        authority.expected_binding = Some(binding.clone());
        authority.expected_prompt_id = Some(prompt.id().into());
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let task = tokio::spawn({
            let barrier = barrier.clone();
            let runtime = runtime.clone();
            let session = session.clone();
            let agent = agent.clone();
            async move {
                barrier.wait().await;
                barrier.wait().await;
                runtime
                    .with_app_side_effect(|app| {
                        crate::app::RemoteLeaseRuntime::new(app).project_remote_runtime_projection(
                            authority,
                            crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                                account_copy_observations: Vec::new(),
                                home_session_id: session.to_string(),
                                home_agent_id: agent.to_string(),
                                provider_run_id: "worker-run-1".to_string(),
                                provider_run: None,
                                prompts: vec![],
                                output_chunks: vec![],
                                notices: vec!["must not project".into()],
                                completions: vec![],
                            },
                        )
                    })
                    .await
            }
        });
        barrier.wait().await;
        let mut replacement = binding;
        replacement.execution_lease_id = "new lease during drain".into();
        runtime
            .owned
            .agent_store
            .bind_remote_execution(&agent, replacement)
            .unwrap();
        barrier.wait().await;
        assert!(!task.await.unwrap().unwrap().accepted);
        assert!(runtime
            .drain_notice_records(&session, &attachment)
            .await
            .is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mp11_review_credential_http_stall_allows_prompt_progress_and_revocation() {
        use crate::config::{
            UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialSourceConfig,
            UserCredentialUse,
        };
        use std::io::{Read, Write};
        #[derive(Debug)]
        struct SyntheticVault;
        impl crate::secret::CredentialVaultStore for SyntheticVault {
            fn get_secret(&self, _: &str, _: &str) -> Result<String, DaemonError> {
                Ok("synthetic-http-test-value".into())
            }
            fn set_secret(&self, _: &str, _: &str, _: &str) -> Result<(), DaemonError> {
                unreachable!()
            }
            fn delete_secret(&self, _: &str, _: &str) -> Result<(), DaemonError> {
                unreachable!()
            }
        }
        let (runtime, session, agent, attachment, binding) =
            mp11_fixture(crate::DaemonConfig::for_tests()).await;
        mp11_submit(&runtime, &session, &agent, &attachment, "caller prompt");
        let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
            home_kernel_id: runtime.owned.config_projection.snapshot().daemon_id,
            home_session_id: session.clone(),
            home_agent_id: agent.clone(),
            leased_agent_id: binding.leased_agent_id.clone(),
            worker_kernel_id: Some(binding.worker_kernel_id.clone()),
            worker_machine_id: Some(binding.worker_machine_id.clone()),
            worker_provider_run_id: "worker-run-1".into(),
        };
        let admitted = runtime
            .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
                "worker-kernel-1",
            ))
            .prepare_forwarded_peer_request(
                &crate::transport::relay_peer::RelayPeerRequest::InvokeHomeCredentialTool {
                    context: context.clone(),
                    tool_name: crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL
                        .into(),
                    arguments: serde_json::json!({}),
                },
            )
            .await
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let arrived = Arc::new(tokio::sync::Notify::new());
        let (release, wait) = std::sync::mpsc::channel();
        let server = std::thread::spawn({
            let arrived = arrived.clone();
            move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                let mut input = [0; 4096];
                let count = stream.read(&mut input).unwrap();
                assert!(
                    String::from_utf8_lossy(&input[..count]).contains("synthetic-http-test-value")
                );
                arrived.notify_one();
                let _ = wait.recv_timeout(std::time::Duration::from_secs(15));
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                );
            }
        });
        let service = crate::secret::RuntimeSecretService::with_vault_store(
            vec![UserCredentialConfig {
                id: "synthetic".into(),
                description: None,
                metadata: None,
                source: UserCredentialSourceConfig::Vault {
                    key: "synthetic".into(),
                },
                allowed_hosts: vec![address.to_string()],
                allowed_uses: vec![UserCredentialUse::Http],
                injection: UserCredentialInjectionConfig::Header {
                    name: "authorization".into(),
                    value: "Bearer ${secret}".into(),
                },
            }],
            "synthetic",
            Arc::new(SyntheticVault),
        );
        let request = crate::secret::CredentialHttpRequest {
            credential_id: "synthetic".into(),
            method: "GET".into(),
            url: format!("http://{address}/stall"),
            headers: Default::default(),
            body_text: None,
            body_json: None,
            timeout_ms: 30_000,
            max_response_bytes: 1024,
        };
        let http = tokio::spawn(async move {
            admitted
                .forwarded_credential_http_request(context, service, request)
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(10), arrived.notified())
            .await
            .unwrap();
        let mut progress = tokio::spawn({
            let runtime = runtime.clone();
            move || async move {
                runtime
                    .clone()
                    .with_app_side_effect_blocking(move |app| {
                        let (other_session, other_agent) = KernelSessionService::new(app)
                            .create_session(CreateSessionRequest::new("unrelated", "unrelated"))?;
                        let other_attachment =
                            KernelSessionService::new(app).attach(AttachRequest::new(
                                other_session.id(),
                                "other-client",
                                ClientCapabilityLevel::FullTerminal,
                            ))?;
                        app.agents
                            .bind_remote_execution(other_agent.id(), binding.clone())?;
                        let prompt = mp11_submit(
                            &runtime,
                            other_session.id(),
                            other_agent.id(),
                            other_attachment.id(),
                            "unrelated prompt progresses",
                        );
                        assert_eq!(prompt.status(), PromptStatus::Running);
                        let mut revoked = binding;
                        revoked.execution_lease_id = "revoked while HTTP stalls".into();
                        app.agents.bind_remote_execution(&agent, revoked)?;
                        Ok(())
                    })
                    .await
            }
        }());
        let advanced_while_stalled =
            tokio::time::timeout(std::time::Duration::from_secs(2), &mut progress).await;
        release.send(()).unwrap();
        let progressed = advanced_while_stalled.is_ok();
        if let Ok(result) = advanced_while_stalled {
            result.unwrap().unwrap();
        } else {
            progress.await.unwrap().unwrap();
        }
        let response = http.await.unwrap();
        server.join().unwrap();
        assert!(
            progressed,
            "stalled credential HTTP blocked unrelated prompt and caller revocation"
        );
        assert!(response.is_err(), "revoked caller received HTTP response");
    }

    #[tokio::test]
    async fn mp11_credential_release_revalidates_after_vault_unlock_deduplication() {
        mp11_vault_wait_revocation(true).await;
    }

    #[tokio::test]
    async fn mp11_project_environment_reauthorizes_after_vault_unlock() {
        mp11_vault_wait_revocation(false).await;
    }

    async fn mp11_vault_wait_revocation(credential: bool) {
        let root =
            std::env::temp_dir().join(format!("chariox-mp11-vault-wait-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let vault = root.join("synthetic-vault.json");
        let mut config = crate::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = vault.display().to_string();
        config.user_config.credential_vault.unlock_policy =
            crate::config::CredentialVaultUnlockPolicy::KernelInit;
        crate::secret::unlock_chariox_encrypted_vault(
            &vault,
            "synthetic MP11 passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
        let (runtime, session, agent, attachment, binding) = mp11_fixture(config).await;
        let grant = runtime.insert_access_grant_for_test(&session);
        let request =
            crate::local::LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
                session_id: session.clone(),
                attachment_id: attachment,
                target_agent_id: Some(agent.clone()),
                prompt: "must not enter runtime".into(),
                attachments: vec![],
            });
        let admitted = if credential {
            runtime
                .with_relay_peer_authority(
                    crate::runtime::relay_peer_authority::test_peer_authority("worker-kernel-1"),
                )
                .prepare_forwarded_peer_request(
                    &crate::transport::relay_peer::RelayPeerRequest::ResolveHomeCredentialSecret {
                        context: crate::transport::relay_peer::RemoteExtensionInvocationContext {
                            home_kernel_id: runtime.owned.config_projection.snapshot().daemon_id,
                            home_session_id: session.clone(),
                            home_agent_id: agent.clone(),
                            leased_agent_id: binding.leased_agent_id.clone(),
                            worker_kernel_id: Some(binding.worker_kernel_id.clone()),
                            worker_machine_id: Some(binding.worker_machine_id.clone()),
                            worker_provider_run_id: "worker-run-1".into(),
                        },
                        credential_id: "synthetic".into(),
                        injection:
                            crate::transport::relay_peer::RemoteCredentialSecretInjection::Pty,
                    },
                )
                .await
                .unwrap()
        } else {
            let project_id = runtime
                .owned
                .session_store
                .get_session(&session)
                .unwrap()
                .project_id()
                .to_string();
            let evidence = crate::project_environment::ProjectEnvironmentEvidence::default();
            crate::project_environment::ProjectEnvironmentStore::new(
                &runtime
                    .owned
                    .config_projection
                    .snapshot()
                    .private_runtime_state_root(),
            )
            .save(&crate::project_environment::StoredProjectEnvironment {
                source: None,
                manifest: crate::project_environment::ProjectEnvironmentManifest {
                    schema_version: 1,
                    project_id,
                    evidence_digest: evidence.digest(),
                    entries: vec![],
                    private_files: vec![],
                    toolchain_hints: vec![],
                    package_hints: vec![],
                    service_hints: vec![],
                },
                evidence,
                reported_missing: Default::default(),
                reviewed_manifest: None,
                last_review: None,
            })
            .unwrap();
            runtime.with_external_command_authority(Some((&grant, &request)))
        };
        // The first request holds the real Vault deduplication mutex while its card waits.
        let first = if credential {
            Some(tokio::spawn({
                let runtime = runtime.clone();
                let session = session.clone();
                let agent = agent.clone();
                async move {
                    runtime
                        .ensure_vault_unlocked_for_agent(&session, &agent, "MP11 initial unlock")
                        .await
                        .map(|_| ())
                }
            }))
        } else {
            None
        };
        let pending_card = |runtime: &KernelRuntimeState| {
            runtime
                .owned
                .session_snapshot(&session)
                .unwrap()
                .active_interactions()
                .iter()
                .find(|card| card.id().starts_with("vault-unlock-"))
                .map(|card| card.id().to_string())
        };
        if credential {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while pending_card(&runtime).is_none() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        let entered = Arc::new(tokio::sync::Notify::new());
        let operation_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task = tokio::spawn({
            let entered = entered.clone();
            let operation_ran = operation_ran.clone();
            let session = session.clone();
            let agent = agent.clone();
            async move {
                entered.notify_one();
                if credential {
                    let _unlock = admitted
                        .ensure_vault_unlocked_for_agent(
                            &session,
                            &agent,
                            "MP11 credential release",
                        )
                        .await?;
                    admitted.with_forwarded_binding_operation(|| {
                        operation_ran.store(true, std::sync::atomic::Ordering::SeqCst);
                        Ok(())
                    })
                } else {
                    admitted
                        .with_project_prompt_environment(&session, &agent, |_| {
                            operation_ran.store(true, std::sync::atomic::Ordering::SeqCst);
                            Ok(())
                        })
                        .await
                }
            }
        });
        entered.notified().await;
        let card = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(card) = pending_card(&runtime) {
                    break card;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if credential {
            let mut replacement = binding;
            replacement.execution_lease_id = "revoked during Vault wait".into();
            runtime
                .owned
                .agent_store
                .bind_remote_execution(&agent, replacement)
                .unwrap();
        } else {
            runtime
                .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
                .unwrap();
        }
        let owner = runtime
            .owned
            .session_store
            .get_session(&session)
            .unwrap()
            .owner_user_id()
            .to_string();
        runtime
            .owned
            .resolve_runtime_interaction(
                &session,
                &card,
                "passphrase",
                Some("synthetic MP11 passphrase"),
                Some(&owner),
                true,
            )
            .unwrap();
        if let Some(first) = first {
            first.await.unwrap().unwrap();
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(10), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(
            !operation_ran.load(std::sync::atomic::Ordering::SeqCst),
            "resource operation ran after authority revocation"
        );
        crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn mp11_completion_receipt_cannot_settle_successor_or_rebound_prompt() {
        let (runtime, session, agent, attachment, binding) =
            mp11_fixture(crate::DaemonConfig::for_tests()).await;
        let first = mp11_submit(&runtime, &session, &agent, &attachment, "first");
        runtime
            .owned
            .complete_remote_prompt_owner_for_receipt(
                &session,
                &agent,
                "worker-run-1",
                None,
                None,
                Some((first.id(), &binding)),
            )
            .unwrap();
        runtime
            .owned
            .agent_store
            .bind_remote_execution(&agent, binding.clone())
            .unwrap();
        let second = mp11_submit(&runtime, &session, &agent, &attachment, "successor");
        assert!(runtime
            .owned
            .complete_remote_prompt_owner_for_receipt(
                &session,
                &agent,
                "worker-run-1",
                None,
                None,
                Some((first.id(), &binding))
            )
            .is_err());
        let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&snapshot, &agent)
                .unwrap()
                .id(),
            second.id()
        );
        let mut replacement = binding.clone();
        replacement.execution_lease_id = "new-lease".into();
        runtime
            .owned
            .agent_store
            .bind_remote_execution(&agent, replacement.clone())
            .unwrap();
        assert!(runtime
            .owned
            .complete_remote_prompt_owner_for_receipt(
                &session,
                &agent,
                "worker-run-1",
                None,
                None,
                Some((second.id(), &binding))
            )
            .is_err());
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent)
                .unwrap()
                .remote_execution(),
            Some(&replacement)
        );
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&snapshot, &agent)
                .unwrap()
                .id(),
            second.id()
        );
    }

    #[tokio::test]
    async fn mp11_completion_wait_barrier_preserves_independently_started_successor() {
        let (runtime, session, agent, attachment, binding) =
            mp11_fixture(crate::DaemonConfig::for_tests()).await;
        let first = mp11_submit(&runtime, &session, &agent, &attachment, "first");
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let receipt = tokio::spawn({
            let runtime = runtime.clone();
            let session = session.clone();
            let agent = agent.clone();
            let binding = binding.clone();
            let first = first.clone();
            let barrier = barrier.clone();
            async move {
                barrier.wait().await;
                barrier.wait().await;
                runtime.owned.complete_remote_prompt_owner_for_receipt(
                    &session,
                    &agent,
                    "worker-run-1",
                    None,
                    None,
                    Some((first.id(), &binding)),
                )
            }
        });
        barrier.wait().await;
        runtime
            .owned
            .complete_remote_prompt_owner_for_receipt(
                &session,
                &agent,
                "worker-run-1",
                None,
                None,
                Some((first.id(), &binding)),
            )
            .unwrap();
        runtime
            .owned
            .agent_store
            .bind_remote_execution(&agent, binding.clone())
            .unwrap();
        let successor = mp11_submit(
            &runtime,
            &session,
            &agent,
            &attachment,
            "successor while awaiting receipt",
        );
        barrier.wait().await;
        assert!(receipt.await.unwrap().is_err());
        let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&snapshot, &agent)
                .unwrap()
                .id(),
            successor.id()
        );
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent)
                .unwrap()
                .remote_execution(),
            Some(&binding)
        );
    }

    #[tokio::test]
    async fn mp11_forwarded_capability_admission_rejects_local_stale_and_foreign_session() {
        let (runtime, session, agent, attachment, binding) =
            mp11_fixture(crate::DaemonConfig::for_tests()).await;
        let prompt = mp11_submit(&runtime, &session, &agent, &attachment, "active");
        let admitted = runtime.with_relay_peer_authority(
            crate::runtime::relay_peer_authority::test_peer_authority("worker-kernel-1"),
        );
        let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
            home_kernel_id: runtime.owned.config_projection.snapshot().daemon_id,
            home_session_id: session.clone(),
            home_agent_id: agent.clone(),
            home_prompt_id: Some(prompt.id().into()),
            leased_agent_id: binding.leased_agent_id.clone(),
            worker_kernel_id: binding.worker_kernel_id.clone(),
            worker_machine_id: binding.worker_machine_id.clone(),
            worker_provider_run_id: "worker-run-1".into(),
            worker_worktree_path: "fixture".into(),
            worker_workspace_identity: crate::io::WorkspaceIdentity::local("fixture"),
        };
        for tool in [
            crate::transport::runtime_tools::LIST_SESSION_AGENTS_TOOL,
            "extension.list",
            "extension.register",
        ] {
            let request = |context| RelayPeerRequest::ForwardCapabilityRuntimeTool {
                context,
                tool_name: tool.into(),
                arguments: serde_json::json!({}),
            };
            assert!(admitted
                .prepare_forwarded_peer_request(&request(context.clone()))
                .await
                .is_ok());
            let mut stale = context.clone();
            stale.worker_provider_run_id = "stale-run".into();
            assert!(admitted
                .prepare_forwarded_peer_request(&request(stale))
                .await
                .is_err());
            let mut foreign = context.clone();
            foreign.home_session_id = "foreign-session".into();
            assert!(admitted
                .prepare_forwarded_peer_request(&request(foreign))
                .await
                .is_err());
            runtime
                .owned
                .agent_store
                .clear_remote_execution(&agent)
                .unwrap();
            assert!(admitted
                .prepare_forwarded_peer_request(&request(context.clone()))
                .await
                .is_err());
            runtime
                .owned
                .agent_store
                .bind_remote_execution(&agent, binding.clone())
                .unwrap();
        }
    }

    #[tokio::test]
    async fn mp11_immediate_steer_wrong_run_preserves_queue() {
        let (runtime, session, agent, attachment, _) =
            mp11_fixture(crate::DaemonConfig::for_tests()).await;
        mp11_submit(&runtime, &session, &agent, &attachment, "active");
        let queued = mp11_submit(&runtime, &session, &agent, &attachment, "queued");
        let prepared = runtime
            .owned
            .prepare_remote_queued_prompt_steer(&session, &agent, &attachment, queued.id(), None)
            .unwrap()
            .unwrap();
        let reservation = runtime
            .owned
            .reserve_remote_queued_prompt_steer(
                &session,
                &agent,
                &attachment,
                queued.id(),
                &prepared,
            )
            .unwrap();
        let prepared = runtime
            .owned
            .prepare_remote_queued_prompt_steer(
                &session,
                &agent,
                &attachment,
                queued.id(),
                Some(reservation),
            )
            .unwrap()
            .unwrap();
        assert!(runtime
            .owned
            .finish_remote_queued_prompt_steer(
                &session,
                &agent,
                &attachment,
                &prepared,
                &prepared.remote_execution,
                reservation,
                "unrelated-run"
            )
            .is_err());
        let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
        assert!(runtime
            .owned
            .prompt_state_owner
            .state_parts(&snapshot, &agent)
            .1
            .iter()
            .any(|prompt| prompt.id() == queued.id()));
    }

    #[tokio::test]
    async fn mp11_credential_release_revalidates_after_approval_wait() {
        let (runtime, session, agent, _, binding) =
            mp11_fixture(crate::DaemonConfig::for_tests()).await;
        let request = crate::transport::relay_peer::RelayPeerRequest::ResolveHomeCredentialSecret {
            context: crate::transport::relay_peer::RemoteExtensionInvocationContext {
                home_kernel_id: runtime.owned.config_projection.snapshot().daemon_id,
                home_session_id: session,
                home_agent_id: agent.clone(),
                leased_agent_id: binding.leased_agent_id.clone(),
                worker_kernel_id: Some(binding.worker_kernel_id.clone()),
                worker_machine_id: Some(binding.worker_machine_id.clone()),
                worker_provider_run_id: "worker-run-1".into(),
            },
            credential_id: "synthetic".into(),
            injection: crate::transport::relay_peer::RemoteCredentialSecretInjection::Pty,
        };
        let admitted = runtime
            .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
                "worker-kernel-1",
            ))
            .prepare_forwarded_peer_request(&request)
            .await
            .unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let operation_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task = tokio::spawn({
            let barrier = barrier.clone();
            let operation_ran = operation_ran.clone();
            async move {
                barrier.wait().await;
                barrier.wait().await;
                admitted.with_forwarded_binding_operation(|| {
                    operation_ran.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok("synthetic")
                })
            }
        });
        barrier.wait().await;
        let mut replacement = binding;
        replacement.execution_lease_id = "revoked-lease".into();
        runtime
            .owned
            .agent_store
            .bind_remote_execution(&agent, replacement)
            .unwrap();
        barrier.wait().await;
        assert!(task.await.unwrap().is_err());
        assert!(!operation_ran.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn mp11_project_environment_reauthorizes_after_app_mutex_wait() {
        let (runtime, session, agent, attachment, _) =
            mp11_fixture(crate::DaemonConfig::for_tests()).await;
        let grant = runtime.insert_access_grant_for_test(&session);
        let request =
            crate::local::LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
                session_id: session.clone(),
                attachment_id: attachment,
                target_agent_id: Some(agent.clone()),
                prompt: "revoked".into(),
                attachments: vec![],
            });
        let probe = Arc::new(tokio::sync::Notify::new());
        let mut admitted = runtime.with_external_command_authority(Some((&grant, &request)));
        admitted.app_lock_wait_probe = Some(probe.clone());
        let operation_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let app = runtime.app.lock().await;
        let task = tokio::spawn({
            let operation_ran = operation_ran.clone();
            let session = session.clone();
            async move {
                admitted
                    .with_project_prompt_environment(&session, &agent, |_| {
                        operation_ran.store(true, std::sync::atomic::Ordering::SeqCst);
                        Ok(())
                    })
                    .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), probe.notified())
            .await
            .unwrap();
        runtime
            .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
            .unwrap();
        drop(app);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(!operation_ran.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn remote_completion_with_queued_prompt_projects_combined_transition() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-remote-queue",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("worker-run-1".to_string()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let projection_store = app.session_state_projection_store();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;

        let first = PromptQueueItem::new(
            "pending:first",
            &attachment_id,
            &agent_id,
            "first remote prompt",
            PromptStatus::Queued,
        );
        let first_submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: first,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("first remote prompt should submit")
            .expect("remote prompt should be handled");
        let active_prompt_id = match first_submission.outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("first remote prompt should start")
            }
        };
        let queued = PromptQueueItem::new(
            "queued:second",
            &attachment_id,
            &agent_id,
            "second remote prompt",
            PromptStatus::Queued,
        );
        let queued_submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: queued,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("second remote prompt should submit")
            .expect("remote prompt should be handled");
        let queued_prompt = match queued_submission.outcome {
            crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
            crate::session::PromptSubmissionOutcome::Started { .. } => {
                panic!("second remote prompt should queue")
            }
        };
        let before_completion_sequence = projection_store.session_change_sequence(&session_id);

        let completion = runtime
            .owned
            .complete_remote_prompt_owner(
                &session_id,
                &agent_id,
                "worker-run-1",
                Some(&queued_prompt),
            )
            .expect("remote prompt completion should advance queue");

        assert_eq!(completion.completed.id(), active_prompt_id);
        let started_next = completion
            .started_next
            .expect("queued remote prompt should start");
        assert_ne!(started_next.id(), queued_prompt.id());
        assert_eq!(started_next.prompt(), queued_prompt.prompt());
        assert!(
            projection_store.session_change_sequence(&session_id) > before_completion_sequence,
            "remote queued advancement should refresh the session projection"
        );
        let projected = projection_store
            .get(&session_id)
            .expect("session projection should refresh");
        let active = projected
            .active_prompt_for_agent(&agent_id)
            .expect("next remote prompt should project as active");
        assert_eq!(active.id(), started_next.id());
        assert_eq!(active.prompt(), "second remote prompt");
        assert!(projected
            .queued_prompts_for_agent(&agent_id)
            .is_some_and(|queue| queue.is_empty()));
    }

    #[tokio::test]
    async fn failed_remote_completion_keeps_the_worker_run_binding() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-settlement-order",
                "worktree-settlement-order",
            ))
            .expect("session should be created");
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-2".to_string(),
                    worker_machine_id: "worker-machine-2".to_string(),
                    execution_lease_id: "lease-2".to_string(),
                    leased_agent_id: "leased-agent-2".to_string(),
                    active_worker_provider_run_id: Some("provider-run-2".to_string()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime
            .owned
            .session_store
            .delete_session(&session_id)
            .expect("test should remove the session before settlement");

        runtime
            .owned
            .complete_remote_prompt_owner(&session_id, &agent_id, "provider-run-2", None)
            .expect_err("completion without its session must fail");

        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("agent should remain available")
                .remote_execution()
                .and_then(|binding| binding.active_worker_provider_run_id.as_deref()),
            Some("provider-run-2"),
            "a failed settlement must not clear the last drainable worker run",
        );
    }

    #[tokio::test]
    async fn stopped_slice_prompt_settles_with_one_visible_durable_error() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-stopped-slice",
                "worktree-stopped-slice",
            ))
            .expect("session should be created");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-stopped-slice",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-stopped".to_string(),
                    worker_machine_id: "worker-machine-stopped".to_string(),
                    execution_lease_id: "lease-stopped".to_string(),
                    leased_agent_id: "leased-agent-stopped".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        let slice = app
            .slices()
            .create(
                "owner-kernel",
                "owner-machine",
                crate::slice::CreateSliceInput {
                    source_slice_ref: None,
                    name: "stopped-slice".to_string(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".to_string(),
                    display_mode: crate::slice::SliceDisplayMode::Headless,
                    display_backend: Default::default(),
                    workspace_id: Some("workspace-stopped-slice".to_string()),
                    worktree_id: Some("worktree-stopped-slice".to_string()),
                    workspace_mount: None,
                    development: None,
                    worker_kernel_ref: Some("slice:stopped-slice".to_string()),
                    display_url: None,
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 1,
                },
            )
            .expect("slice should be created stopped");
        app.slices()
            .attach_agent(&slice.id, session.id(), agent.id(), 2)
            .expect("agent should attach to slice");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;

        let submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: PromptQueueItem::new(
                    "pending:stopped",
                    attachment.id(),
                    &agent_id,
                    "prompt for stopped slice",
                    PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .expect("stopped slice prompt should be admitted locally")
            .expect("remote prompt should be handled");
        let mut dispatch = submission
            .remote_dispatch
            .expect("admitted stopped-slice prompt should reach remote dispatch settlement");
        let dispatch_error =
            super::remote_prompt_worker_submission_runtime::submit_remote_prompt_to_worker_with_binding_refresh(
                &runtime,
                &mut dispatch,
                "prompt for stopped slice".to_string(),
                Vec::new(),
            )
            .await
            .expect_err("stopped slice must fail before relay transport");

        assert!(dispatch_error
            .to_string()
            .contains("stopped slice `stopped-slice`"));
        runtime
            .finish_remote_prompt_dispatch(dispatch, Err(dispatch_error))
            .await
            .expect_err("authoritative dispatch failure must remain visible to the caller");
        assert_eq!(
            runtime
                .owned
                .slice_store
                .resolve(&slice.id)
                .expect("slice should remain available")
                .status,
            crate::slice::SliceStatus::Stopped,
        );
        let output_records = runtime
            .owned
            .terminal_stream
            .drain_output_records(&session_id, attachment.id());
        let errors = output_records
            .iter()
            .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderError)
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 1, "live trace must show the rejection once");
        assert!(String::from_utf8_lossy(&errors[0].bytes).contains("stopped slice `stopped-slice`"));
        let durable_events = runtime
            .owned
            .operational_history_store
            .load_session_events(&session_id, Some(&agent_id))
            .expect("durable agent history should load");
        assert_eq!(
            durable_events
                .iter()
                .filter(|event| event.kind == crate::history::HistoryEventKind::ProviderError)
                .count(),
            1,
            "refresh history must contain exactly one visible error",
        );
        assert!(!durable_events.iter().any(|event| {
            event.kind == crate::history::HistoryEventKind::Notice
                && event
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains("Remote prompt dispatch failed"))
        }));
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .expect("session should remain available"),
                &agent_id,
            )
            .is_none());
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("agent should remain available")
                .state(),
            crate::agent::AgentState::Error,
        );
        let refreshed = runtime
            .owned
            .session_snapshot(&session_id)
            .expect("failed stopped-slice session should remain refreshable");
        let refreshed_agent = refreshed
            .agents()
            .iter()
            .find(|candidate| candidate.id() == agent_id)
            .expect("remote agent should remain projected after refresh");
        assert_eq!(refreshed_agent.state(), crate::agent::AgentState::Error);
        assert_eq!(
            refreshed_agent
                .remote_execution()
                .map(|remote| remote.worker_kernel_id.as_str()),
            Some("worker-kernel-stopped"),
        );
    }

    #[tokio::test]
    async fn worker_settled_remote_cancellation_clears_the_home_active_prompt() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-cancel",
                "worktree-cancel",
            ))
            .expect("session should be created");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-remote-cancel",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-cancel".to_string(),
                    worker_machine_id: "worker-machine-cancel".to_string(),
                    execution_lease_id: "lease-cancel".to_string(),
                    leased_agent_id: "leased-agent-cancel".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let prompt = PromptQueueItem::new(
            "pending:cancel",
            &attachment_id,
            &agent_id,
            "stale remote prompt",
            PromptStatus::Queued,
        );
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("remote prompt should submit")
            .expect("remote prompt should be handled");

        let cancellation = runtime
            .owned
            .finalize_remote_prompt_cancellation_after_worker_settled(
                &session_id,
                &agent_id,
                &attachment_id,
            )
            .expect("settled worker cancellation should finalize at home");

        assert_eq!(
            cancellation.cancellation.prompt.status(),
            PromptStatus::Cancelled
        );
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .expect("session should remain available"),
                &agent_id,
            )
            .is_none());
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .expect("agent should remain available")
                .state(),
            crate::agent::AgentState::Focused
        );
    }
}
