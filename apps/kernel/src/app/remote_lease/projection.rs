use crate::app::provider_output;
use crate::error::DaemonError;
use crate::history::SessionHistoryEntry;
use crate::terminal::TerminalOutputKind;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayProjectedCompletion, RelayProjectedOutputChunk,
};

use super::RemoteLeaseRuntime;

impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn drain_leased_runtime_projection(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
    ) -> Result<Option<(String, RelayPeerEvent)>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        if pump_output {
            let _ = provider_output::pump_terminal_output_for_attachment(
                self.app,
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )?;
        }
        let output_chunks = self
            .app
            .terminal
            .drain_output_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .filter(|record| record.provider_run_id == provider_run_id)
            .map(|record| RelayProjectedOutputChunk {
                kind: record.kind,
                merge_key: record.merge_key,
                bytes: record.bytes,
            })
            .collect::<Vec<_>>();
        let notices = self
            .app
            .terminal
            .drain_notice_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .filter(|record| {
                record
                    .provider_run_id
                    .as_deref()
                    .is_none_or(|record_provider_run_id| record_provider_run_id == provider_run_id)
            })
            .map(|record| record.message)
            .collect::<Vec<_>>();
        let mut completions = self
            .app
            .terminal
            .drain_completion_records(
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )
            .into_iter()
            .filter(|record| record.provider_run_id == provider_run_id)
            .map(|record| RelayProjectedCompletion {
                message_id: record.message_id,
                completed_at_ms: record.completed_at_ms,
            })
            .collect::<Vec<_>>();
        let backing_prompt_active = self
            .app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )?
            .is_some();
        let completion_already_projected = leased_agent
            .projected_completion_provider_run_ids
            .iter()
            .any(|id| id == provider_run_id);
        if completions.is_empty() && !backing_prompt_active && !completion_already_projected {
            completions.push(RelayProjectedCompletion {
                message_id: format!("leased-{provider_run_id}-completion"),
                completed_at_ms: crate::session::unix_epoch_ms(),
            });
        }
        if !completions.is_empty() {
            if backing_prompt_active {
                let _ = self.app.complete_active_prompt(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                    Some(provider_run_id),
                )?;
            }
            if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                if !agent
                    .projected_completion_provider_run_ids
                    .iter()
                    .any(|id| id == provider_run_id)
                {
                    agent
                        .projected_completion_provider_run_ids
                        .push(provider_run_id.to_string());
                }
            }
            self.app.leased_workflow_turns.remove(provider_run_id);
        }
        if output_chunks.is_empty() && notices.is_empty() && completions.is_empty() {
            return Ok(None);
        }
        Ok(Some((
            lease.home_kernel_id,
            RelayPeerEvent::LeasedRuntimeProjection {
                home_session_id: lease.home_session_id,
                home_agent_id: lease.home_agent_id,
                provider_run_id: provider_run_id.to_string(),
                output_chunks,
                notices,
                completions,
            },
        )))
    }

    pub(crate) fn pump_leased_runtime_projections(
        &mut self,
    ) -> Result<Vec<(String, RelayPeerEvent)>, DaemonError> {
        let leased_agents = self.app.leased_agents.values().cloned().collect::<Vec<_>>();
        let mut events = Vec::new();
        for leased_agent in leased_agents {
            let Some(provider_run_id) = self
                .app
                .providers
                .get_run_for_agent(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                )
                .or_else(|| {
                    self.app.providers.get_latest_run_for_agent(
                        &leased_agent.backing_session_id,
                        &leased_agent.backing_agent_id,
                    )
                })
                .map(|run| run.id().to_string())
            else {
                continue;
            };
            let _ = provider_output::ProviderOutputPump::new(self.app).pump_provider_output(
                provider_output::ProviderOutputPumpRequest {
                    session_id: &leased_agent.backing_session_id,
                    provider_run_id: &provider_run_id,
                    recipient_attachment_ids: vec![leased_agent.backing_attachment_id.clone()],
                    initial_liveness_already_checked: false,
                },
            )?;
            if let Some(event) =
                self.drain_leased_runtime_projection(&leased_agent.id, &provider_run_id, false)?
            {
                events.push(event);
            }
        }
        Ok(events)
    }

    pub(crate) fn project_remote_runtime_projection(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    ) -> Result<(), DaemonError> {
        let _ = self.app.sessions.get_session(session_id)?;
        let recipient_attachment_ids = self.app.attachments.list_session_attachment_ids(session_id);
        let saw_completion = !completions.is_empty();
        for chunk in output_chunks {
            self.app.terminal.fan_out_output(
                session_id,
                provider_run_id,
                Some(agent_id),
                chunk.kind.clone(),
                chunk.merge_key.clone(),
                recipient_attachment_ids.clone(),
                &chunk.bytes,
            );
            if chunk.kind != TerminalOutputKind::PromptEcho {
                self.app.append_history_entry(
                    session_id,
                    SessionHistoryEntry::provider_output(
                        session_id,
                        provider_run_id,
                        Some(agent_id),
                        chunk.kind,
                        chunk.merge_key,
                        String::from_utf8_lossy(&chunk.bytes).into_owned(),
                    ),
                );
            }
        }
        for notice in notices {
            self.app.terminal.record_notice(
                session_id,
                Some(provider_run_id),
                Some(agent_id),
                recipient_attachment_ids.clone(),
                notice.clone(),
            );
            self.app.append_history_entry(
                session_id,
                SessionHistoryEntry::notice(
                    session_id,
                    Some(provider_run_id),
                    Some(agent_id),
                    notice,
                ),
            );
        }
        for completion in completions {
            self.app.terminal.record_assistant_message_completion(
                session_id,
                provider_run_id,
                Some(agent_id),
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
        }
        if let Some(active_prompt) = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
        {
            let workflow_output_ready = active_prompt.workflow_run_id().is_some()
                && crate::app::workflow_runtime::workflow_prompt_has_completion_output_from_runtime(
                    self.app,
                    session_id,
                    &active_prompt,
                    Some(provider_run_id),
                );
            if !saw_completion && !workflow_output_ready {
                return Ok(());
            }
            if active_prompt.workflow_run_id().is_some() && !workflow_output_ready {
                if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                    active_prompt.workflow_run_id(),
                    active_prompt.workflow_node_run_id(),
                ) {
                    let message =
                        "provider completed workflow turn without a validated workflow output";
                    let provider_diagnostic = self
                        .app
                        .providers()
                        .get_run(provider_run_id)
                        .ok()
                        .and_then(|run| run.terminal_diagnostic().map(str::to_string))
                        .filter(|message| !message.trim().is_empty());
                    let (failure_kind, failure_message, notice_message) = if let Some(diagnostic) =
                        provider_diagnostic
                    {
                        (
                            crate::session::WorkflowFailureKind::ProviderFailure,
                            diagnostic.clone(),
                            format!(
                                "Workflow run `{workflow_run_id}` failed after provider turn failure: {diagnostic}"
                            ),
                        )
                    } else {
                        (
                            crate::session::WorkflowFailureKind::MissingStructuredOutput,
                            message.to_string(),
                            format!(
                                "Workflow run `{workflow_run_id}` failed after provider turn completion without workflow output."
                            ),
                        )
                    };
                    let failure = crate::session::WorkflowFailureEvent::new(
                        failure_kind,
                        workflow_node_run_id,
                        Vec::new(),
                        failure_message,
                    );
                    let _ = self.app.sessions_mut().record_workflow_failure_event(
                        session_id,
                        workflow_run_id,
                        failure,
                    );
                    self.app.sessions_mut().fail_workflow_node_run(
                        session_id,
                        workflow_run_id,
                        workflow_node_run_id,
                    )?;
                    self.app.record_notice(
                        session_id,
                        Some(provider_run_id),
                        recipient_attachment_ids.clone(),
                        notice_message,
                    );
                    let _ = crate::app::KernelSessionReadService::new(self.app)
                        .session_snapshot(session_id);
                    let _ = self
                        .app
                        .prompt_owner_complete_active_prompt_only(session_id, agent_id)?;
                }
                return Ok(());
            }
            let remote_execution = self
                .app
                .agents
                .get_agent(agent_id)?
                .remote_execution()
                .cloned();
            let completed = self
                .app
                .prompt_owner_complete_active_prompt_only(session_id, agent_id)?;
            let _ =
                crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id);
            crate::app::workflow_runtime::complete_workflow_prompt_from_runtime(
                self.app,
                session_id,
                &completed,
                Some(provider_run_id),
            )?;
            if let Some(remote_execution) = remote_execution {
                if self
                    .app
                    .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
                    .is_none()
                {
                    let started_next = self.app.advance_next_queued_prompt_remote(
                        session_id,
                        agent_id,
                        &remote_execution.worker_kernel_id,
                        &remote_execution.leased_agent_id,
                        remote_execution.relay_url.as_deref(),
                        remote_execution.relay_token.as_deref(),
                    )?;
                    if started_next.is_none() {
                        self.app.sync_focused_provider_run_if_idle(session_id)?;
                    }
                }
            }
        }
        Ok(())
    }
}
