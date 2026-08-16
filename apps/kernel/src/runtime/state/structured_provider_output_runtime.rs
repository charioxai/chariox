//! Structured provider-output polling and batch application.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn pump_owned_structured_provider_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        let mut provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Parked {
            if !owned.provider_run_has_active_prompt(session_id, &provider_run)? {
                return Ok(owned
                    .structured_output_records
                    .take_and_stop_polling(provider_run_id));
            }
            provider_run = owned.provider_store.resume_run_detached(provider_run_id)?;
            owned.provider_run_projection.update(provider_run.clone());
        }
        if provider_run.endpoint_mode() != crate::provider::AgentEndpointMode::External {
            if let Err(error) = self
                .with_app_side_effect(|app| {
                    app.drain_provider_pty_output_for_runtime(provider_run_id)
                })
                .await
            {
                if self
                    .reconcile_provider_run_exit(session_id, provider_run_id)
                    .await?
                {
                    return Ok(owned
                        .structured_output_records
                        .take_and_stop_polling(provider_run_id));
                }
                if !matches!(error, DaemonError::PtyProcessNotFound { .. }) {
                    return Err(error);
                }
            }
        }
        let mut records = owned.structured_output_records.take(provider_run_id);
        let now_ms = crate::session::unix_epoch_ms();
        for finished in owned
            .provider_store
            .drain_finished_structured_output_poll_jobs()
        {
            let finished_run_id = finished.provider_run_id.clone();
            let polled_prompt_id = owned
                .structured_output_records
                .take_in_flight_prompt_id(&finished_run_id);
            let is_requested_run = finished_run_id == provider_run_id;
            crate::logging::debug_with_fields(
                "daemon.provider",
                "drained finished structured output poll",
                serde_json::json!({
                    "requested_provider_run_id": provider_run_id,
                    "finished_provider_run_id": finished_run_id,
                    "is_requested_run": is_requested_run,
                }),
            );
            let poll_result = match finished.result {
                Ok(Some(poll_result)) => poll_result,
                Ok(None) => {
                    owned
                        .structured_output_records
                        .schedule_after_empty_poll(finished_run_id, now_ms);
                    continue;
                }
                Err(error) => {
                    let reconcile_result = if is_requested_run {
                        self.reconcile_provider_run_exit(session_id, provider_run_id)
                            .await
                    } else {
                        match owned.provider_store.get_run(&finished_run_id) {
                            Ok(run) => {
                                self.reconcile_provider_run_exit(run.session_id(), &finished_run_id)
                                    .await
                            }
                            Err(run_error) => Err(run_error),
                        }
                    };
                    match reconcile_result {
                        Ok(true) => {
                            owned
                                .structured_output_records
                                .stop_polling(&finished_run_id);
                            continue;
                        }
                        Ok(false) if is_requested_run => return Err(error),
                        Ok(false) => {
                            owned
                                .structured_output_records
                                .schedule_after_empty_poll(finished_run_id.clone(), now_ms);
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll failed",
                                serde_json::json!({
                                    "provider_run_id": finished_run_id,
                                    "error": error.to_string(),
                                }),
                            );
                            continue;
                        }
                        Err(reconcile_error) if is_requested_run => return Err(reconcile_error),
                        Err(reconcile_error) => {
                            owned
                                .structured_output_records
                                .schedule_after_empty_poll(finished_run_id.clone(), now_ms);
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll reconciliation failed",
                                serde_json::json!({
                                    "provider_run_id": finished_run_id,
                                    "error": reconcile_error.to_string(),
                                }),
                            );
                            continue;
                        }
                    }
                }
            };
            let active_prompt = owned
                .provider_store
                .get_run(&finished_run_id)
                .ok()
                .and_then(|run| {
                    run.agent_instance_id()
                        .map(str::to_string)
                        .map(|agent_id| (run, agent_id))
                })
                .and_then(|(run, agent_id)| {
                    owned
                        .session_store
                        .get_session(run.session_id())
                        .ok()
                        .and_then(|session| {
                            owned
                                .prompt_state_owner
                                .active_prompt_for_agent(&session, &agent_id)
                        })
                });
            let active_prompt_id = active_prompt.as_ref().map(|prompt| prompt.id().to_string());
            let active_prompt_is_dispatching = active_prompt.as_ref().is_some_and(|prompt| {
                prompt.status() == crate::session::PromptStatus::Dispatching
                    || prompt.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            });
            if polled_prompt_id != active_prompt_id || active_prompt_is_dispatching {
                crate::logging::debug_with_fields(
                    "daemon.provider",
                    "discarding stale structured output poll before prompt delivery",
                    serde_json::json!({
                        "provider_run_id": finished_run_id,
                        "polled_prompt_id": polled_prompt_id,
                        "active_prompt_id": active_prompt_id,
                        "active_prompt_is_dispatching": active_prompt_is_dispatching,
                    }),
                );
                owned
                    .structured_output_records
                    .schedule_next_poll(finished_run_id, now_ms);
                continue;
            }
            let run = match owned.provider_store.get_run(&finished_run_id) {
                Ok(run) => run,
                Err(_) => {
                    owned.structured_output_records.clear(&finished_run_id);
                    continue;
                }
            };
            let next_due_at_ms =
                if crate::app::provider_output::structured_output_batch_should_poll_immediately(
                    &poll_result,
                ) {
                    now_ms
                } else {
                    now_ms.saturating_add(
                        crate::app::provider_output::STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS,
                    )
                };
            let run_session_id = run.session_id().to_string();
            let recipients = if is_requested_run {
                recipient_attachment_ids.clone()
            } else {
                owned
                    .attachment_store
                    .list_session_attachment_ids(&run_session_id)
            };
            let applied = self
                .apply_owned_structured_output_batch(
                    &run_session_id,
                    &finished_run_id,
                    recipients,
                    poll_result,
                )
                .await?;
            if is_requested_run {
                records.extend(applied);
            } else {
                owned
                    .structured_output_records
                    .append(finished_run_id.clone(), applied);
            }
            owned
                .structured_output_records
                .schedule_next_poll(finished_run_id, next_due_at_ms);
        }
        if owned
            .structured_output_records
            .poll_due(provider_run_id, crate::session::unix_epoch_ms())
        {
            let active_prompt = owned
                .provider_store
                .get_run(provider_run_id)
                .ok()
                .and_then(|run| {
                    run.agent_instance_id()
                        .map(str::to_string)
                        .map(|agent_id| (run, agent_id))
                })
                .and_then(|(run, agent_id)| {
                    owned
                        .session_store
                        .get_session(run.session_id())
                        .ok()
                        .and_then(|session| {
                            owned
                                .prompt_state_owner
                                .active_prompt_for_agent(&session, &agent_id)
                        })
                });
            if active_prompt.as_ref().is_some_and(|prompt| {
                prompt.status() == crate::session::PromptStatus::Dispatching
                    || prompt.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            }) {
                owned.structured_output_records.schedule_after_empty_poll(
                    provider_run_id.to_string(),
                    crate::session::unix_epoch_ms(),
                );
                return Ok(records);
            }
            match owned
                .provider_store
                .enqueue_structured_output_poll(provider_run_id)?
            {
                true => {
                    let prompt_id = active_prompt.map(|prompt| prompt.id().to_string());
                    owned
                        .structured_output_records
                        .mark_poll_enqueued(provider_run_id, prompt_id);
                }
                false => owned.structured_output_records.schedule_after_empty_poll(
                    provider_run_id.to_string(),
                    crate::session::unix_epoch_ms(),
                ),
            }
        }
        if owned.prompt_completion_settlement_pending(provider_run_id) {
            let _ = self
                .settle_owned_provider_prompt(session_id, provider_run_id, false, false, false)
                .await?;
        }
        Ok(records)
    }

    pub(super) async fn apply_owned_structured_output_batch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        mut poll_result: crate::provider::ProviderPromptSignalBatch,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let session = owned.session_store.get_session(session_id)?;
        reject_workflow_publication_opencode_model_substitution(
            session.is_hidden() && !session.workflow_publications().is_empty(),
            &provider_run,
            &mut poll_result,
        );
        if !poll_result.chunks.is_empty()
            || !poll_result.completions.is_empty()
            || !poll_result.notices.is_empty()
            || poll_result.prompt_completed
            || poll_result.terminal_failure.is_some()
            || poll_result.resolved_model.is_some()
            || poll_result.resolved_variant.is_some()
            || poll_result.resolved_usage_tokens_total.is_some()
            || poll_result.resolved_usage.is_some()
            || poll_result.resolved_resume_state.is_some()
        {
            crate::logging::debug_with_fields(
                "daemon.provider",
                "applying structured output batch",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "chunks": poll_result.chunks.len(),
                    "completions": poll_result.completions.len(),
                    "prompt_completed": poll_result.prompt_completed,
                    "terminal_failure": poll_result.terminal_failure,
                }),
            );
        }
        owned
            .provider_store
            .apply_structured_output_metadata(provider_run_id, &poll_result)?;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run.agent_instance_id().map(str::to_string);
        if let Some(resume_state) = poll_result.resolved_resume_state.as_ref() {
            if let Some(agent_id) = provider_run.agent_instance_id() {
                let agent = owned.agent_store.set_agent_runtime_profile(
                    agent_id,
                    provider_run.provider(),
                    Some(provider_run.model().to_string()),
                    provider_run.variant().map(str::to_string),
                    resume_state.clone(),
                )?;
                let _ = owned.session_snapshot(provider_run.session_id())?;
                self.append_agent_durable_event("agent.runtime_profile_updated", &agent, None)
                    .await?;
            }
        }
        if let Some(agent_id) = provider_run.agent_instance_id() {
            owned.external_provider_sessions.mark_provider_run_attached(
                provider_run.adapter_key(),
                provider_run.provider_session_id(),
                provider_run.resume_state(),
                provider_run.session_id(),
                agent_id,
            );
        }
        owned.provider_run_projection.update(provider_run);
        let terminal_failure = poll_result
            .terminal_failure
            .as_deref()
            .map(provider_prompt_dispatch_failure_notice);
        project_terminal_failure_chunk(&mut poll_result, terminal_failure.as_deref());
        let mut recorded_notice_messages = std::collections::HashSet::new();
        if let Some(message) = terminal_failure.as_ref() {
            recorded_notice_messages.insert(message.clone());
        }
        for notice in &poll_result.notices {
            let message = provider_notice_message(notice);
            if !recorded_notice_messages.insert(message.clone()) {
                continue;
            }
            owned.record_notice(
                session_id,
                Some(provider_run_id),
                recipient_attachment_ids.clone(),
                message,
            );
        }
        let saw_response_content = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                crate::terminal::TerminalOutputKind::ProviderOutput
                    | crate::terminal::TerminalOutputKind::ProviderReasoning
            )
        });
        let saw_runtime_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                crate::terminal::TerminalOutputKind::ProviderOutput
                    | crate::terminal::TerminalOutputKind::ProviderReasoning
                    | crate::terminal::TerminalOutputKind::ProviderTool
                    | crate::terminal::TerminalOutputKind::ProviderStatus
            )
        });
        let saw_settlement_blocking_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                crate::terminal::TerminalOutputKind::ProviderOutput
                    | crate::terminal::TerminalOutputKind::ProviderReasoning
                    | crate::terminal::TerminalOutputKind::ProviderTool
            )
        });
        if saw_response_content {
            owned.note_prompt_response_content(provider_run_id);
        } else if saw_runtime_activity {
            owned.note_prompt_output(provider_run_id);
        }
        let completions = poll_result.completions;
        let prompt_completed = poll_result.prompt_completed;
        if let Some(message) = terminal_failure.as_deref() {
            let run = owned
                .provider_store
                .record_terminal_diagnostic(provider_run_id, message.to_string())?;
            owned.provider_run_projection.update(run);
        }
        let mut history_entries = Vec::with_capacity(poll_result.chunks.len());
        let prompt_metadata =
            owned.active_prompt_transcript_metadata_for_agent(session_id, agent_id.as_deref());
        let terminal_outputs = poll_result
            .chunks
            .into_iter()
            .map(|chunk| {
                let history_text = String::from_utf8_lossy(&chunk.bytes).into_owned();
                if chunk.kind != crate::terminal::TerminalOutputKind::PromptEcho {
                    history_entries.push(
                        crate::history::SessionHistoryEntry::provider_output(
                            session_id,
                            provider_run_id,
                            agent_id.as_deref(),
                            chunk.kind.clone(),
                            chunk.merge_key.clone(),
                            history_text.clone(),
                        )
                        .with_prompt_origin(prompt_metadata.prompt_origin)
                        .with_source_attachment_id(prompt_metadata.source_attachment_id.clone()),
                    );
                }
                super::prompt_transcript_owned_state::TerminalOutputBatchAppend {
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: agent_id.clone(),
                    kind: chunk.kind,
                    merge_key: chunk.merge_key,
                    bytes: chunk.bytes,
                }
            })
            .collect::<Vec<_>>();
        let records = owned.fan_out_terminal_outputs_to_recipients(
            session_id,
            recipient_attachment_ids.clone(),
            terminal_outputs,
        );
        owned.append_history_entries(session_id, history_entries);
        for completion in &completions {
            owned.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
            owned.mark_prompt_completion_recorded(provider_run_id);
        }
        if let Some(message) = terminal_failure {
            self.fail_owned_provider_prompt(session_id, provider_run_id, &message, false)
                .await?;
            return Ok(records);
        }
        if !self
            .reconcile_provider_run_exit(session_id, provider_run_id)
            .await?
        {
            let _ = self
                .settle_owned_provider_prompt(
                    session_id,
                    provider_run_id,
                    prompt_completed,
                    saw_settlement_blocking_activity,
                    false,
                )
                .await?;
        }
        if prompt_completed {
            self.observe_git_after_provider_activity_if_pending(provider_run_id)
                .await;
        }
        Ok(records)
    }
}

pub(super) fn reject_workflow_publication_opencode_model_substitution(
    is_publication_runtime: bool,
    provider_run: &crate::provider::RuntimeProviderRun,
    poll_result: &mut crate::provider::ProviderPromptSignalBatch,
) -> Option<String> {
    if !is_publication_runtime
        || provider_run.adapter_key() != "opencode"
        || provider_run.model() == "default"
    {
        return None;
    }
    let resolved_model = poll_result.resolved_model.as_deref()?;
    if resolved_model == provider_run.model() {
        return None;
    }

    let failure = format!(
        "deployed workflow provider model substitution is disabled: requested `{}`, OpenCode resolved `{resolved_model}`",
        provider_run.model(),
    );
    crate::logging::warn_with_fields(
        "daemon.provider.opencode",
        "rejected deployed workflow provider model substitution",
        serde_json::json!({
            "provider_run_id": provider_run.id(),
            "requested_model": provider_run.model(),
            "resolved_model": resolved_model,
            "resolved_model_source": poll_result.resolved_model_source,
        }),
    );
    poll_result.resolved_model = None;
    poll_result.resolved_model_source = None;
    poll_result.prompt_completed = true;
    if poll_result.terminal_failure.is_none() {
        poll_result.terminal_failure = Some(failure.clone());
    }
    Some(failure)
}

fn provider_prompt_dispatch_failure_notice(message: &str) -> String {
    format!(
        "Provider prompt dispatch failed: {}",
        provider_error_message(message).unwrap_or_else(|| compact_provider_notice_message(message))
    )
}

fn project_terminal_failure_chunk(
    poll_result: &mut crate::provider::ProviderPromptSignalBatch,
    message: Option<&str>,
) {
    let Some(message) = message else {
        return;
    };
    poll_result
        .chunks
        .retain(|chunk| chunk.kind != crate::terminal::TerminalOutputKind::ProviderError);
    poll_result
        .chunks
        .push(crate::provider::ProviderPromptChunk {
            kind: crate::terminal::TerminalOutputKind::ProviderError,
            merge_key: None,
            bytes: message.as_bytes().to_vec(),
        });
}

fn provider_notice_message(message: &str) -> String {
    provider_error_message(message)
        .map(|message| format!("Provider prompt dispatch failed: {message}"))
        .unwrap_or_else(|| message.to_string())
}

fn provider_error_message(message: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(message).ok()?;
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("message").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToOwned::to_owned)
}

fn compact_provider_notice_message(message: &str) -> String {
    let mut compact = message.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_CHARS: usize = 500;
    if compact.chars().count() > MAX_CHARS {
        compact = compact.chars().take(MAX_CHARS).collect::<String>();
        compact.push_str("...");
    }
    compact
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representative_provider_exhaustion_envelopes_project_one_canonical_error() {
        let cases = [
            (
                "Codex",
                r#"{"error":{"type":"usage_limit_reached","message":"You have no weighted tokens left"}}"#,
                "You have no weighted tokens left",
            ),
            (
                "Claude",
                "You've hit your usage limit. Your limit will reset later.",
                "You've hit your usage limit. Your limit will reset later.",
            ),
            (
                "OpenCode",
                "Insufficient balance. Manage your billing to continue.",
                "Insufficient balance. Manage your billing to continue.",
            ),
        ];

        for (provider, envelope, expected) in cases {
            let mut batch = crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderError,
                    merge_key: None,
                    bytes: format!("provider-specific rendering for {provider}").into_bytes(),
                }],
                terminal_failure: Some(envelope.to_string()),
                ..Default::default()
            };
            let message = provider_prompt_dispatch_failure_notice(envelope);

            project_terminal_failure_chunk(&mut batch, Some(&message));

            let errors = batch
                .chunks
                .iter()
                .filter(|chunk| chunk.kind == crate::terminal::TerminalOutputKind::ProviderError)
                .collect::<Vec<_>>();
            assert_eq!(errors.len(), 1, "{provider} should project one error");
            assert_eq!(
                String::from_utf8_lossy(&errors[0].bytes),
                format!("Provider prompt dispatch failed: {expected}"),
                "{provider} should preserve the provider explanation",
            );
        }
    }
}
