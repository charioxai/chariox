use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{ProviderRunLivenessReconciliation, RuntimeProviderRun};
use crate::pty::PtyProcessState;
use crate::session::PromptStatus;

use super::provider_processes::ProviderProcessTracker;
use super::provider_run_read::ProviderRunReadService;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ProviderRunExitPromptSettlement {
    FinalizeCancellation,
    FailActivePrompt,
    SyncIdleProvider,
}

impl ProviderRunExitPromptSettlement {
    fn from_active_prompt_status(active_prompt_status: Option<PromptStatus>) -> Self {
        match active_prompt_status {
            Some(PromptStatus::Cancelling) => Self::FinalizeCancellation,
            Some(_) => Self::FailActivePrompt,
            None => Self::SyncIdleProvider,
        }
    }
}

pub(crate) struct ProviderRunLivenessRuntime<'a> {
    app: &'a mut DaemonApp,
}

#[derive(Debug, Clone)]
struct ProviderRunLivenessOutcome {
    ended_run: RuntimeProviderRun,
    session_id: String,
    provider_run_id: String,
    agent_id: String,
    provider_termination: Option<crate::provider::ProviderRunTermination>,
    transition: ProviderRunLivenessTransition,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ProviderRunLivenessTransition {
    AlreadyEnded,
    UnexpectedExit,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ProviderRunExitSessionOutcome {
    had_active_prompt: bool,
    cancelled_prompt: bool,
    started_next_prompt: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunExitSessionSummary {
    pub(crate) had_active_prompt: bool,
    pub(crate) cancelled_prompt: bool,
    pub(crate) started_next_prompt: bool,
}

struct ProviderRunLivenessProcesses;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderProcessExit {
    pub(crate) exit_code: Option<u32>,
    pub(crate) signal: Option<String>,
    pub(crate) terminal_diagnostic: Option<String>,
}

impl ProviderRunLivenessProcesses {
    fn poll_process_exit(
        app: &mut DaemonApp,
        provider_run_id: &str,
    ) -> Result<Option<ProviderProcessExit>, DaemonError> {
        match app.pty.poll_process_state(provider_run_id) {
            Ok(PtyProcessState::Running) => Ok(None),
            Ok(PtyProcessState::Exited { exit_code, signal }) => {
                let terminal_diagnostic = app
                    .pty
                    .drain_output(provider_run_id)?
                    .into_iter()
                    .map(|chunk| String::from_utf8_lossy(&chunk.bytes).into_owned())
                    .collect::<String>();
                let terminal_diagnostic =
                    crate::provider::sanitize_provider_diagnostic(&terminal_diagnostic);
                Ok(Some(ProviderProcessExit {
                    exit_code,
                    signal,
                    terminal_diagnostic: (!terminal_diagnostic.is_empty())
                        .then_some(terminal_diagnostic),
                }))
            }
            Err(DaemonError::PtyProcessNotFound { .. }) => Ok(Some(ProviderProcessExit {
                exit_code: None,
                signal: None,
                terminal_diagnostic: None,
            })),
            Err(error) => Err(error),
        }
    }

    fn remove_tracked_process(
        app: &mut DaemonApp,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        ProviderProcessTracker::new(app).remove_run(provider_run_id)
    }
}

struct ProviderRunLivenessState;

impl ProviderRunLivenessState {
    fn reconcile_run_liveness(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
        process_running: Option<bool>,
    ) -> Result<ProviderRunLivenessReconciliation, DaemonError> {
        let reconciliation = app.providers.reconcile_run_liveness_provider_only(
            session_id,
            provider_run_id,
            process_running,
        )?;
        Self::sync_ended_provider_run_session_state(
            app,
            session_id,
            provider_run_id,
            &reconciliation,
        )?;
        Ok(reconciliation)
    }

    fn sync_ended_provider_run_session_state(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
        reconciliation: &ProviderRunLivenessReconciliation,
    ) -> Result<(), DaemonError> {
        if !matches!(
            reconciliation,
            ProviderRunLivenessReconciliation::AlreadyEnded(_)
                | ProviderRunLivenessReconciliation::NewlyEnded(_)
        ) {
            return Ok(());
        }
        clear_active_provider_run_session_pointer(app, session_id, provider_run_id)
    }
}

pub(super) fn poll_provider_run_process_exit(
    app: &mut DaemonApp,
    provider_run_id: &str,
) -> Result<Option<ProviderProcessExit>, DaemonError> {
    ProviderRunLivenessProcesses::poll_process_exit(app, provider_run_id)
}

pub(crate) fn clear_active_provider_run_session_pointer(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
) -> Result<(), DaemonError> {
    if app
        .sessions
        .get_session(session_id)?
        .active_provider_run_id()
        == Some(provider_run_id)
    {
        app.sessions.set_active_provider_run(session_id, None)?;
    }
    Ok(())
}

struct ProviderRunLivenessNotices;

impl ProviderRunLivenessNotices {
    fn record_provider_exit(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
        message: String,
    ) {
        let recipients = app.attachments.list_session_attachment_ids(session_id);
        app.record_notice(session_id, Some(provider_run_id), recipients, message);
    }
}

struct ProviderRunLivenessSessionEffects;

impl ProviderRunLivenessSessionEffects {
    fn apply_provider_exit(
        app: &mut DaemonApp,
        outcome: &ProviderRunLivenessOutcome,
    ) -> Result<ProviderRunExitSessionOutcome, DaemonError> {
        let active_prompt_status = app
            .prompt_owner_active_prompt_for_agent(&outcome.session_id, &outcome.agent_id)?
            .and_then(|prompt| prompt.is_chariox_owned().then(|| prompt.status()));
        let had_active_prompt = active_prompt_status.is_some();
        // MP-08 / MP-10 / MP-11: native process exit is not an interruption receipt.
        let native_cancel_failed = active_prompt_status == Some(PromptStatus::Cancelling)
            && crate::provider::provider_run_is_claude_native_tui(&outcome.ended_run)
            && !app
                .active_turns
                .get(&outcome.provider_run_id)
                .is_some_and(|turn| turn.native_interruption_confirmed);
        let cancelled_prompt =
            active_prompt_status == Some(PromptStatus::Cancelling) && !native_cancel_failed;
        let settlement = if native_cancel_failed {
            ProviderRunExitPromptSettlement::FailActivePrompt
        } else {
            ProviderRunExitPromptSettlement::from_active_prompt_status(active_prompt_status)
        };
        let termination = if native_cancel_failed {
            Some(crate::provider::ProviderRunTermination::runtime_failure(
                super::provider_output_claude_native::CLAUDE_CANCELLATION_EXIT_FAILURE,
                crate::session::unix_epoch_ms(),
            ))
        } else {
            outcome.provider_termination.clone()
        };
        let started_next_prompt = match settlement {
            ProviderRunExitPromptSettlement::FinalizeCancellation => app
                .finalize_active_prompt_cancellation(
                    &outcome.session_id,
                    &outcome.agent_id,
                    Some(&outcome.provider_run_id),
                )?
                .started_next
                .is_some(),
            ProviderRunExitPromptSettlement::FailActivePrompt => app
                .fail_active_prompt_with_termination(
                    &outcome.session_id,
                    &outcome.agent_id,
                    Some(&outcome.provider_run_id),
                    termination,
                )?
                .started_next
                .is_some(),
            ProviderRunExitPromptSettlement::SyncIdleProvider => {
                app.sync_focused_provider_run_if_idle(&outcome.session_id)?;
                false
            }
        };
        if !cancelled_prompt
            && app
                .agents
                .mark_unexpected_provider_exit_error(&outcome.agent_id, had_active_prompt)?
        {
            let _ = crate::app::KernelSessionReadService::new(app)
                .session_snapshot(&outcome.session_id)?;
        }

        Ok(ProviderRunExitSessionOutcome {
            had_active_prompt,
            cancelled_prompt,
            started_next_prompt,
        })
    }
}

impl<'a> ProviderRunLivenessRuntime<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn reconcile_provider_run_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(outcome) =
            self.reconcile_provider_run_exit_provider_phase(session_id, provider_run_id)?
        else {
            return Ok(false);
        };
        if outcome.transition == ProviderRunLivenessTransition::AlreadyEnded {
            return Ok(true);
        }

        let session_outcome =
            ProviderRunLivenessSessionEffects::apply_provider_exit(self.app, &outcome)?;
        if !session_outcome.had_active_prompt || session_outcome.cancelled_prompt {
            return Ok(true);
        }
        ProviderRunLivenessNotices::record_provider_exit(
            self.app,
            &outcome.session_id,
            &outcome.provider_run_id,
            format!(
                "Provider run `{}` for `{}` ended unexpectedly. {}",
                outcome.provider_run_id,
                outcome.ended_run.provider(),
                if session_outcome.started_next_prompt {
                    "The active prompt was closed and Chariox advanced the queued backlog onto the next available provider run."
                } else {
                    "The active prompt was closed without starting the queued backlog."
                }
            ),
        );

        Ok(true)
    }

    fn reconcile_provider_run_exit_provider_phase(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<ProviderRunLivenessOutcome>, DaemonError> {
        let provider_run = ProviderRunReadService::new(self.app)
            .ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        match ProviderRunLivenessState::reconcile_run_liveness(
            self.app,
            session_id,
            provider_run_id,
            None,
        )? {
            ProviderRunLivenessReconciliation::AlreadyEnded(run) => {
                self.app.update_provider_run_projection(run.clone());
                let _ = ProviderRunLivenessProcesses::remove_tracked_process(
                    self.app,
                    provider_run_id,
                )?;
                return Ok(Some(ProviderRunLivenessOutcome {
                    ended_run: run,
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id,
                    provider_termination: None,
                    transition: ProviderRunLivenessTransition::AlreadyEnded,
                }));
            }
            ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | ProviderRunLivenessReconciliation::NewlyEnded(_) => return Ok(None),
            ProviderRunLivenessReconciliation::StillRunning(_) => {}
        }

        let process_exit =
            ProviderRunLivenessProcesses::poll_process_exit(self.app, provider_run_id)?;
        if let Some(diagnostic) = process_exit
            .as_ref()
            .and_then(|exit| exit.terminal_diagnostic.as_deref())
        {
            let run = self
                .app
                .providers
                .record_terminal_diagnostic(provider_run_id, diagnostic.to_string())?;
            self.app.update_provider_run_projection(run);
        }
        let ended_run = match ProviderRunLivenessState::reconcile_run_liveness(
            self.app,
            session_id,
            provider_run_id,
            Some(process_exit.is_none()),
        )? {
            ProviderRunLivenessReconciliation::AlreadyEnded(run)
            | ProviderRunLivenessReconciliation::NewlyEnded(run) => run,
            ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | ProviderRunLivenessReconciliation::StillRunning(_) => return Ok(None),
        };
        self.app.update_provider_run_projection(ended_run.clone());
        let _ = ProviderRunLivenessProcesses::remove_tracked_process(self.app, provider_run_id)?;
        let provider_termination = process_exit.as_ref().map(|process_exit| {
            match (process_exit.exit_code, process_exit.signal.as_deref()) {
                (Some(exit_code), _) => crate::provider::ProviderRunTermination::process_exit(
                    exit_code,
                    crate::session::unix_epoch_ms(),
                ),
                (None, Some(signal)) => crate::provider::ProviderRunTermination::signal(
                    signal,
                    crate::session::unix_epoch_ms(),
                ),
                (None, None) => crate::provider::ProviderRunTermination::unknown_process_exit(
                    crate::session::unix_epoch_ms(),
                ),
            }
        });

        Ok(Some(ProviderRunLivenessOutcome {
            ended_run,
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id,
            provider_termination,
            transition: ProviderRunLivenessTransition::UnexpectedExit,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::KernelSessionService;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::provider::LaunchProviderRequest;

    #[test]
    fn app_unexpected_provider_exit_marks_active_agent_error() {
        assert_app_provider_exit_state(false, None);
    }

    #[test]
    fn app_cancelled_provider_exit_does_not_mark_agent_error() {
        assert_app_provider_exit_state(true, None);
    }

    #[test]
    fn native_cancel_process_exit_requires_an_official_receipt() {
        assert_app_provider_exit_state(true, Some(false));
        assert_app_provider_exit_state(true, Some(true));
    }

    fn assert_app_provider_exit_state(cancelling: bool, native_receipt: Option<bool>) {
        let worktree = crate::test_support::TestWorktree::new("app-unexpected-exit");
        let mut app =
            crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
                .expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .expect("session should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-app-unexpected-exit",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let run = app
            .launch_provider(
                LaunchProviderRequest::new(session.id(), "dev-stub", "codex", "default", "gpt-5")
                    .with_agent_id(agent.id()),
            )
            .expect("provider should launch");
        app.submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "do work\n",
            Vec::new(),
        )
        .expect("prompt should start");
        if cancelling {
            app.prompt_owner_begin_cancelling_active_prompt(session.id(), agent.id())
                .expect("cancellation should precede provider exit");
        }
        let ended_run = app
            .providers_mut()
            .mark_run_ended_provider_only(session.id(), run.id())
            .expect("provider should end")
            .into_run();
        let ended_run = if let Some(confirmed) = native_receipt {
            if confirmed {
                let turn = app
                    .active_turns
                    .get(run.id())
                    .expect("admitted turn exists");
                app.active_turns
                    .confirm_native_interruption(run.id(), &turn.prompt_id);
            }
            let request =
                LaunchProviderRequest::new(session.id(), "claude", "claude", "default", "haiku")
                    .with_agent_id(agent.id())
                    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
            let mut native = RuntimeProviderRun::new(
                run.id(),
                &request,
                crate::provider::ProviderLaunchResult {
                    endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                    process_label: "native-exit-metadata".into(),
                    pty_target: None,
                    pty_program: None,
                    pty_args: vec![],
                    pty_env: Default::default(),
                    pty_env_remove: vec![],
                    working_directory: None,
                    structured_endpoint: None,
                },
            );
            native.mark_ended();
            native
        } else {
            ended_run
        };
        let outcome = ProviderRunLivenessOutcome {
            ended_run,
            session_id: session.id().to_string(),
            provider_run_id: run.id().to_string(),
            agent_id: agent.id().to_string(),
            provider_termination: None,
            transition: ProviderRunLivenessTransition::UnexpectedExit,
        };

        let session_outcome =
            ProviderRunLivenessSessionEffects::apply_provider_exit(&mut app, &outcome)
                .expect("unexpected exit effects should apply");

        assert!(session_outcome.had_active_prompt);
        let cancelled = cancelling && native_receipt != Some(false);
        assert_eq!(session_outcome.cancelled_prompt, cancelled);
        assert!(app
            .sessions()
            .get_session(session.id())
            .expect("session should remain available")
            .active_prompt_for_agent(agent.id())
            .is_none());
        let state = app
            .agents
            .get_agent(agent.id())
            .expect("agent should remain available")
            .state();
        if cancelled {
            assert_ne!(state, crate::agent::AgentState::Error);
            let turn = app
                .completed_git_turn_snapshot_store()
                .latest_projection_for_agent(session.id(), agent.id())
                .expect("cancelled turn outcome should be projected even without Git changes");
            assert_eq!(
                turn.settlement_status,
                crate::git_observer::CompletedTurnSettlementStatus::Cancelled
            );
        } else {
            assert_eq!(state, crate::agent::AgentState::Error);
            if native_receipt == Some(false) {
                let turn = app
                    .completed_git_turn_snapshot_store()
                    .latest_projection_for_agent(session.id(), agent.id())
                    .unwrap();
                assert_eq!(
                    turn.settlement_status,
                    crate::git_observer::CompletedTurnSettlementStatus::Failed
                );
                assert!(turn
                    .provider_termination
                    .unwrap()
                    .reason
                    .contains("exited before acknowledging cancellation"));
            }
        }
    }
}

#[cfg(test)]
mod terminal_diagnostic_loss;
