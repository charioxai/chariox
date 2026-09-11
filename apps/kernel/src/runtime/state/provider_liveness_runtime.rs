//! Provider process liveness reconciliation and unexpected-exit settlement.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn reconcile_provider_run_exit(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let owned = &self.owned;

        if let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            None,
        )? {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
            self.owned
                .connector_adapter_processes
                .shutdown_run(provider_run_id)
                .await;
            let session_outcome = if owned
                .provider_run_has_active_prompt(session_id, &exit.ended_run)?
            {
                let termination = crate::provider::ProviderRunTermination::runtime_failure(
                    "provider run was already ended during liveness reconciliation",
                    crate::session::unix_epoch_ms(),
                );
                let diagnosed = owned
                    .provider_store
                    .record_terminal_diagnostic(provider_run_id, termination.reason.clone())?;
                owned.provider_run_projection.update(diagnosed);
                self.settle_unexpected_provider_run_exit(
                    session_id,
                    provider_run_id,
                    exit.ended_run.agent_instance_id().ok_or_else(|| {
                        DaemonError::AgentNotFound {
                            agent_id: "provider run has no agent".to_string(),
                        }
                    })?,
                    termination,
                )
                .await?
            } else {
                self.settle_owned_provider_prompt(session_id, provider_run_id, false, false, true)
                    .await?
            };
            if session_outcome.had_active_prompt && !session_outcome.cancelled_prompt {
                let recipients = owned
                    .attachment_store
                    .list_session_attachment_ids(session_id);
                owned.record_notice(
                    session_id,
                    Some(provider_run_id),
                    recipients,
                    format!(
                        "Provider run `{}` for `{}` was already ended during liveness reconciliation. {}",
                        provider_run_id,
                        exit.ended_run.provider(),
                        if session_outcome.started_next_prompt {
                            "The active prompt was closed and Chariox advanced the queued backlog onto the next available provider run."
                        } else {
                            "The active prompt was closed without starting the queued backlog."
                        }
                    ),
                );
            }
            return Ok(exit.already_ended);
        }

        let process_exit = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).poll_exit(provider_run_id)
            })
            .await?;
        let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            Some(process_exit.is_none()),
        )?
        else {
            return Ok(false);
        };
        let (_, process_key) = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
            })
            .await
            .unwrap_or((false, None));
        owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
        self.owned
            .connector_adapter_processes
            .shutdown_run(provider_run_id)
            .await;
        if exit.already_ended {
            let _ = self
                .settle_owned_provider_prompt(session_id, provider_run_id, false, false, true)
                .await?;
            return Ok(true);
        }

        let agent_id =
            exit.ended_run
                .agent_instance_id()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: "provider run has no agent".to_string(),
                })?;
        let occurred_at_ms = crate::session::unix_epoch_ms();
        let termination = process_exit
            .and_then(|exit| exit.exit_code)
            .map(|exit_code| {
                crate::provider::ProviderRunTermination::process_exit(exit_code, occurred_at_ms)
            })
            .unwrap_or_else(|| {
                crate::provider::ProviderRunTermination::unknown_process_exit(occurred_at_ms)
            });
        let session_outcome = self
            .settle_unexpected_provider_run_exit(
                session_id,
                provider_run_id,
                agent_id,
                termination.clone(),
            )
            .await?;
        if !session_outcome.had_active_prompt || session_outcome.cancelled_prompt {
            return Ok(true);
        }
        let recipients = owned
            .attachment_store
            .list_session_attachment_ids(session_id);
        owned.record_notice(
            session_id,
            Some(provider_run_id),
            recipients,
            format!(
                "Provider run `{}` for `{}` ended unexpectedly: {}. {}",
                provider_run_id,
                exit.ended_run.provider(),
                termination.reason,
                if session_outcome.started_next_prompt {
                    "The active prompt was closed and Chariox advanced the queued backlog onto the next available provider run."
                } else {
                    "The active prompt was closed without starting the queued backlog."
                }
            ),
        );
        Ok(true)
    }

    pub(super) async fn settle_unexpected_provider_run_exit(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        termination: crate::provider::ProviderRunTermination,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let provider_run = self
            .owned
            .ensure_provider_run_in_session(session_id, provider_run_id)?;
        if !self
            .owned
            .provider_run_has_active_prompt(session_id, &provider_run)?
        {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        }
        let active_prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&self.owned.session_store.get_session(session_id)?, agent_id);
        let Some(active_prompt) = active_prompt else {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                cancelled_prompt: false,
                started_next_prompt: false,
            });
        };
        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            return self
                .settle_owned_provider_prompt(session_id, provider_run_id, false, false, true)
                .await;
        }
        let message = format!(
            "Provider run `{provider_run_id}` ended unexpectedly: {}.",
            termination.reason
        );
        self.fail_owned_provider_prompt_with_termination(
            session_id,
            provider_run_id,
            &message,
            true,
            Some(termination.clone()),
        )
        .await?;
        let started_next_prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&self.owned.session_store.get_session(session_id)?, agent_id)
            .is_some();
        Ok(crate::app::ProviderRunExitSessionSummary {
            had_active_prompt: true,
            cancelled_prompt: false,
            started_next_prompt,
        })
    }
}
