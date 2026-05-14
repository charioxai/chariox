//! Provider output pumping and terminal snapshot orchestration.
//!
//! These methods bridge owned runtime state with provider processes/endpoints and translate
//! provider runtime events back into prompt/session mutations.

use super::*;

const PTY_PROMPT_SETTLE_QUIET_FOR: std::time::Duration = std::time::Duration::from_millis(50);

impl KernelRuntimeState {
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
