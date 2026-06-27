use crate::app::provider_output;
use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::history::SessionHistoryEntryKind;
use crate::session::{PromptCompletion, PromptQueueItem, PromptStatus, PromptSubmissionOutcome};
use crate::terminal::TerminalOutputKind;
use crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout;
use crate::transport::relay_peer::{
    RelayPeerEvent, RelayPeerRequest, RelayPeerResponse, RelayProjectedCompletion,
    RelayProjectedOutputChunk, RelayProjectedPrompt,
};
use arroba_relay::protocol::ClientTarget;

use super::RemoteLeaseRuntime;

const REMOTE_COMPLETION_HARVEST_RESPONSE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(60);

#[derive(Debug, Default)]
pub(crate) struct RemoteRuntimeProjectionOutcome {
    pub(crate) completions: Vec<PromptCompletion>,
}

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
        let had_output_history_before_pump =
            self.leased_provider_run_has_output_history(&leased_agent, provider_run_id)?;
        let mut pumped_output_records = Vec::new();
        let mut settled_quiet = false;
        if pump_output {
            settled_quiet =
                self.settle_quiet_leased_prompt_if_needed(&leased_agent, provider_run_id)?;
            pumped_output_records = provider_output::pump_terminal_output_for_attachment(
                self.app,
                &leased_agent.backing_session_id,
                &leased_agent.backing_attachment_id,
            )?;
            if !settled_quiet {
                settled_quiet =
                    self.settle_quiet_leased_prompt_if_needed(&leased_agent, provider_run_id)?;
            }
        }
        let mut output_chunks = pumped_output_records
            .into_iter()
            .chain(
                self.app
                    .terminal
                    .drain_output_records(
                        &leased_agent.backing_session_id,
                        &leased_agent.backing_attachment_id,
                    )
                    .into_iter(),
            )
            .filter(|record| record.provider_run_id == provider_run_id)
            .map(|record| RelayProjectedOutputChunk {
                kind: record.kind,
                merge_key: record.merge_key,
                bytes: record.bytes,
            })
            .collect::<Vec<_>>();
        let mut projected_output_history_keys = Vec::new();
        let mut history_chunks =
            self.leased_provider_run_output_history_chunks(&leased_agent, provider_run_id)?;
        let latest_output_history_completion_key = history_chunks
            .iter()
            .rev()
            .find(|chunk| chunk.kind == TerminalOutputKind::ProviderOutput)
            .map(|chunk| {
                leased_provider_run_history_chunk_key(&leased_agent, provider_run_id, chunk)
            });
        history_chunks.retain(|history_chunk| {
            let history_key = leased_provider_run_history_chunk_key(
                &leased_agent,
                provider_run_id,
                history_chunk,
            );
            !leased_agent
                .projected_output_history_keys
                .iter()
                .any(|key| key == &history_key)
                && !output_chunks.iter().any(|chunk| {
                    chunk.kind == history_chunk.kind
                        && chunk.merge_key == history_chunk.merge_key
                        && chunk.bytes == history_chunk.bytes
                })
        });
        for history_chunk in &history_chunks {
            projected_output_history_keys.push(leased_provider_run_history_chunk_key(
                &leased_agent,
                provider_run_id,
                history_chunk,
            ));
        }
        if !history_chunks.is_empty() {
            output_chunks.extend(history_chunks);
        }
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
        completions.retain(|completion| {
            let completion_key = leased_provider_run_completion_key(
                &leased_agent,
                provider_run_id,
                &completion.message_id,
            );
            !leased_agent
                .projected_completion_keys
                .iter()
                .any(|key| key == &completion_key)
        });
        let mut prompts = Vec::new();
        let mut latest_home_origin_prompt_key = None;
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
                let prompt_history_key = format!(
                    "history:{}:{}:{}",
                    entry
                        .source_attachment_id
                        .as_deref()
                        .unwrap_or(&leased_agent.backing_attachment_id),
                    entry.timestamp_ms,
                    stable_prompt_hash(&entry.text)
                );
                if entry
                    .source_attachment_id
                    .as_deref()
                    .is_none_or(|source_attachment_id| {
                        source_attachment_id == leased_agent.backing_attachment_id
                    })
                {
                    latest_home_origin_prompt_key = Some(prompt_history_key);
                    continue;
                }
                if !leased_agent
                    .projected_prompt_ids
                    .iter()
                    .any(|id| id == &prompt_history_key)
                {
                    prompts.push(RelayProjectedPrompt {
                        prompt_id: prompt_history_key,
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
            if prompt.source_attachment_id() != leased_agent.backing_attachment_id
                && !leased_agent
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
        let mut backing_prompt_active = backing_active_prompt.is_some();
        let backing_active_prompt_id = backing_active_prompt
            .as_ref()
            .map(|prompt| prompt.id().to_string());
        let current_batch_has_provider_output = output_chunks
            .iter()
            .any(|chunk| chunk.kind == TerminalOutputKind::ProviderOutput);
        let has_settleable_output_history = had_output_history_before_pump
            || current_batch_has_provider_output
            || (output_chunks.is_empty()
                && self.leased_provider_run_has_output_history(&leased_agent, provider_run_id)?);
        let should_complete_from_history = completions.is_empty()
            && prompts.is_empty()
            && backing_active_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.workflow_run_id().is_none())
            && has_settleable_output_history;
        if should_complete_from_history {
            let _ = self.app.complete_active_prompt(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
                Some(provider_run_id),
            )?;
            let _generated_prompt_completions = self
                .app
                .terminal
                .drain_completion_records(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_attachment_id,
                )
                .into_iter()
                .filter(|record| record.provider_run_id == provider_run_id)
                .collect::<Vec<_>>();
            backing_prompt_active = false;
            settled_quiet = true;
        }
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
        if completions.is_empty() && (settled_quiet || !backing_prompt_active) {
            let message_id = leased_synthetic_completion_message_id(
                &leased_agent,
                provider_run_id,
                backing_active_prompt_id.as_deref(),
                latest_home_origin_prompt_key.as_deref(),
                latest_output_history_completion_key.as_deref(),
                &output_chunks,
            );
            let completion_key =
                leased_provider_run_completion_key(&leased_agent, provider_run_id, &message_id);
            if !leased_agent
                .projected_completion_keys
                .iter()
                .any(|key| key == &completion_key)
            {
                completions.push(RelayProjectedCompletion {
                    message_id,
                    completed_at_ms: crate::session::unix_epoch_ms(),
                });
            }
        }
        if !completions.is_empty() {
            if backing_prompt_active {
                let _ = self.app.complete_active_prompt(
                    &leased_agent.backing_session_id,
                    &leased_agent.backing_agent_id,
                    Some(provider_run_id),
                )?;
            }
            let completion_keys = completions
                .iter()
                .map(|completion| {
                    leased_provider_run_completion_key(
                        &leased_agent,
                        provider_run_id,
                        &completion.message_id,
                    )
                })
                .collect::<Vec<_>>();
            if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                for completion_key in completion_keys {
                    if !agent
                        .projected_completion_keys
                        .iter()
                        .any(|key| key == &completion_key)
                    {
                        agent.projected_completion_keys.push(completion_key);
                    }
                }
            }
            self.app.leased_workflow_turns.remove(provider_run_id);
        }
        if !projected_output_history_keys.is_empty() {
            if let Some(agent) = self.app.leased_agents.get_mut(leased_agent_id) {
                for key in projected_output_history_keys {
                    if !agent
                        .projected_output_history_keys
                        .iter()
                        .any(|id| id == &key)
                    {
                        agent.projected_output_history_keys.push(key);
                    }
                }
            }
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
                provider_run: self.app.providers.get_run(provider_run_id).ok(),
                prompts,
                output_chunks,
                notices,
                completions,
            },
        )))
    }

    fn settle_quiet_leased_prompt_if_needed(
        &mut self,
        leased_agent: &LeasedAgent,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(active_prompt) = self.app.prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )?
        else {
            return Ok(false);
        };
        if active_prompt.workflow_run_id().is_some() {
            return Ok(false);
        }
        if !crate::transport::flow_control::prompt_output_quiet_after_response(
            self.app,
            provider_run_id,
            std::time::Duration::from_millis(50),
        ) {
            return Ok(false);
        }
        let _ = self.app.complete_active_prompt(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
            Some(provider_run_id),
        )?;
        Ok(true)
    }

    fn leased_provider_run_has_output_history(
        &mut self,
        leased_agent: &LeasedAgent,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let entries = self
            .app
            .load_session_history_entries(&session, Some(&leased_agent.backing_agent_id))?;
        Ok(entries.into_iter().any(|entry| {
            entry.provider_run_id.as_deref() == Some(provider_run_id)
                && entry.kind == SessionHistoryEntryKind::ProviderOutput
        }))
    }

    fn leased_provider_run_output_history_chunks(
        &mut self,
        leased_agent: &LeasedAgent,
        provider_run_id: &str,
    ) -> Result<Vec<RelayProjectedOutputChunk>, DaemonError> {
        let session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let entries = self
            .app
            .load_session_history_entries(&session, Some(&leased_agent.backing_agent_id))?;
        Ok(entries
            .into_iter()
            .filter(|entry| {
                entry.provider_run_id.as_deref() == Some(provider_run_id)
                    && entry.kind == SessionHistoryEntryKind::ProviderOutput
            })
            .map(|entry| RelayProjectedOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: entry.merge_key,
                bytes: entry.text.into_bytes(),
            })
            .collect())
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
        provider_run: Option<crate::provider::RuntimeProviderRun>,
        prompts: Vec<RelayProjectedPrompt>,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    ) -> Result<RemoteRuntimeProjectionOutcome, DaemonError> {
        let _ = self.app.sessions.get_session(session_id)?;
        let mut outcome = RemoteRuntimeProjectionOutcome::default();
        if let Some(provider_run) = provider_run {
            let leased_agent_id = self
                .app
                .agents
                .get_agent(agent_id)
                .ok()
                .and_then(|agent| {
                    agent
                        .remote_execution()
                        .map(|remote| remote.leased_agent_id.clone())
                })
                .unwrap_or_else(|| agent_id.to_string());
            let projected_provider_run_id = crate::provider::projected_leased_provider_run_id(
                &leased_agent_id,
                provider_run_id,
            );
            let projected_run = provider_run.projected_for_home_agent_with_id(
                projected_provider_run_id,
                session_id.to_string(),
                agent_id.to_string(),
            );
            self.app
                .update_provider_run_projection(projected_run.clone());
            let _ = self
                .app
                .sessions
                .set_active_provider_run(session_id, Some(projected_run.id().to_string()));
            if let Ok(agent) = self.app.agents.get_agent(agent_id) {
                if agent.remote_execution().is_some() {
                    let _ = self
                        .app
                        .agents
                        .set_remote_execution_active_worker_provider_run_id(
                            agent_id,
                            Some(provider_run_id.to_string()),
                        );
                    let _ = self.app.agents.set_agent_runtime_profile(
                        agent_id,
                        projected_run.provider(),
                        Some(projected_run.model().to_string()),
                        projected_run.variant().map(str::to_string),
                        projected_run.resume_state().clone(),
                    );
                }
            }
        }
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
            self.app.fan_out_output_for_agent(
                session_id,
                provider_run_id,
                Some(agent_id),
                chunk.kind.clone(),
                chunk.merge_key.clone(),
                recipient_attachment_ids.clone(),
                &chunk.bytes,
            );
        }
        for notice in notices {
            self.app.record_notice_for_agent(
                session_id,
                Some(provider_run_id),
                Some(agent_id),
                recipient_attachment_ids.clone(),
                notice.clone(),
            );
        }
        for completion in completions {
            self.app.record_assistant_message_completion_for_agent(
                session_id,
                provider_run_id,
                Some(agent_id),
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
        }
        let remote_execution = self
            .app
            .agents
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.remote_execution().cloned());
        if saw_completion {
            if let Some(remote_execution) = remote_execution.as_ref() {
                self.harvest_remote_completion_observations(remote_execution, provider_run_id);
            }
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
                return Ok(outcome);
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
                    let completed = self
                        .app
                        .prompt_owner_complete_active_prompt_only(session_id, agent_id)?;
                    outcome.completions.push(PromptCompletion {
                        completed,
                        started_next: None,
                    });
                }
                return Ok(outcome);
            }
            let completed = self
                .app
                .prompt_owner_complete_active_prompt_only(session_id, agent_id)?;
            if let Ok(agent) = self.app.agents.get_agent(agent_id) {
                if agent.remote_execution().is_some() {
                    let _ = self
                        .app
                        .agents
                        .set_remote_execution_active_worker_provider_run_id(agent_id, None);
                }
            }
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
                    outcome.completions.push(PromptCompletion {
                        completed,
                        started_next,
                    });
                } else {
                    outcome.completions.push(PromptCompletion {
                        completed,
                        started_next: None,
                    });
                }
            } else {
                outcome.completions.push(PromptCompletion {
                    completed,
                    started_next: None,
                });
            }
        }
        Ok(outcome)
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
        let outcome =
            self.app
                .prompt_owner_submit_prepared_prompt(session_id, prompt.clone(), false)?;
        self.app.spawn_user_prompt_history_append(
            session_id,
            &attachment_id,
            agent_id,
            prompt.prompt(),
            prompt.attachments(),
        )?;
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
        let response = self.app.block_on_relay_future(
            send_peer_request_via_temporary_connection_with_timeout(
                &relay_config,
                ClientTarget {
                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::ObserveLeasedGitAfter {
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                },
                REMOTE_COMPLETION_HARVEST_RESPONSE_TIMEOUT,
            ),
        );
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

fn leased_provider_run_completion_key(
    leased_agent: &LeasedAgent,
    provider_run_id: &str,
    message_id: &str,
) -> String {
    format!(
        "{}:{provider_run_id}:{message_id}",
        leased_agent.backing_session_id
    )
}

fn leased_synthetic_completion_message_id(
    leased_agent: &LeasedAgent,
    provider_run_id: &str,
    backing_active_prompt_id: Option<&str>,
    latest_home_origin_prompt_key: Option<&str>,
    latest_output_history_completion_key: Option<&str>,
    output_chunks: &[RelayProjectedOutputChunk],
) -> String {
    if let Some(prompt_key) = latest_home_origin_prompt_key {
        return format!("leased-{provider_run_id}-completion:{prompt_key}");
    }
    if let Some(output_key) = latest_output_history_completion_key {
        return format!("leased-{provider_run_id}-completion:{output_key}");
    }
    if let Some(prompt_id) = backing_active_prompt_id {
        return format!("leased-{provider_run_id}-completion:{prompt_id}");
    }
    if let Some(chunk) = output_chunks
        .iter()
        .rev()
        .find(|chunk| chunk.kind == TerminalOutputKind::ProviderOutput)
    {
        return format!(
            "leased-{provider_run_id}-completion:{}",
            leased_provider_run_history_chunk_key(leased_agent, provider_run_id, chunk)
        );
    }
    format!("leased-{provider_run_id}-completion:quiet")
}

fn leased_provider_run_history_chunk_key(
    leased_agent: &LeasedAgent,
    provider_run_id: &str,
    chunk: &RelayProjectedOutputChunk,
) -> String {
    format!(
        "{}:{provider_run_id}:{}:{}:{}",
        leased_agent.backing_session_id,
        format!("{:?}", chunk.kind),
        chunk.merge_key.as_deref().unwrap_or(""),
        stable_bytes_hash(&chunk.bytes)
    )
}

fn stable_bytes_hash(bytes: &[u8]) -> u64 {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

fn stable_prompt_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}
