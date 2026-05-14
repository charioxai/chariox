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
                return Ok(Vec::new());
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
                    return Ok(Vec::new());
                }
                if !matches!(error, DaemonError::PtyProcessNotFound { .. }) {
                    return Err(error);
                }
            }
        }
        let mut records = owned.structured_output_records.take(provider_run_id);
        for finished in owned
            .provider_store
            .drain_finished_structured_output_poll_jobs()
        {
            let finished_run_id = finished.provider_run_id.clone();
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
                Ok(None) => continue,
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
                        Ok(true) => continue,
                        Ok(false) if is_requested_run => return Err(error),
                        Ok(false) => {
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
            let run = match owned.provider_store.get_run(&finished_run_id) {
                Ok(run) => run,
                Err(_) => continue,
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
                    .append(finished_run_id, applied);
            }
        }
        owned
            .provider_store
            .enqueue_structured_output_poll(provider_run_id)?;
        Ok(records)
    }

    pub(super) async fn apply_owned_structured_output_batch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        poll_result: crate::provider::ProviderPromptSignalBatch,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
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
        owned.provider_run_projection.update(provider_run);
        for notice in &poll_result.notices {
            owned.record_notice(
                session_id,
                Some(provider_run_id),
                recipient_attachment_ids.clone(),
                notice.to_string(),
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
        for completion in &poll_result.completions {
            owned.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
            owned.mark_prompt_completion_recorded(provider_run_id);
        }
        let prompt_completed = poll_result.prompt_completed;
        let terminal_failure = poll_result.terminal_failure.clone();
        if let Some(message) = terminal_failure.as_deref() {
            let run = owned
                .provider_store
                .record_terminal_diagnostic(provider_run_id, message.to_string())?;
            owned.provider_run_projection.update(run);
        }
        let records = poll_result
            .chunks
            .into_iter()
            .map(|chunk| {
                owned.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    chunk.kind,
                    chunk.merge_key,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect::<Vec<_>>();
        if let Some(message) = terminal_failure {
            self.fail_owned_provider_prompt(session_id, provider_run_id, &message)
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
        Ok(records)
    }
}
