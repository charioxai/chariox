use crate::app::provider_output;
use crate::error::DaemonError;
use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};
use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome};
use crate::terminal::TerminalOutputKind;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayPeerRequest, RelayPeerResponse, RelayProjectedCompletion,
    RelayProjectedOutputChunk, RelayProjectedPrompt,
};
use arroba_relay::protocol::ClientTarget;

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
        let mut prompts = Vec::new();
        if let Ok(backing_session) = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)
        {
            let history_entries = self.app.load_session_history_entries(
                &backing_session,
                Some(&leased_agent.backing_agent_id),
            )?;
            for entry in history_entries
                .into_iter()
                .filter(|entry| entry.kind == SessionHistoryEntryKind::UserPrompt)
            {
                let prompt_id = format!(
                    "history:{}:{}:{}",
                    entry
                        .source_attachment_id
                        .as_deref()
                        .unwrap_or(&leased_agent.backing_attachment_id),
                    entry.timestamp_ms,
                    stable_prompt_hash(&entry.text)
                );
                if !leased_agent
                    .projected_prompt_ids
                    .iter()
                    .any(|id| id == &prompt_id)
                {
                    prompts.push(RelayProjectedPrompt {
                        prompt_id,
                        text: entry.text,
                    });
                }
            }
        }
        let backing_active_prompt = self.app.prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )?;
        if let Some(prompt) = backing_active_prompt.as_ref() {
            if !leased_agent
                .projected_prompt_ids
                .iter()
                .any(|id| id == prompt.id())
            {
                prompts.push(RelayProjectedPrompt {
                    prompt_id: prompt.id().to_string(),
                    text: prompt.prompt().to_string(),
                });
            }
        }
        let backing_prompt_active = backing_active_prompt.is_some();
        let completion_already_projected = leased_agent
            .projected_completion_provider_run_ids
            .iter()
            .any(|id| id == provider_run_id);
        if !prompts.is_empty() {
            if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                for prompt in &prompts {
                    if !agent
                        .projected_prompt_ids
                        .iter()
                        .any(|id| id == &prompt.prompt_id)
                    {
                        agent.projected_prompt_ids.push(prompt.prompt_id.clone());
                    }
                }
            }
        }
        if completions.is_empty()
            && !backing_prompt_active
            && (!completion_already_projected || !prompts.is_empty())
        {
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
        if output_chunks.is_empty()
            && notices.is_empty()
            && completions.is_empty()
            && prompts.is_empty()
        {
            return Ok(None);
        }
        Ok(Some((
            lease.home_kernel_id,
            RelayPeerEvent::LeasedRuntimeProjection {
                home_session_id: lease.home_session_id,
                home_agent_id: lease.home_agent_id,
                provider_run_id: provider_run_id.to_string(),
                prompts,
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
        prompts: Vec<RelayProjectedPrompt>,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    ) -> Result<(), DaemonError> {
        let _ = self.app.sessions.get_session(session_id)?;
        let recipient_attachment_ids = self.app.attachments.list_session_attachment_ids(session_id);
        let saw_completion = !completions.is_empty();
        for prompt in prompts {
            self.project_remote_native_prompt_started(
                session_id,
                agent_id,
                provider_run_id,
                prompt,
            )?;
        }
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
            if let Some(remote_execution) = remote_execution.as_ref() {
                self.harvest_remote_completion_observations(remote_execution, provider_run_id);
            }
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

    fn project_remote_native_prompt_started(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        projected: RelayProjectedPrompt,
    ) -> Result<(), DaemonError> {
        if self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .is_some()
        {
            return Ok(());
        }
        let Some(attachment_id) = self
            .app
            .attachments
            .list_session_attachment_ids(session_id)
            .into_iter()
            .next()
        else {
            return Ok(());
        };
        let prompt = PromptQueueItem::new(
            self.app.sessions_mut().reserve_prompt_id(),
            &attachment_id,
            agent_id,
            &projected.text,
            PromptStatus::Queued,
        );
        self.app.spawn_user_prompt_history_append(
            session_id,
            &attachment_id,
            agent_id,
            prompt.prompt(),
            prompt.attachments(),
        )?;
        let outcome = self
            .app
            .prompt_owner_submit_prepared_prompt(session_id, prompt, false)?;
        if matches!(outcome, PromptSubmissionOutcome::Started { .. }) {
            crate::transport::flow_control::note_prompt_started(self.app, provider_run_id);
        }
        Ok(())
    }

    fn harvest_remote_completion_observations(
        &mut self,
        remote_execution: &crate::agent::RemoteAgentBinding,
        provider_run_id: &str,
    ) {
        let relay_config = self.app.relay_config_for_remote_execution(remote_execution);
        let response = self
            .app
            .block_on_relay_future(send_peer_request_via_temporary_connection(
                &relay_config,
                ClientTarget {
                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::ObserveLeasedGitAfter {
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                },
            ));
        match response {
            Ok(RelayPeerResponse::LeasedGitObserved {
                git_observations,
                workspace_live_sync_change,
                ..
            }) => {
                if let Err(error) = crate::git_observer::append_observations(
                    &self.app.operational_history_store(),
                    git_observations,
                ) {
                    crate::logging::warn_with_fields(
                        "daemon.git_observer",
                        "failed to append projected remote git observations",
                        serde_json::json!({
                            "worker_kernel_id": remote_execution.worker_kernel_id,
                            "leased_agent_id": remote_execution.leased_agent_id,
                            "error": error.to_string(),
                        }),
                    );
                }
                if let Some(change) = workspace_live_sync_change {
                    self.app.fanout_remote_workspace_live_sync_change(
                        change,
                        Some(&remote_execution.worker_kernel_id),
                    );
                }
            }
            Ok(other) => crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "unexpected projected remote completion harvest response",
                serde_json::json!({
                    "worker_kernel_id": remote_execution.worker_kernel_id,
                    "leased_agent_id": remote_execution.leased_agent_id,
                    "response": format!("{other:?}"),
                }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "failed to harvest projected remote completion observations",
                serde_json::json!({
                    "worker_kernel_id": remote_execution.worker_kernel_id,
                    "leased_agent_id": remote_execution.leased_agent_id,
                    "error": error.to_string(),
                }),
            ),
        }
    }
}

fn stable_prompt_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}
