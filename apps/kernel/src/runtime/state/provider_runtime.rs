//! Provider prompt settlement, output pumping, and cancellation orchestration.
//!
//! These methods bridge owned runtime state with provider processes/endpoints and translate
//! provider runtime events back into prompt/session mutations.

use super::*;

const PTY_PROMPT_SETTLE_QUIET_FOR: std::time::Duration = std::time::Duration::from_millis(50);

impl KernelRuntimeState {
    pub(super) async fn settle_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
        saw_settlement_blocking_activity: bool,
        force: bool,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        let active_prompt = owned
            .prompt_state_owner
            .active_prompt_for_agent(&owned.session_store.get_session(session_id)?, &agent_id);
        let Some(active_prompt) = active_prompt else {
            if !force && !prompt_completed {
                crate::logging::debug_with_fields(
                    "daemon.provider",
                    "settle provider prompt skipped without completion signal",
                    serde_json::json!({
                        "session_id": session_id,
                        "provider_run_id": provider_run_id,
                        "agent_id": agent_id,
                        "prompt_completed": prompt_completed,
                        "force": force,
                    }),
                );
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: false,
                    started_next_prompt: false,
                });
            }
            crate::logging::debug_with_fields(
                "daemon.provider",
                "settle provider prompt found no active prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_completed": prompt_completed,
                    "force": force,
                }),
            );
            if owned.clear_prompt_activity(provider_run_id) {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            let _ = owned.sync_focused_provider_run_if_idle(session_id);
            let _ = owned.session_snapshot(session_id);
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                started_next_prompt: false,
            });
        };

        let completion_recorded = owned.prompt_completion_recorded(provider_run_id);
        let settlement_pending = owned.prompt_completion_settlement_pending(provider_run_id);
        if !force && !prompt_completed && !settlement_pending {
            crate::logging::debug_with_fields(
                "daemon.provider",
                "settle provider prompt skipped until provider completion",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "active_prompt_status": active_prompt.status(),
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: false,
            });
        }

        if !force && completion_recorded && saw_settlement_blocking_activity {
            owned.note_prompt_settlement_requested(provider_run_id);
            let _ = owned.session_snapshot(session_id);
            crate::logging::debug_with_fields(
                "daemon.provider",
                "provider completion is draining final output",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: false,
            });
        }

        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            if !force && completion_recorded && saw_settlement_blocking_activity {
                owned.note_prompt_settlement_requested(provider_run_id);
                let _ = owned.session_snapshot(session_id);
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: true,
                    started_next_prompt: false,
                });
            }
            let cancellation = owned.finalize_local_prompt_cancellation_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?;
            owned.workflow_cancel_prompt(session_id, &cancellation.cancellation.prompt)?;
            if cancellation.released_claim {
                self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            }
            if let Some(dispatch) = cancellation.dispatch {
                if let Err(error) = self
                    .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                    .await
                {
                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                }
            }
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: cancellation.cancellation.started_next.is_some(),
            });
        }

        if (prompt_completed || settlement_pending)
            && !force
            && active_prompt.workflow_run_id().is_some()
            && active_prompt.workflow_node_run_id().is_some()
            && !owned.workflow_prompt_has_completion_output(
                session_id,
                active_prompt
                    .workflow_run_id()
                    .expect("workflow run id checked"),
                active_prompt
                    .workflow_node_run_id()
                    .expect("workflow node run id checked"),
                provider_run_id,
            )
        {
            owned.note_prompt_settlement_requested(provider_run_id);
            let _ = owned.session_snapshot(session_id);
            crate::logging::debug_with_fields(
                "daemon.provider",
                "provider completed workflow prompt before workflow output",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": active_prompt.id(),
                }),
            );
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: false,
            });
        }

        if !force && (prompt_completed || settlement_pending) {
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                active_prompt.workflow_run_id(),
                active_prompt.workflow_node_run_id(),
            ) {
                if !owned.workflow_prompt_has_completion_output(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                    provider_run_id,
                ) {
                    let message =
                        "provider completed workflow turn without a validated workflow output";
                    owned.workflow_fail_provider_prompt(
                        session_id,
                        &active_prompt,
                        Some(provider_run_id),
                        message,
                    )?;
                    let _ = owned.complete_local_prompt_without_advance(
                        session_id,
                        &agent_id,
                        Some(provider_run_id),
                    )?;
                    return Ok(crate::app::ProviderRunExitSessionSummary {
                        had_active_prompt: true,
                        started_next_prompt: false,
                    });
                }
            }
        }
        let provider_run_state = provider_run.state();
        let next_queued_prompt = if provider_run_state == crate::provider::ProviderRunState::Running
        {
            owned
                .prompt_state_owner
                .peek_next_queued_prompt(&owned.session_store.get_session(session_id)?, &agent_id)
        } else {
            None
        };
        let completion = if let Some(next_queued_prompt) = next_queued_prompt.as_ref() {
            owned.complete_local_prompt_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
                next_queued_prompt,
            )?
        } else {
            owned.complete_local_prompt_without_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?
        }
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "settle provider prompt",
            message: "owned prompt runtime could not settle provider prompt".to_string(),
        })?;
        crate::logging::debug_with_fields(
            "daemon.provider",
            "settled provider prompt",
            serde_json::json!({
                "session_id": session_id,
                "provider_run_id": provider_run_id,
                "agent_id": agent_id,
                "prompt_completed": prompt_completed,
                "force": force,
                "started_next": completion.completion.started_next.is_some(),
                "released_claim": completion.released_claim,
            }),
        );
        if completion.completion.completed.workflow_run_id().is_some() {
            let mut dispatches = owned.workflow_complete_prompt(
                session_id,
                &completion.completion.completed,
                Some(provider_run_id),
            )?;
            if completion.released_claim {
                dispatches.extend(owned.workflow_retry_blocked_claims());
            }
            self.spawn_workflow_prompt_dispatches(dispatches);
        }
        if let Some(started_next) = completion.completion.started_next.as_ref() {
            if crate::scheduler::runtime::is_workflow_prompt_attachment(
                started_next.source_attachment_id(),
            ) {
                owned.workflow_start_prompt(session_id, started_next)?;
            }
        }
        if completion.released_claim && completion.completion.completed.workflow_run_id().is_none()
        {
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        }
        if let Some(dispatch) = completion.dispatch {
            if let Err(error) = self
                .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                .await
            {
                let _ = self.fail_prompt_dispatch(dispatch, error).await;
            }
        }
        let state = self.clone();
        let session_id_for_continuation = session_id.to_string();
        let agent_id_for_continuation = agent_id.clone();
        let continuation: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
            Box::pin(async move {
                if let Err(error) = state
                    .run_pending_mcp_continuation_after_completion(
                        &session_id_for_continuation,
                        &agent_id_for_continuation,
                    )
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.provider",
                        "pending MCP continuation failed",
                        serde_json::json!({
                            "session_id": session_id_for_continuation,
                            "agent_id": agent_id_for_continuation,
                            "error": error.to_string(),
                        }),
                    );
                }
            });
        tokio::spawn(continuation);
        Ok(crate::app::ProviderRunExitSessionSummary {
            had_active_prompt: true,
            started_next_prompt: completion.completion.started_next.is_some(),
        })
    }

    async fn fail_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;

        let leased_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_workflow_turn_context_for_provider_run(provider_run_id)
            })
            .await;
        if let Some(context) = leased_context {
            let response = self
                .with_app_side_effect(|app| {
                    app.block_on_relay_future(
                        crate::transport::relay_client::send_peer_request_via_temporary_connection(
                            app.config(),
                            arroba_relay::protocol::ClientTarget {
                                daemon_id: Some(context.home_kernel_id.clone()),
                                daemon_alias: None,
                            },
                            crate::transport::relay_peer::RelayPeerRequest::ForwardWorkflowProviderFailure {
                                context,
                                message: message.to_string(),
                            },
                        ),
                    )
                })
                .await?;
            if !matches!(
                response,
                crate::transport::relay_peer::RelayPeerResponse::WorkflowProviderFailureHandled
            ) {
                return Err(DaemonError::LocalTransport {
                    operation: "forward workflow provider failure",
                    message: format!("unexpected workflow provider failure response: {response:?}"),
                });
            }
            let _ = self
                .with_app_side_effect(|app| {
                    crate::app::RemoteLeaseRuntime::new(app)
                        .complete_leased_workflow_prompt_for_provider_run(provider_run_id)
                })
                .await?;
            return Ok(());
        }

        let session = owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
        else {
            return Ok(());
        };
        if active_prompt.workflow_run_id().is_some() {
            owned.workflow_fail_provider_prompt(
                session_id,
                &active_prompt,
                Some(provider_run_id),
                message,
            )?;
        }
        let completion = owned.complete_local_prompt_without_advance(
            session_id,
            &agent_id,
            Some(provider_run_id),
        )?;
        if completion
            .as_ref()
            .is_some_and(|completion| completion.released_claim)
            && active_prompt.workflow_run_id().is_none()
        {
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        }
        if let Some(reason) = crate::provider::classify_provider_substitutable_failure_text(
            provider_run.adapter_key(),
            message,
        ) {
            if let Err(error) = self
                .activate_next_agent_substitute_after_failure(session_id, &agent_id, &reason)
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "automatic substitute activation after provider failure failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
        Ok(())
    }

    pub(super) async fn pump_owned_provider_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        initial_liveness_already_checked: bool,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        owned.reap_structured_prompt_jobs();
        if !initial_liveness_already_checked
            && self
                .reconcile_provider_run_exit(session_id, provider_run_id)
                .await?
        {
            return Ok(Vec::new());
        }
        let mut provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            return Ok(Vec::new());
        }
        if provider_run.state() == crate::provider::ProviderRunState::Parked {
            if !owned.provider_run_has_active_prompt(session_id, &provider_run)? {
                return Ok(Vec::new());
            }
            provider_run = owned.provider_store.resume_run_detached(provider_run_id)?;
            owned.provider_run_projection.update(provider_run.clone());
            crate::logging::warn_with_fields(
                "daemon.provider",
                "resumed parked provider run that still had an active prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": provider_run.agent_instance_id(),
                }),
            );
        }

        if provider_run.client_interface().is_arroba()
            && owned
                .provider_store
                .run_uses_structured_prompt_io(&provider_run)
        {
            return self
                .pump_owned_structured_provider_output(
                    session_id,
                    provider_run_id,
                    recipient_attachment_ids,
                )
                .await;
        }

        let chunks = match self
            .with_app_side_effect(|app| app.drain_provider_pty_output_for_runtime(provider_run_id))
            .await
        {
            Ok(chunks) => chunks,
            Err(error) => {
                if self
                    .reconcile_provider_run_exit(session_id, provider_run_id)
                    .await?
                {
                    return Ok(Vec::new());
                }
                return Err(error);
            }
        };
        if !chunks.is_empty() {
            owned.note_prompt_response_content(provider_run_id);
        }
        let terminal_failure = crate::provider::classify_provider_terminal_failure_text(
            provider_run.adapter_key(),
            &chunks
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.bytes))
                .collect::<String>(),
        );
        let records = chunks
            .into_iter()
            .map(|chunk| {
                owned.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    None,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect::<Vec<_>>();
        if let Some(message) = terminal_failure {
            let run = owned
                .provider_store
                .record_terminal_diagnostic(provider_run_id, message.clone())?;
            owned.provider_run_projection.update(run);
            self.fail_owned_provider_prompt(session_id, provider_run_id, &message)
                .await?;
            return Ok(records);
        }
        if !self
            .reconcile_provider_run_exit(session_id, provider_run_id)
            .await?
        {
            if records.is_empty() {
                let _ = self
                    .settle_owned_pty_prompt_if_quiet(session_id, provider_run_id)
                    .await?;
            }
        }
        Ok(records)
    }

    async fn settle_owned_pty_prompt_if_quiet(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let owned = &self.owned;
        if !owned.prompt_output_quiet_after_response(provider_run_id, PTY_PROMPT_SETTLE_QUIET_FOR) {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                started_next_prompt: false,
            });
        }
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                started_next_prompt: false,
            });
        };
        let session = owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                started_next_prompt: false,
            });
        };
        if active_prompt.status() != crate::session::PromptStatus::Cancelling {
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                active_prompt.workflow_run_id(),
                active_prompt.workflow_node_run_id(),
            ) {
                if !owned.workflow_prompt_has_completion_output(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                    provider_run_id,
                ) {
                    return Ok(crate::app::ProviderRunExitSessionSummary {
                        had_active_prompt: true,
                        started_next_prompt: false,
                    });
                }
            }
        }
        self.settle_owned_provider_prompt(session_id, provider_run_id, true, false, false)
            .await
    }

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

    pub(crate) async fn pump_terminal_output_with_snapshot(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<
        (
            Vec<crate::terminal::TerminalOutputRecord>,
            Option<crate::session::RuntimeSession>,
        ),
        DaemonError,
    > {
        let owned = &self.owned;
        owned.reap_structured_prompt_jobs();
        owned.ensure_attachment_in_session(session_id, attachment_id)?;
        let active_provider_run_id = owned
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);
        let mut provider_run_ids = BTreeSet::new();
        if let Some(provider_run_id) = active_provider_run_id {
            provider_run_ids.insert(provider_run_id);
        }
        provider_run_ids.extend(
            owned
                .provider_store
                .list_runs()
                .into_iter()
                .filter(|run| run.session_id() == session_id)
                .filter(|run| {
                    matches!(
                        run.state(),
                        crate::provider::ProviderRunState::Starting
                            | crate::provider::ProviderRunState::Running
                    )
                })
                .map(|run| run.id().to_string()),
        );
        let recipient_attachment_ids = owned
            .attachment_store
            .list_session_attachment_ids(session_id);
        for provider_run_id in provider_run_ids {
            let _ = self
                .pump_owned_provider_output(
                    session_id,
                    &provider_run_id,
                    recipient_attachment_ids.clone(),
                    false,
                )
                .await?;
        }
        let records = owned
            .terminal_stream
            .drain_output_records(session_id, attachment_id);
        let session = owned.session_snapshot(session_id).ok();
        Ok((records, session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let app_locked = app.lock().await;
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.history_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.session_history_projection_store(),
            app_locked.prompt_state_owner(),
            app_locked.active_turn_store(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workspace_coordinator(),
        )
    }

    #[tokio::test]
    async fn provider_switch_does_not_park_runs_with_active_prompts() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-1",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let second_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("second"),
            )
            .expect("second agent should spawn");
        let idle_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("idle"),
            )
            .expect("idle agent should spawn");

        let first_run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(first_agent.id()),
            )
            .expect("first provider should launch");
        app.update_provider_run_projection(first_run.clone());
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(first_agent.id()),
            "first prompt\n",
            Vec::new(),
        )
        .expect("first prompt should start");

        let second_run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(second_agent.id()),
            )
            .expect("second provider should launch");
        app.update_provider_run_projection(second_run.clone());

        assert_eq!(
            app.providers
                .get_run(first_run.id())
                .expect("first run should exist")
                .state(),
            crate::provider::ProviderRunState::Running,
            "launching another provider must not park a run that owns an active prompt",
        );

        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(second_agent.id()),
            "second prompt\n",
            Vec::new(),
        )
        .expect("second prompt should start");
        crate::app::KernelSessionService::new(&mut app)
            .focus_agent(session.id(), idle_agent.id())
            .expect("idle agent focus should succeed");

        assert_eq!(
            app.providers
                .get_run(second_run.id())
                .expect("second run should exist")
                .state(),
            crate::provider::ProviderRunState::Running,
            "focusing an idle agent while multiple prompts are active must not park active work",
        );
        assert_eq!(
            app.sessions
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            Some(second_run.id()),
            "ambiguous multi-agent prompt work should keep the active provider pointer stable",
        );
    }

    #[tokio::test]
    async fn provider_completed_signal_settles_matching_active_prompt() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-1",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run should launch");
        app.update_provider_run_projection(run.clone());
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "status\n",
            Vec::new(),
        )
        .expect("prompt should start");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let first_settlement = runtime
            .settle_owned_provider_prompt(session.id(), run.id(), true, false, false)
            .await
            .expect("provider completion signal should be accepted");
        assert!(first_settlement.had_active_prompt);
        assert!(!first_settlement.started_next_prompt);
        assert!(runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should exist")
            .active_prompt_for_agent(agent.id())
            .is_none());
    }

    #[tokio::test]
    async fn provider_completion_with_output_waits_for_quiet_poll_before_settling() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-completion-drain",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run should launch");
        app.update_provider_run_projection(run.clone());
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "status\n",
            Vec::new(),
        )
        .expect("prompt should start");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let records = runtime
            .apply_owned_structured_output_batch(
                session.id(),
                run.id(),
                vec![attachment.id().to_string()],
                crate::provider::ProviderPromptSignalBatch {
                    chunks: vec![crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                        merge_key: Some("assistant-final".to_string()),
                        bytes: b"final output".to_vec(),
                    }],
                    completions: vec![crate::provider::ProviderAssistantCompletion {
                        message_id: "assistant-final".to_string(),
                        completed_at_ms: crate::session::unix_epoch_ms(),
                    }],
                    prompt_completed: true,
                    ..crate::provider::ProviderPromptSignalBatch::default()
                },
            )
            .await
            .expect("completion batch with output should be accepted");
        assert_eq!(records.len(), 1);

        let draining_session = runtime
            .owned
            .session_snapshot(session.id())
            .expect("session snapshot should exist");
        assert!(
            draining_session
                .active_prompt_for_agent(agent.id())
                .is_some(),
            "final output and completion in the same batch should keep the turn settling"
        );
        let draining_activity = runtime
            .agent_activity_for_session(&draining_session)
            .get(agent.id())
            .cloned()
            .expect("agent activity should be projected");
        assert!(draining_activity.busy);
        assert_eq!(
            draining_activity.prompt_status,
            crate::runtime::projection::AgentPromptRuntimeStatus::Settling
        );

        runtime
            .apply_owned_structured_output_batch(
                session.id(),
                run.id(),
                vec![attachment.id().to_string()],
                crate::provider::ProviderPromptSignalBatch::default(),
            )
            .await
            .expect("quiet poll should settle the completed prompt");
        let settled_session = runtime
            .owned
            .session_snapshot(session.id())
            .expect("session snapshot should exist");
        assert!(settled_session
            .active_prompt_for_agent(agent.id())
            .is_none());
    }

    #[tokio::test]
    async fn provider_quiet_gap_does_not_settle_without_completion_signal() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-quiet-gap",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run should launch");
        app.update_provider_run_projection(run.clone());
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "long quiet turn\n",
            Vec::new(),
        )
        .expect("prompt should start");

        if let Some(state) = app.prompt_activity.write().get_mut(run.id()) {
            state.saw_response_content = true;
            state.last_output_at =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(5));
        } else {
            panic!("prompt activity should exist for the active run");
        }

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let settlement = runtime
            .settle_owned_provider_prompt(session.id(), run.id(), false, false, false)
            .await
            .expect("quiet provider poll should be accepted");
        assert!(settlement.had_active_prompt);
        assert!(!settlement.started_next_prompt);

        let session_state = runtime
            .owned
            .session_snapshot(session.id())
            .expect("session snapshot should exist");
        assert!(session_state.active_prompt_for_agent(agent.id()).is_some());
        let activity = runtime.agent_activity_for_session(&session_state);
        let agent_activity = activity
            .get(agent.id())
            .expect("agent activity should be projected");
        assert_eq!(
            agent_activity.status,
            crate::runtime::projection::AgentRuntimeStatus::Working
        );
        assert_eq!(
            agent_activity.prompt_status,
            crate::runtime::projection::AgentPromptRuntimeStatus::Running
        );
        assert!(agent_activity.busy);
        assert!(agent_activity.active_turn.is_some());
    }

    #[tokio::test]
    async fn codex_tool_output_text_does_not_classify_as_terminal_failure() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-tool-output",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "codex",
            "codex",
            "default",
            "gpt-5.5",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-codex",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-codex".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-codex-runtime".to_string()),
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");
        app.update_provider_run_projection(run.clone());

        let prompt = crate::session::PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "read tool output\n",
            crate::session::PromptStatus::Queued,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should start");
        crate::transport::flow_control::note_prompt_started(&mut app, run.id());

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let records = runtime
            .apply_owned_structured_output_batch(
                session.id(),
                run.id(),
                vec![attachment.id().to_string()],
                crate::provider::ProviderPromptSignalBatch {
                    chunks: vec![crate::provider::ProviderPromptChunk {
                        kind: crate::terminal::TerminalOutputKind::ProviderTool,
                        merge_key: Some("tool-call-1".to_string()),
                        bytes: br#"{"tool":"bash","status":"completed","output":"Check if a CLAUDE.md file exists in the project root. If it does not exist, create it. This text mentions model context but is normal tool output."}"#.to_vec(),
                    }],
                    ..crate::provider::ProviderPromptSignalBatch::default()
                },
            )
            .await
            .expect("structured tool output should be accepted");

        assert_eq!(records.len(), 1);
        let session_state = runtime
            .owned
            .session_snapshot(session.id())
            .expect("session snapshot should exist");
        assert!(
            session_state.active_prompt_for_agent(agent.id()).is_some(),
            "normal Codex tool output must not settle the active prompt"
        );
        let agent_activity = runtime
            .agent_activity_for_session(&session_state)
            .get(agent.id())
            .cloned()
            .expect("agent activity should be projected");
        assert!(agent_activity.busy);
        assert_eq!(
            runtime
                .owned
                .provider_store
                .get_run(run.id())
                .expect("provider run should exist")
                .terminal_diagnostic(),
            None
        );
    }
}
