use crate::agent::RemoteAgentBinding;
use crate::app::{DaemonApp, KernelRemotePromptDispatch};
use crate::error::DaemonError;
use crate::session::{PromptCancellation, PromptCompletion, PromptQueueItem};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

use super::super::KernelAgentService;
use super::completion::{KernelPromptCompletionAdmission, KernelPromptOwnerCompletion};

fn remote_workspace_live_sync_mode_for_agent(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> Option<crate::config::WorkspaceLiveSyncMode> {
    let session = app.sessions().get_session(session_id).ok()?;
    let agent = app.agents().get_agent(agent_id).ok()?;
    Some(
        crate::provider::provider_workspace_live_sync_mode_for_session(
            agent.provider(),
            app.config(),
            Some(&session),
        ),
    )
}

fn remote_git_turn_context_for_prompt(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
    prompt: &PromptQueueItem,
    home_prompt_id: &str,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: session_id.to_string(),
        home_agent_id: agent_id.to_string(),
        home_prompt_id: home_prompt_id.to_string(),
        home_turn_id: home_prompt_id.to_string(),
        source_attachment_id: Some(prompt.source_attachment_id().to_string()),
        workspace_live_sync_mode: remote_workspace_live_sync_mode_for_agent(
            app, session_id, agent_id,
        ),
        prompt_origin: Some(prompt.prompt_origin()),
        external_provider: prompt.external_provider().map(str::to_string),
        external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
        external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            prompt.prompt(),
            prompt.attachments(),
        ),
    }
}

fn remote_prompt_error_is_already_settled(error: &DaemonError) -> bool {
    match error {
        DaemonError::NoActivePrompt { .. } => true,
        DaemonError::LocalTransport { message, .. } => {
            message.contains("no active prompt")
                || message.contains("NoActivePrompt")
                || message.contains("no_active_prompt")
        }
        _ => false,
    }
}

impl<'a> KernelAgentService<'a> {
    pub(super) fn cancel_remote_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
        active_prompt: &PromptQueueItem,
        remote_execution: RemoteAgentBinding,
    ) -> Result<PromptCancellation, DaemonError> {
        let expected_worker_provider_run_id = active_prompt.durable_delivery_provider_run_id();
        if active_prompt.durable_delivery_phase()
            != Some(crate::session::DurablePromptDeliveryPhase::Delivered)
            || expected_worker_provider_run_id.is_none()
            || remote_execution.active_worker_provider_run_id.as_deref()
                != expected_worker_provider_run_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: "remote prompt has no matching durable worker-run receipt".to_string(),
            });
        }
        if !remote_execution.relay_peer_protocol_compatible() {
            return Err(DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: "worker relay peer protocol does not support run-scoped cancellation"
                    .to_string(),
            });
        }
        let expected_worker_provider_run_id = expected_worker_provider_run_id
            .expect("checked above")
            .to_string();
        let relay_config = self
            .app
            .relay_config_for_remote_execution(&remote_execution);
        let cancellation_response =
            self.app
                .block_on_relay_future(send_peer_request_via_temporary_connection(
                    &relay_config,
                    ClientTarget {
                        daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::CancelLeasedPrompt {
                        leased_agent_id: remote_execution.leased_agent_id.clone(),
                        home_prompt_id: active_prompt.id().to_string(),
                        worker_provider_run_id: expected_worker_provider_run_id,
                    },
                ));
        match cancellation_response {
            Ok(RelayPeerResponse::LeasedPromptCancelled { .. }) => {}
            Ok(other) => {
                return Err(DaemonError::LocalTransport {
                    operation: "cancel remote prompt",
                    message: format!("unexpected remote prompt cancellation response: {other:?}"),
                });
            }
            Err(error) if remote_prompt_error_is_already_settled(&error) => {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt cancellation already settled on worker",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "worker_kernel_id": remote_execution.worker_kernel_id,
                        "leased_agent_id": remote_execution.leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
                return self.finalize_active_prompt_cancellation(session_id, agent_id, None);
            }
            Err(error) => return Err(error),
        };
        let prompt = self
            .app
            .prompt_owner_begin_cancelling_active_prompt(session_id, agent_id)?;
        let recipients = match attachment_id {
            Some(attachment_id) => self.app.other_attachment_ids(session_id, attachment_id),
            None => self.app.attachments.list_session_attachment_ids(session_id),
        };
        let message = match attachment_id {
            Some(attachment_id) => format!(
                "Attachment `{attachment_id}` requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                active_prompt.id(),
                remote_execution.worker_kernel_id
            ),
            None => format!(
                "Chariox requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                active_prompt.id(),
                remote_execution.worker_kernel_id
            ),
        };
        self.app
            .record_notice(session_id, None, recipients, message);
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
        Ok(PromptCancellation {
            prompt,
            started_next: None,
        })
    }

    pub(super) fn defer_compat_remote_prompt_dispatch(
        &mut self,
        dispatch: Option<KernelRemotePromptDispatch>,
    ) -> Result<(), DaemonError> {
        let Some(dispatch) = dispatch else {
            return Ok(());
        };
        // The app-side-effect wrapper drains this queue after releasing the
        // DaemonApp mutex and hands the item to the kernel remote dispatcher.
        self.app
            .defer_remote_prompt_dispatch_after_app_side_effect(dispatch);
        Ok(())
    }

    pub(super) fn complete_remote_prompt_from_admission(
        &mut self,
        admission: KernelPromptCompletionAdmission,
    ) -> Result<KernelPromptOwnerCompletion, DaemonError> {
        let KernelPromptCompletionAdmission::Remote {
            session_id,
            agent_id,
            remote_execution,
            next_queued_prompt,
        } = admission
        else {
            return Err(DaemonError::LocalTransport {
                operation: "complete prompt admission",
                message: "expected remote prompt completion admission".to_string(),
            });
        };

        let relay_config = self
            .app
            .relay_config_for_remote_execution(&remote_execution);
        let completion_response =
            self.app
                .block_on_relay_future(send_peer_request_via_temporary_connection(
                    &relay_config,
                    ClientTarget {
                        daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::CompleteLeasedPrompt {
                        leased_agent_id: remote_execution.leased_agent_id.clone(),
                    },
                ));
        let (remote_provider_run_id, provider_termination) = match completion_response {
            Ok(response) => match response {
                RelayPeerResponse::LeasedPromptCompleted {
                    provider_run_id,
                    provider_termination,
                    git_observations,
                    workspace_live_sync_change,
                    ..
                } => {
                    let _ = crate::git_observer::append_observations(
                        &self.app.operational_history_store(),
                        git_observations,
                    )?;
                    if let Some(change) = workspace_live_sync_change {
                        self.app.fanout_remote_workspace_live_sync_change(
                            change,
                            Some(&remote_execution.worker_kernel_id),
                        );
                    }
                    (provider_run_id, provider_termination)
                }
                other => {
                    return Err(DaemonError::LocalTransport {
                        operation: "complete remote prompt",
                        message: format!("unexpected remote prompt completion response: {other:?}"),
                    });
                }
            },
            Err(error) if remote_prompt_error_is_already_settled(&error) => {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt completion already settled on worker",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "worker_kernel_id": remote_execution.worker_kernel_id,
                        "leased_agent_id": remote_execution.leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
                (None, None)
            }
            Err(error) => return Err(error),
        };
        let completed = self
            .app
            .prompt_owner_complete_active_prompt_only(&session_id, &agent_id)?;
        let _ = self
            .app
            .agents()
            .set_remote_execution_active_worker_provider_run_id(&agent_id, None)?;
        let settlement_status = if provider_termination.is_some() {
            crate::git_observer::CompletedTurnSettlementStatus::Failed
        } else {
            crate::git_observer::CompletedTurnSettlementStatus::Completed
        };
        Ok(KernelPromptOwnerCompletion {
            session_id,
            agent_id,
            completed,
            provider_run_id: None,
            remote_execution: Some(remote_execution),
            remote_provider_run_id,
            next_queued_prompt,
            settlement_status,
            provider_termination,
        })
    }

    pub(super) fn finish_remote_prompt_completion(
        &mut self,
        completion: KernelPromptOwnerCompletion,
    ) -> Result<PromptCompletion, DaemonError> {
        if completion.provider_termination.is_some() {
            let _ = self
                .app
                .agents()
                .mark_unexpected_provider_exit_error(&completion.agent_id, true);
        }
        let remote_provider_run_id = remote_completion_provider_run_id(
            completion.remote_execution.as_ref(),
            completion.remote_provider_run_id.as_deref(),
        );
        let settled_at_ms = crate::session::unix_epoch_ms();
        let started_at_ms = self
            .app
            .active_turn_store()
            .get(&remote_provider_run_id)
            .map(|turn| turn.started_at_ms)
            .or(Some(completion.completed.created_at_ms()));
        self.app
            .operational_history_store()
            .record_prompt_settlement(
                self.app.history_archive_enabled(),
                &completion.session_id,
                &completion.agent_id,
                completion.completed.id(),
                Some(&remote_provider_run_id),
                settled_at_ms,
                completion.settlement_status.as_str(),
            );
        self.app
            .completed_git_turn_snapshot_store()
            .record_prompt_settlement_with_termination(
                &completion.session_id,
                &completion.agent_id,
                &remote_provider_run_id,
                &completion.completed,
                settled_at_ms,
                started_at_ms,
                completion.settlement_status,
                completion.provider_termination.clone(),
            );
        let recipient_attachment_ids = self
            .app
            .attachments
            .list_session_attachment_ids(&completion.session_id);
        self.record_assistant_message_completion(
            &completion.session_id,
            &remote_provider_run_id,
            recipient_attachment_ids,
            &format!("prompt-complete:{}", completion.completed.id()),
            settled_at_ms,
        );
        let started_next = if self
            .app
            .prompt_owner_active_prompt_for_agent(&completion.session_id, &completion.agent_id)?
            .is_none()
        {
            let remote_execution = completion.remote_execution.as_ref().ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "complete remote prompt",
                    message: "missing remote execution binding".to_string(),
                }
            })?;
            self.advance_next_queued_prompt_remote(
                &completion.session_id,
                &completion.agent_id,
                &remote_execution.worker_kernel_id,
                &remote_execution.leased_agent_id,
                remote_execution.relay_url.as_deref(),
                remote_execution.relay_token.as_deref(),
                completion.next_queued_prompt.as_ref(),
            )?
        } else {
            None
        };
        if started_next.is_none() {
            self.app
                .sync_focused_provider_run_if_idle(&completion.session_id)?;
        }
        crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(&completion.session_id)?;

        let prompt_completion = PromptCompletion {
            completed: completion.completed,
            started_next,
        };
        self.inject_orphaned_metaagent_task_event_after_turn(
            &completion.agent_id,
            &prompt_completion,
        )?;
        Ok(prompt_completion)
    }

    pub(crate) fn advance_next_queued_prompt_remote(
        &mut self,
        session_id: &str,
        agent_id: &str,
        worker_kernel_id: &str,
        leased_agent_id: &str,
        relay_url: Option<&str>,
        relay_token: Option<&str>,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        self.advance_next_queued_prompt_remote_inner(
            session_id,
            agent_id,
            worker_kernel_id,
            leased_agent_id,
            relay_url,
            relay_token,
            expected_next,
        )
    }

    pub(crate) fn advance_next_queued_prompt_remote_with_workflow_dispatch(
        &mut self,
        session_id: &str,
        agent_id: &str,
        worker_kernel_id: &str,
        leased_agent_id: &str,
        relay_url: Option<&str>,
        relay_token: Option<&str>,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        self.advance_next_queued_prompt_remote_inner(
            session_id,
            agent_id,
            worker_kernel_id,
            leased_agent_id,
            relay_url,
            relay_token,
            expected_next,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn advance_next_queued_prompt_remote_inner(
        &mut self,
        session_id: &str,
        agent_id: &str,
        worker_kernel_id: &str,
        leased_agent_id: &str,
        relay_url: Option<&str>,
        relay_token: Option<&str>,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut expected_next = expected_next.cloned();
        loop {
            let next_candidate =
                self.next_queued_prompt_candidate(session_id, agent_id, expected_next.as_ref())?;
            let Some(peeked) = next_candidate else {
                return Ok(None);
            };
            let is_workflow_prompt = crate::app::workflow_runtime::is_workflow_prompt_source(
                peeked.source_attachment_id(),
            );
            if let Err(error) = crate::app::KernelSessionReadService::new(self.app)
                .ensure_attachment_in_session(session_id, peeked.source_attachment_id())
            {
                if !is_workflow_prompt {
                    self.remove_detached_queued_remote_prompt(
                        session_id, agent_id, &peeked, error,
                    )?;
                    expected_next = None;
                    continue;
                }
            }
            if !is_workflow_prompt {
                let Some((active, mut dispatch_intent)) = self.admit_next_queued_remote_prompt(
                    session_id,
                    agent_id,
                    expected_next.as_ref(),
                )?
                else {
                    return Ok(None);
                };
                if let (Some(relay_url), Some(relay_token)) = (relay_url, relay_token) {
                    dispatch_intent.dispatch.relay_url = Some(relay_url.to_string());
                    dispatch_intent.dispatch.relay_token = Some(relay_token.to_string());
                }
                self.app
                    .defer_remote_prompt_dispatch_after_app_side_effect(dispatch_intent.dispatch);
                return Ok(Some(active));
            }
            let agent = self.app.agents().get_agent(agent_id)?;
            let remote_execution =
                agent
                    .remote_execution()
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "advance remote queued prompt",
                        message: format!("agent `{agent_id}` lost its remote binding"),
                    })?;
            self.app
                .ensure_remote_agent_binding_protocol(remote_execution)?;
            let workflow_context = crate::app::RemoteWorkflowTurnContextResolver::new(self.app)
                .remote_workflow_turn_context_for_prompt(session_id, agent_id, &peeked)?;
            let home_prompt_id = self.app.sessions_mut().reserve_prompt_id();
            let (_session, next_candidate) = self
                .activate_next_queued_prompt_for_mirror_with_prompt_id(
                    session_id,
                    agent_id,
                    Some(&peeked),
                    home_prompt_id,
                )?;
            let Some(active) = next_candidate else {
                continue;
            };
            let active =
                self.prepare_promoted_queued_prompt_start(session_id, agent_id, active.id())?;
            let session = self.app.sessions().get_session(session_id)?;
            self.app.defer_remote_prompt_dispatch_after_app_side_effect(
                crate::app::KernelRemotePromptDispatch {
                    session_id: session_id.to_string(),
                    agent_id: agent_id.to_string(),
                    prompt_id: active.id().to_string(),
                    worker_kernel_id: worker_kernel_id.to_string(),
                    leased_agent_id: leased_agent_id.to_string(),
                    relay_url: relay_url.map(str::to_string),
                    relay_token: relay_token.map(str::to_string),
                    source_attachment_id: active.source_attachment_id().to_string(),
                    prompt: active.prompt().to_string(),
                    hidden_system_context: active.hidden_system_context().to_string(),
                    attachments: active.attachments().to_vec(),
                    workspace_live_sync_mode: Some(
                        crate::provider::provider_workspace_live_sync_mode_for_session(
                            agent.provider(),
                            self.app.config(),
                            Some(&session),
                        ),
                    ),
                    prompt_origin: active.prompt_origin(),
                    external_provider: active.external_provider().map(str::to_string),
                    external_provider_session_id: active
                        .external_provider_session_id()
                        .map(str::to_string),
                    external_provider_turn_id: active
                        .external_provider_turn_id()
                        .map(str::to_string),
                    workflow_context: Some(workflow_context),
                },
            );
            return Ok(Some(active));
        }
    }

    /// Admit an ordinary queued prompt before handing its delivery intent to the
    /// runtime. The caller must dispatch the returned intent after releasing the
    /// app lock so binding refreshes and prompt delivery share the ordered lane.
    pub(crate) fn admit_next_queued_remote_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<
        Option<(
            PromptQueueItem,
            crate::app::KernelRemotePromptDispatchIntent,
        )>,
        DaemonError,
    > {
        let mut expected_next = expected_next.cloned();
        loop {
            let Some(candidate) =
                self.next_queued_prompt_candidate(session_id, agent_id, expected_next.as_ref())?
            else {
                return Ok(None);
            };
            // Workflow queue advancement remains owned by the workflow scheduler.
            if crate::app::workflow_runtime::is_workflow_prompt_source(
                candidate.source_attachment_id(),
            ) {
                return Ok(None);
            }
            if let Err(error) = crate::app::KernelSessionReadService::new(self.app)
                .ensure_attachment_in_session(session_id, candidate.source_attachment_id())
            {
                self.remove_detached_queued_remote_prompt(session_id, agent_id, &candidate, error)?;
                expected_next = None;
                continue;
            }
            let agent = self.app.agents.get_agent(agent_id)?;
            let remote =
                agent
                    .remote_execution()
                    .cloned()
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "admit remote queued prompt",
                        message: format!("agent `{agent_id}` lost its remote binding"),
                    })?;
            self.app.ensure_remote_agent_binding_protocol(&remote)?;
            let prompt_id = self.app.sessions_mut().reserve_prompt_id();
            let (_, Some(active)) = self.activate_next_queued_prompt_for_mirror_with_prompt_id(
                session_id,
                agent_id,
                Some(&candidate),
                prompt_id,
            )?
            else {
                continue;
            };
            let active =
                self.prepare_promoted_remote_prompt_start(session_id, agent_id, &active)?;
            let workflow_context = if crate::app::workflow_runtime::is_workflow_prompt_source(
                active.source_attachment_id(),
            ) {
                Some(
                    crate::app::RemoteWorkflowTurnContextResolver::new(self.app)
                        .remote_workflow_turn_context_for_prompt(session_id, agent_id, &active)?,
                )
            } else {
                None
            };
            let dispatch = KernelRemotePromptDispatch {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                prompt_id: active.id().to_string(),
                worker_kernel_id: remote.worker_kernel_id,
                leased_agent_id: remote.leased_agent_id,
                relay_url: remote.relay_url,
                relay_token: remote.relay_token,
                source_attachment_id: active.source_attachment_id().to_string(),
                prompt: active.prompt().to_string(),
                hidden_system_context: active.hidden_system_context().to_string(),
                attachments: active.attachments().to_vec(),
                workspace_live_sync_mode: remote_workspace_live_sync_mode_for_agent(
                    self.app, session_id, agent_id,
                ),
                prompt_origin: active.prompt_origin(),
                external_provider: active.external_provider().map(str::to_string),
                external_provider_session_id: active
                    .external_provider_session_id()
                    .map(str::to_string),
                external_provider_turn_id: active.external_provider_turn_id().map(str::to_string),
                workflow_context,
            };
            return Ok(Some((
                active,
                crate::app::KernelRemotePromptDispatchIntent { dispatch },
            )));
        }
    }

    fn remove_detached_queued_remote_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        prompt: &PromptQueueItem,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        let session = self.app.sessions.get_session(session_id)?;
        let removed =
            self.app
                .prompt_state_owner()
                .remove_queued_prompt(&session, agent_id, prompt.id());
        if removed.is_some() {
            self.app.record_notice(
                session_id,
                None,
                self.app.attachments.list_session_attachment_ids(session_id),
                format!(
                    "Skipped queued prompt `{}` because its source attachment is no longer active: {}",
                    prompt.id(),
                    error
                ),
            );
            self.app
                .mirror_prompt_owner_agent_state(session_id, agent_id)?;
        }
        Ok(())
    }

    fn prepare_promoted_remote_prompt_start(
        &mut self,
        session_id: &str,
        agent_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<PromptQueueItem, DaemonError> {
        let active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active.id() != prompt.id() {
            return Err(DaemonError::LocalTransport {
                operation: "admit remote queued prompt",
                message: format!(
                    "expected prompt `{}` but activated `{}`",
                    prompt.id(),
                    active.id()
                ),
            });
        }
        let source_attachment_id = self
            .app
            .promoted_prompt_source_attachment_id(session_id, active.source_attachment_id())?;
        let history_text = crate::prompt_transcript::workflow_prompt_history_text(&active);
        let sent_at_ms = crate::session::unix_epoch_ms();
        self.app.spawn_user_prompt_history_append_with_prompt_id(
            session_id,
            &source_attachment_id,
            active.target_agent_id(),
            &history_text,
            active.attachments(),
            active.prompt_origin(),
            active.id(),
            sent_at_ms,
            active.workflow_run_id(),
            active.workflow_node_run_id(),
        )?;
        self.app.agents.note_prompt_sent_at(agent_id, sent_at_ms)?;
        self.app
            .sessions
            .note_prompt_sent(session_id, agent_id, sent_at_ms)?;
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
        Ok(active)
    }
}

fn remote_completion_provider_run_id(
    remote_execution: Option<&RemoteAgentBinding>,
    completed_worker_provider_run_id: Option<&str>,
) -> String {
    let Some(remote_execution) = remote_execution else {
        return completed_worker_provider_run_id
            .unwrap_or("remote-provider-run-completed")
            .to_string();
    };
    let worker_provider_run_id = completed_worker_provider_run_id
        .or(remote_execution.active_worker_provider_run_id.as_deref());
    worker_provider_run_id
        .map(|worker_provider_run_id| {
            crate::provider::projected_leased_provider_run_id(
                &remote_execution.leased_agent_id,
                worker_provider_run_id,
            )
        })
        .unwrap_or_else(|| "remote-provider-run-completed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::DaemonConfig;
    use crate::session::{CreateSessionRequest, PromptStatus, PromptSubmissionOutcome};

    #[test]
    fn remote_completion_uses_the_home_projected_provider_run_id() {
        let binding = RemoteAgentBinding {
            worker_kernel_id: "worker-kernel".to_string(),
            worker_machine_id: "slice:slice-1".to_string(),
            leased_agent_id: "leased-agent-1".to_string(),
            execution_lease_id: "lease-1".to_string(),
            active_worker_provider_run_id: Some("worker-run-old".to_string()),
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };

        assert_eq!(
            remote_completion_provider_run_id(Some(&binding), Some("worker-run-1")),
            "leased:leased-agent-1:worker-run-1",
        );
    }

    #[test]
    fn queued_remote_prompt_persists_reserved_home_prompt_after_activation() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let first_prompt_id = app.sessions_mut().reserve_prompt_id();
        let first = PromptQueueItem::new(
            first_prompt_id,
            attachment.id(),
            agent.id(),
            "first",
            PromptStatus::Queued,
        );
        assert!(matches!(
            app.prompt_owner_submit_prepared_prompt(session.id(), first, false)
                .expect("first prompt should submit"),
            PromptSubmissionOutcome::Started { .. }
        ));
        let second_prompt_id = app.sessions_mut().reserve_prompt_id();
        let second = PromptQueueItem::new(
            second_prompt_id,
            attachment.id(),
            agent.id(),
            "second",
            PromptStatus::Queued,
        );
        let PromptSubmissionOutcome::Queued { prompt: queued } = app
            .prompt_owner_submit_prepared_prompt(session.id(), second, false)
            .expect("second prompt should queue")
        else {
            panic!("second prompt should remain queued")
        };
        app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
            .expect("first prompt should complete without promoting the queue");
        let peeked = app
            .prompt_owner_peek_next_queued_prompt(session.id(), agent.id())
            .expect("queued prompt should load")
            .expect("second prompt should remain queued");
        assert_eq!(peeked.id(), queued.id());

        let canonical_prompt_id = app.sessions_mut().reserve_prompt_id();
        let context = remote_git_turn_context_for_prompt(
            &app,
            session.id(),
            agent.id(),
            &peeked,
            &canonical_prompt_id,
        );
        let (_session, active) = KernelAgentService::new(&mut app)
            .activate_next_queued_prompt_for_mirror_with_prompt_id(
                session.id(),
                agent.id(),
                Some(&peeked),
                canonical_prompt_id,
            )
            .expect("queued prompt should activate with the reserved id");
        let active = active.expect("queued prompt should become active");
        let expected_merge_key = format!("prompt:{}", active.id());
        assert_eq!(
            app.operational_history_store()
                .load_session_events(session.id(), Some(agent.id()))
                .expect("operational history should load before delivery finishes")
                .into_iter()
                .filter(|event| {
                    event
                        .metadata
                        .get("merge_key")
                        .and_then(serde_json::Value::as_str)
                        == Some(expected_merge_key.as_str())
                })
                .count(),
            0,
            "activation alone must not persist a prompt that has not finished delivery"
        );
        let active = KernelAgentService::new(&mut app)
            .finish_promoted_queued_prompt_start(
                session.id(),
                "worker-provider-run-2",
                agent.id(),
                active.id(),
            )
            .expect("queued remote prompt should finish activation");

        assert_ne!(queued.id(), active.id());
        assert_eq!(context.home_prompt_id, active.id());
        assert_eq!(context.home_turn_id, active.id());
        assert_eq!(active.status(), PromptStatus::Running);
        let history = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("promoted prompt history should load");
        let entry = history
            .iter()
            .find(|entry| entry.text.contains("second"))
            .expect("promoted prompt should persist in home history");
        assert_eq!(entry.source_attachment_id.as_deref(), Some(attachment.id()));
        assert_eq!(
            entry.merge_key.as_deref(),
            Some(expected_merge_key.as_str())
        );
        assert_eq!(
            entry.prompt_origin,
            Some(crate::session::PromptOrigin::Chariox)
        );
        assert_eq!(
            app.operational_history_store()
                .load_session_events(session.id(), Some(agent.id()))
                .expect("operational history should load after delivery finishes")
                .into_iter()
                .filter(|event| {
                    event
                        .metadata
                        .get("merge_key")
                        .and_then(serde_json::Value::as_str)
                        == Some(expected_merge_key.as_str())
                })
                .count(),
            1,
            "successful remote promotion must persist one canonical prompt event"
        );
        assert!(app
            .agents()
            .get_agent(agent.id())
            .expect("agent should load")
            .last_prompt_sent_at_ms()
            .is_some());
    }

    #[test]
    fn compatibility_completion_defers_promoted_remote_prompt_once() {
        let mut app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "compat-remote-completion",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should create");
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");

        let first = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "first",
            PromptStatus::Queued,
        );
        assert!(matches!(
            app.prompt_owner_submit_prepared_prompt(session.id(), first, false)
                .expect("first prompt should start"),
            PromptSubmissionOutcome::Started { .. }
        ));
        let queued = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "queued second",
            PromptStatus::Queued,
        );
        let queued = match app
            .prompt_owner_submit_prepared_prompt(session.id(), queued, false)
            .expect("second prompt should queue")
        {
            PromptSubmissionOutcome::Queued { prompt } => prompt,
            PromptSubmissionOutcome::Started { .. } => panic!("second prompt must queue"),
        };
        app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
            .expect("first prompt should complete before queue promotion");

        let promoted = KernelAgentService::new(&mut app)
            .advance_next_queued_prompt_remote(
                session.id(),
                agent.id(),
                "worker-kernel-1",
                "leased-agent-1",
                None,
                None,
                Some(&queued),
            )
            .expect("completion should not wait for relay I/O under the app lock")
            .expect("queued prompt should promote");
        assert_eq!(promoted.prompt(), "queued second");
        assert_eq!(promoted.status(), PromptStatus::Dispatching);

        let deferred = app.take_deferred_workflow_remote_prompt_dispatches();
        assert_eq!(
            deferred.len(),
            1,
            "promotion should enqueue exactly one send"
        );
        assert_eq!(deferred[0].prompt_id, promoted.id());
        assert_eq!(deferred[0].leased_agent_id, "leased-agent-1");
        assert!(app
            .take_deferred_workflow_remote_prompt_dispatches()
            .is_empty());
    }
}
