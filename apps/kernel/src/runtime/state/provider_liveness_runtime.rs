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
        let terminal_diagnostic = process_exit
            .as_ref()
            .and_then(|exit| exit.terminal_diagnostic.clone());
        if let Some(diagnostic) = terminal_diagnostic.as_deref() {
            let run = owned
                .provider_store
                .record_terminal_diagnostic(provider_run_id, diagnostic.to_string())?;
            owned.provider_run_projection.update(run);
        }
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
        let termination = termination_for_process_exit(
            process_exit
                .as_ref()
                .map(|exit| (exit.exit_code, exit.signal.as_deref())),
            crate::session::unix_epoch_ms(),
        );
        let session_outcome = self
            .settle_unexpected_provider_run_exit_with_diagnostic(
                session_id,
                provider_run_id,
                agent_id,
                termination.clone(),
                terminal_diagnostic.as_deref(),
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
        self.settle_unexpected_provider_run_exit_with_diagnostic(
            session_id,
            provider_run_id,
            agent_id,
            termination,
            None,
        )
        .await
    }

    async fn settle_unexpected_provider_run_exit_with_diagnostic(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        termination: crate::provider::ProviderRunTermination,
        terminal_diagnostic: Option<&str>,
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
        let message = match terminal_diagnostic {
            Some(diagnostic) if !diagnostic.trim().is_empty() => format!(
                "Provider run `{provider_run_id}` ended unexpectedly: {}. Provider terminal diagnostic: {diagnostic}",
                termination.reason
            ),
            _ => format!(
                "Provider run `{provider_run_id}` ended unexpectedly: {}.",
                termination.reason
            ),
        };
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

fn termination_for_process_exit(
    process_exit: Option<(Option<u32>, Option<&str>)>,
    occurred_at_ms: u64,
) -> crate::provider::ProviderRunTermination {
    match process_exit {
        Some((exit_code, signal)) => match (exit_code, signal) {
            (Some(exit_code), _) => {
                crate::provider::ProviderRunTermination::process_exit(exit_code, occurred_at_ms)
            }
            (None, Some(signal)) => {
                crate::provider::ProviderRunTermination::signal(signal, occurred_at_ms)
            }
            (None, None) => {
                crate::provider::ProviderRunTermination::unknown_process_exit(occurred_at_ms)
            }
        },
        None => crate::provider::ProviderRunTermination::unknown_process_exit(occurred_at_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::termination_for_process_exit;

    #[test]
    fn process_exit_mapping_keeps_code_signal_and_unknown_distinct() {
        let exit_code = termination_for_process_exit(Some((Some(1), None)), 42);
        assert_eq!(
            exit_code.category,
            crate::provider::ProviderRunTerminationCategory::ProcessExit
        );
        assert!(exit_code.reason.ends_with("status 1"));

        let signal = termination_for_process_exit(Some((None, Some("SIGTERM"))), 43);
        assert_eq!(
            signal.category,
            crate::provider::ProviderRunTerminationCategory::Signal
        );
        assert!(signal.reason.ends_with("signal SIGTERM"));

        let unknown = termination_for_process_exit(Some((None, None)), 44);
        assert_eq!(
            unknown.category,
            crate::provider::ProviderRunTerminationCategory::Unknown
        );
        assert!(unknown.reason.contains("without an available status"));
        assert_eq!(
            termination_for_process_exit(None, 45).category,
            crate::provider::ProviderRunTerminationCategory::Unknown
        );
    }
}
