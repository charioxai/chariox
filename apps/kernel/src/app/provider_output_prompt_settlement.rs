use std::time::{Duration, Instant};

use crate::app::{ActivePromptState, ActiveTurnStore, DaemonApp, PromptActivityStore};
use crate::error::DaemonError;
use crate::provider::ProviderProcessServiceStore;
use crate::runtime::projection::AgentRuntimeProjectionStore;
use crate::session::{PromptQueueItem, PromptStatus};

const PTY_PROMPT_SETTLE_QUIET_FOR: Duration = Duration::from_millis(50);
const STRUCTURED_PROMPT_SETTLE_QUIET_FOR: Duration = Duration::from_millis(50);
const WORKFLOW_MISSING_OUTPUT_SETTLE_QUIET_FOR: Duration = Duration::from_millis(
    crate::app::provider_output::STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS + 50,
);

pub(crate) struct ProviderOutputPromptSettlement<'a> {
    app: &'a mut DaemonApp,
    provider_store: ProviderProcessServiceStore,
    active_turns: ActiveTurnStore,
    prompt_activity: PromptActivityStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
}

impl<'a> ProviderOutputPromptSettlement<'a> {
    pub(crate) fn new(
        app: &'a mut DaemonApp,
        provider_store: ProviderProcessServiceStore,
        active_turns: ActiveTurnStore,
        prompt_activity: PromptActivityStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
    ) -> Self {
        Self {
            app,
            provider_store,
            active_turns,
            prompt_activity,
            agent_runtime_projection,
        }
    }

    pub(crate) fn settle_structured_completion(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
        saw_settlement_blocking_activity: bool,
    ) -> Result<(), DaemonError> {
        let Some(active_prompt) = self.active_prompt_for_settlement(session_id, provider_run_id)?
        else {
            return Ok(());
        };
        if active_prompt.delivery_pending() {
            return Ok(());
        }
        let active_prompt_status = active_prompt.status();
        if prompt_completed {
            crate::transport::flow_control::mark_prompt_completion_recorded(
                self.app,
                provider_run_id,
            );
        }
        let completion_recorded =
            crate::transport::flow_control::prompt_completion_recorded(self.app, provider_run_id);
        let mut settlement_pending =
            crate::transport::flow_control::prompt_completion_settlement_pending(
                self.app,
                provider_run_id,
            );
        if prompt_completed || settlement_pending {
            let quiet_after_response =
                crate::transport::flow_control::prompt_output_quiet_after_response(
                    self.app,
                    provider_run_id,
                    STRUCTURED_PROMPT_SETTLE_QUIET_FOR,
                );
            if !settlement_pending || saw_settlement_blocking_activity || !quiet_after_response {
                self.note_prompt_settlement_requested(provider_run_id);
                return Ok(());
            }
        }
        if !prompt_completed && !settlement_pending && completion_recorded {
            self.note_prompt_settlement_requested(provider_run_id);
            let _ =
                crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id);
            if saw_settlement_blocking_activity {
                return Ok(());
            }
            settlement_pending = true;
        }
        if active_prompt_status == PromptStatus::Cancelling {
            if (prompt_completed || settlement_pending) && !saw_settlement_blocking_activity {
                let agent_id = self.provider_run_agent_id(provider_run_id)?;
                let _ = self.app.finalize_active_prompt_cancellation(
                    session_id,
                    &agent_id,
                    Some(provider_run_id),
                )?;
                self.clear_active_turn(provider_run_id);
            }
        } else if prompt_completed || settlement_pending {
            if self.workflow_prompt_is_waiting_for_completion_output(session_id, provider_run_id)? {
                if !crate::transport::flow_control::prompt_output_quiet_after_response(
                    self.app,
                    provider_run_id,
                    WORKFLOW_MISSING_OUTPUT_SETTLE_QUIET_FOR,
                ) {
                    self.note_prompt_settlement_requested(provider_run_id);
                    return Ok(());
                }
                self.fail_for_missing_workflow_output(
                    session_id,
                    provider_run_id,
                    "provider completed workflow turn without a validated workflow output",
                )?;
                return Ok(());
            }
            self.settle_prompt_by_status(session_id, provider_run_id)?;
        }
        Ok(())
    }

    pub(crate) fn settle_pty_if_quiet(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let provider_run = self.provider_store.get_run(provider_run_id)?;
        if crate::provider::provider_run_uses_claude_native_bridge(&provider_run) {
            return Ok(());
        }
        if !crate::transport::flow_control::prompt_output_quiet_after_response(
            self.app,
            provider_run_id,
            PTY_PROMPT_SETTLE_QUIET_FOR,
        ) {
            return Ok(());
        }
        let Some(prompt) = self.active_prompt_for_settlement(session_id, provider_run_id)? else {
            return Ok(());
        };
        if prompt.status() != PromptStatus::Cancelling && prompt.delivery_pending() {
            return Ok(());
        }
        if prompt.status() != PromptStatus::Cancelling
            && self.workflow_prompt_is_waiting_for_completion_output(session_id, provider_run_id)?
        {
            return Ok(());
        }
        self.settle_prompt_by_status(session_id, provider_run_id)
    }

    pub(crate) fn fail_for_terminal_failure(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
    ) -> Result<(), DaemonError> {
        self.fail_for_terminal_failure_if_matches(session_id, provider_run_id, None, None, message)
            .map(|_| ())
    }

    pub(crate) fn fail_for_terminal_failure_if_matches(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        expected_prompt_id: Option<&str>,
        provider_termination: Option<crate::provider::ProviderRunTermination>,
        message: &str,
    ) -> Result<bool, DaemonError> {
        let Some(prompt) = self.active_prompt_for_settlement(session_id, provider_run_id)? else {
            if expected_prompt_id.is_none() {
                self.clear_prompt_activity(provider_run_id);
            }
            return Ok(false);
        };
        if expected_prompt_id.is_some_and(|expected_prompt_id| prompt.id() != expected_prompt_id) {
            return Ok(false);
        };
        if expected_prompt_id.is_some() {
            let diagnosed = self
                .provider_store
                .record_terminal_diagnostic(provider_run_id, message.to_string())?;
            self.app.update_provider_run_projection(diagnosed);
        }
        if let Ok(outcome) = self
            .provider_store
            .terminate_run_provider_only(session_id, provider_run_id)
        {
            let _ = super::provider_liveness::clear_active_provider_run_session_pointer(
                self.app,
                session_id,
                outcome.run().id(),
            );
            self.app.update_provider_run_projection(outcome.into_run());
        }
        let agent_id = self.provider_run_agent_id(provider_run_id)?;
        if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        {
            let failure = crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::ProviderFailure,
                workflow_node_run_id,
                Vec::new(),
                message,
            );
            let _ = self.app.sessions_mut().record_workflow_failure_event(
                session_id,
                workflow_run_id,
                failure,
            );
            let workflow_run = self.app.sessions_mut().fail_workflow_node_run(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            self.app.record_notice(
                session_id,
                Some(provider_run_id),
                self.app.attachments.list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{}` failed after provider turn failure: {}",
                    workflow_run.id(),
                    message
                ),
            );
            let _ =
                crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id);
        }
        let _ = self.app.fail_active_prompt_with_termination(
            session_id,
            &agent_id,
            Some(provider_run_id),
            provider_termination,
        )?;
        self.clear_active_turn(provider_run_id);
        Ok(true)
    }

    fn fail_for_missing_workflow_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
    ) -> Result<(), DaemonError> {
        let Some(prompt) = self.active_prompt_for_settlement(session_id, provider_run_id)? else {
            self.clear_prompt_activity(provider_run_id);
            return Ok(());
        };
        if prompt.delivery_pending() {
            return Ok(());
        }
        let agent_id = self.provider_run_agent_id(provider_run_id)?;
        if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        {
            let failure = crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::MissingStructuredOutput,
                workflow_node_run_id,
                Vec::new(),
                message,
            );
            let _ = self.app.sessions_mut().record_workflow_failure_event(
                session_id,
                workflow_run_id,
                failure,
            );
            let workflow_run = self.app.sessions_mut().fail_workflow_node_run(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            let _ = self.app.release_workflow_node_workspace_claim(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            );
            self.app.record_notice(
                session_id,
                Some(provider_run_id),
                self.app.attachments.list_session_attachment_ids(session_id),
                format!("Workflow run `{}` failed: {message}.", workflow_run.id()),
            );
            let _ =
                crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id);
        }
        let _ = self
            .app
            .fail_active_prompt(session_id, &agent_id, Some(provider_run_id))?;
        self.clear_active_turn(provider_run_id);
        Ok(())
    }

    fn workflow_prompt_is_waiting_for_completion_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(prompt) = self.active_prompt_for_settlement(session_id, provider_run_id)? else {
            return Ok(false);
        };
        if prompt.workflow_run_id().is_none() || prompt.workflow_node_run_id().is_none() {
            return Ok(false);
        }
        Ok(
            !crate::app::workflow_runtime::workflow_prompt_has_completion_output_from_runtime(
                self.app,
                session_id,
                &prompt,
                Some(provider_run_id),
            ),
        )
    }

    fn note_prompt_settlement_requested(&self, provider_run_id: &str) {
        self.active_turns.mark_settling(provider_run_id);
        self.prompt_activity
            .write()
            .entry(provider_run_id.to_string())
            .and_modify(|state| {
                state.request_settlement();
            })
            .or_insert(ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: false,
                settlement_requested: true,
                active_tool_ids: std::collections::BTreeSet::new(),
            });
    }

    fn settle_prompt_by_status(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(prompt) = self.active_prompt_for_settlement(session_id, provider_run_id)? else {
            self.clear_prompt_activity(provider_run_id);
            return Ok(());
        };
        if prompt.status() == PromptStatus::Dispatching
            || prompt.durable_delivery_phase()
                == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        {
            return Ok(());
        }
        let agent_id = self.provider_run_agent_id(provider_run_id)?;
        if prompt.status() == PromptStatus::Cancelling {
            let _ = self.app.finalize_active_prompt_cancellation(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?;
        } else {
            let _ =
                self.app
                    .complete_active_prompt(session_id, &agent_id, Some(provider_run_id))?;
        }
        self.clear_active_turn(provider_run_id);
        Ok(())
    }

    fn active_prompt_for_settlement(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let agent_id = self.provider_run_agent_id(provider_run_id)?;
        if let Some(prompt) = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
        {
            if prompt.is_external() {
                return Ok(None);
            }
            return Ok(Some(prompt));
        }
        let active = self
            .agent_runtime_projection
            .get(&agent_id)
            .filter(|projection| projection.session_id == session_id)
            .and_then(|projection| projection.active_prompt);
        if active.as_ref().is_some_and(|prompt| prompt.is_external()) {
            return Ok(None);
        }
        Ok(active)
    }

    fn provider_run_agent_id(&self, provider_run_id: &str) -> Result<String, DaemonError> {
        self.provider_store
            .get_run(provider_run_id)?
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })
    }

    fn clear_prompt_activity(&mut self, provider_run_id: &str) {
        crate::transport::flow_control::clear_prompt_activity(self.app, provider_run_id);
    }

    fn clear_active_turn(&self, provider_run_id: &str) {
        self.active_turns.clear(provider_run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_failure_ends_provider_run_before_settling_prompt() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-terminal-failure-settlement",
                "worktree-terminal-failure-settlement",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-terminal-failure-settlement",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "codex",
            "codex",
            "default",
            "gpt-5.3-codex-spark",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-terminal-failure-settlement",
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
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");
        app.update_provider_run_projection(run.clone());
        let prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "trigger usage limit\n",
            PromptStatus::Queued,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should start");

        let provider_store = app.providers.clone();
        let active_turns = app.active_turns.clone();
        let prompt_activity = app.prompt_activity.clone();
        let agent_runtime_projection = app.agent_runtime_projection_store();
        ProviderOutputPromptSettlement::new(
            &mut app,
            provider_store.clone(),
            active_turns,
            prompt_activity,
            agent_runtime_projection,
        )
        .fail_for_terminal_failure(session.id(), run.id(), "usage limit reached")
        .expect("terminal failure should settle");

        assert_eq!(
            provider_store
                .get_run(run.id())
                .expect("provider run should remain recorded")
                .state(),
            crate::provider::ProviderRunState::Ended
        );
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none());
        assert_eq!(
            app.sessions()
                .get_session(session.id())
                .expect("session should exist")
                .active_provider_run_id(),
            None
        );
        assert_eq!(
            app.completed_git_turn_snapshot_store()
                .latest_projection_for_agent(session.id(), agent.id())
                .expect("failed prompt should remain visible")
                .settlement_status,
            crate::git_observer::CompletedTurnSettlementStatus::Failed
        );
    }

    #[test]
    fn terminal_failure_does_not_settle_a_replacement_prompt_for_a_stale_poll() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-stale-poll-settlement",
                "worktree-stale-poll-settlement",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-stale-poll-settlement",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "codex",
            "codex",
            "default",
            "gpt-5.6",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-stale-poll-settlement",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-stale-poll-settlement".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test-stale-poll-settlement".to_string()),
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");
        app.update_provider_run_projection(run.clone());

        let old_prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "original prompt",
            PromptStatus::Queued,
        );
        let old_prompt_id = match app
            .prompt_owner_submit_prepared_prompt(session.id(), old_prompt, false)
            .expect("original prompt should start")
        {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("original prompt should start immediately")
            }
        };
        app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
            .expect("original prompt should be completed before replacement");

        let replacement = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "replacement prompt",
            PromptStatus::Queued,
        );
        let replacement_id = match app
            .prompt_owner_submit_prepared_prompt(session.id(), replacement, false)
            .expect("replacement prompt should start")
        {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("replacement prompt should start immediately")
            }
        };
        let history_before = app
            .operational_history_store()
            .load_session_events(session.id(), Some(agent.id()))
            .expect("operational history should load");

        let provider_store = app.providers.clone();
        let active_turns = app.active_turns.clone();
        let prompt_activity = app.prompt_activity.clone();
        let agent_runtime_projection = app.agent_runtime_projection_store();
        let settled = ProviderOutputPromptSettlement::new(
            &mut app,
            provider_store.clone(),
            active_turns,
            prompt_activity,
            agent_runtime_projection,
        )
        .fail_for_terminal_failure_if_matches(
            session.id(),
            run.id(),
            Some(&old_prompt_id),
            Some(
                crate::provider::ProviderRunTermination::explicit_provider_error(
                    "late oversized poll",
                    crate::session::unix_epoch_ms(),
                ),
            ),
            "late oversized poll",
        )
        .expect("stale terminal failure should be ignored");
        assert!(!settled);

        assert_eq!(
            app.prompt_owner_active_prompt_for_agent(session.id(), agent.id())
                .expect("active prompt should load")
                .expect("replacement prompt should remain active")
                .id(),
            replacement_id
        );
        let run_after = provider_store
            .get_run(run.id())
            .expect("provider run should remain inspectable");
        assert_eq!(
            run_after.state(),
            crate::provider::ProviderRunState::Running
        );
        assert_eq!(run_after.terminal_diagnostic(), None);
        let history_after = app
            .operational_history_store()
            .load_session_events(session.id(), Some(agent.id()))
            .expect("operational history should load after stale failure");
        assert_eq!(history_after, history_before);
    }

    #[test]
    fn claude_native_composer_quiet_does_not_settle_the_active_prompt() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-native-claude-settlement",
                "worktree-native-claude-settlement",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-native-claude-settlement",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
        .with_agent_id(agent.id())
        .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-native-claude-settlement",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "claude:claude:claude-sonnet-4-6".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");
        let prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "prompt waiting in the Claude composer\n",
            PromptStatus::Queued,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should start");
        crate::transport::flow_control::note_prompt_started(&mut app, run.id());
        crate::transport::flow_control::note_prompt_response_content(&mut app, run.id());
        app.prompt_activity
            .write()
            .get_mut(run.id())
            .expect("prompt activity should exist")
            .last_output_at = Some(Instant::now() - Duration::from_millis(100));

        let provider_store = app.providers.clone();
        let active_turns = app.active_turns.clone();
        let prompt_activity = app.prompt_activity.clone();
        let agent_runtime_projection = app.agent_runtime_projection_store();
        ProviderOutputPromptSettlement::new(
            &mut app,
            provider_store,
            active_turns,
            prompt_activity,
            agent_runtime_projection,
        )
        .settle_pty_if_quiet(session.id(), run.id())
        .expect("quiet settlement should succeed");

        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some());
    }

    #[test]
    fn settlement_prefers_authoritative_prompt_owner_over_stale_projection() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-stale-settlement-projection",
                "worktree-stale-settlement-projection",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-stale-settlement-projection",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "codex",
            "codex",
            "default",
            "gpt-5.3-codex-spark",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-stale-settlement-projection",
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
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");

        let first = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "first prompt\n",
            PromptStatus::Queued,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), first, false)
            .expect("first prompt should start");
        let stale_prompt = app
            .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("first active prompt should load")
            .expect("first prompt should be active");
        app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
            .expect("first prompt should complete");
        let second = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "second prompt\n",
            PromptStatus::Queued,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), second, false)
            .expect("second prompt should start");
        let authoritative_prompt = app
            .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("second active prompt should load")
            .expect("second prompt should be active");

        let agent_runtime_projection = app.agent_runtime_projection_store();
        agent_runtime_projection.update_agent_prompt_state(
            session.id(),
            agent.id(),
            Some(stale_prompt.clone()),
            None,
            0,
        );
        let provider_store = app.providers.clone();
        let active_turns = app.active_turns.clone();
        let prompt_activity = app.prompt_activity.clone();
        let selected = ProviderOutputPromptSettlement::new(
            &mut app,
            provider_store,
            active_turns,
            prompt_activity,
            agent_runtime_projection,
        )
        .active_prompt_for_settlement(session.id(), run.id())
        .expect("active prompt selection should succeed")
        .expect("an active prompt should be selected");

        assert_eq!(selected.id(), authoritative_prompt.id());
        assert_ne!(selected.id(), stale_prompt.id());
    }

    #[test]
    fn settlement_projection_order_local_completion_publishes_bound_idle_snapshot_after_completion_event(
    ) {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-settlement-projection-order",
                "worktree-settlement-projection-order",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-settlement-projection-order",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "test-model",
        )
        .with_agent_id(agent.id());
        let mut run = crate::provider::RuntimeProviderRun::new(
            "provider-run-settlement-projection-order",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "test-dev-stub-settlement-projection-order".to_string(),
                pty_target: Some("test-dev-stub-settlement-projection-order".to_string()),
                pty_program: Some("/bin/sh".to_string()),
                pty_args: vec![
                    "-lc".to_string(),
                    "printf '%s\\n' 'legacy response'; sleep 5".to_string(),
                ],
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.pty
            .spawn_for_run(&run)
            .expect("legacy provider PTY should start");
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .expect("active provider run should be set");
        app.update_provider_run_projection(run.clone());
        assert!(
            !crate::provider::provider_run_uses_structured_prompt_io(&run),
            "this fixture must exercise the legacy PTY callback"
        );
        assert!(
            agent.remote_execution().is_none(),
            "the callback must take the local completion path"
        );

        let prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "finish the legacy settlement turn",
            PromptStatus::Queued,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should start");
        let prompt_id = app
            .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .expect("prompt should be active")
            .id()
            .to_string();
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(run.id().to_string()),
            None,
        )
        .expect("prompt should bind to the local provider run");
        crate::transport::flow_control::note_prompt_started(&mut app, run.id());

        let initial_snapshot =
            crate::runtime::projection::SessionSnapshotProjection::from_daemon_app(
                &mut app,
                session.id(),
                0,
            )
            .expect("public pre-output snapshot should be available");
        let initial_activity = initial_snapshot
            .agent_activity
            .get(agent.id())
            .expect("agent activity should be present before output");
        assert_eq!(
            initial_activity.status,
            crate::runtime::projection::AgentRuntimeStatus::Working
        );
        assert!(initial_activity.busy);
        assert_eq!(initial_activity.active_prompt_count, 1);
        assert!(initial_activity.active_turn.is_some());

        let (first_snapshot, first_records) = loop {
            let result = crate::runtime_transport::watch_subscription_state(
                &mut app,
                session.id(),
                attachment.id(),
                true,
                Some(initial_snapshot.clone()),
                0,
            );
            let crate::runtime_transport::WatchResult::Ok {
                records,
                completions,
                snapshot,
                ..
            } = result
            else {
                panic!("legacy provider subscription should remain available");
            };
            if !records.is_empty() {
                assert!(
                    completions.is_empty(),
                    "the public completion event must not precede the first provider output"
                );
                break (
                    snapshot
                        .as_ref()
                        .clone()
                        .expect("first provider output should carry a public snapshot"),
                    records,
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let first_activity = first_snapshot
            .agent_activity
            .get(agent.id())
            .expect("agent activity should remain present while output is active");
        assert_eq!(
            first_activity.status,
            crate::runtime::projection::AgentRuntimeStatus::Working,
            "a public snapshot cannot become idle while the legacy callback still has output"
        );
        assert!(first_activity.busy);
        assert_eq!(first_activity.active_prompt_count, 1);
        assert!(first_activity.active_turn.is_some());
        assert!(
            first_records
                .iter()
                .any(|record| record.provider_run_id == run.id()),
            "provider output must remain bound to the active local run"
        );
        let projection_before_settlement = app.session_state_projection_store().change_sequence();

        std::thread::sleep(Duration::from_millis(75));
        let result = crate::runtime_transport::watch_subscription_state(
            &mut app,
            session.id(),
            attachment.id(),
            true,
            Some(first_snapshot.clone()),
            0,
        );
        let crate::runtime_transport::WatchResult::Ok {
            records,
            completions,
            workflow_run_updates,
            snapshot,
            ..
        } = result
        else {
            panic!("settled provider subscription should remain available");
        };
        assert!(
            records.is_empty(),
            "the quiet callback should settle only after final output has drained"
        );
        assert!(
            workflow_run_updates.is_empty(),
            "the non-workflow settlement should not select a workflow-run event instead of the public activity/snapshot path"
        );
        assert_eq!(
            completions.len(),
            1,
            "the legacy callback must expose exactly one public completion"
        );
        assert_eq!(completions[0].provider_run_id, run.id());
        assert_eq!(
            completions[0].agent_id.as_deref(),
            Some(agent.id()),
            "the completion event must retain the local run's agent binding"
        );
        assert!(
            app.session_state_projection_store().change_sequence() > projection_before_settlement,
            "legacy outer settlement must invalidate the public session projection"
        );

        // The transport loop emits completion records before the activity delta. Requiring both
        // here makes an eventual idle snapshot insufficient: an idle event observed without this
        // completion is a premature projection.
        let settled_snapshot = snapshot
            .as_ref()
            .clone()
            .expect("settlement should carry the public idle snapshot");
        let settled_activity = settled_snapshot
            .agent_activity
            .get(agent.id())
            .expect("settled public snapshot should include the local agent");
        assert_eq!(
            settled_activity.status,
            crate::runtime::projection::AgentRuntimeStatus::Idle
        );
        assert!(!settled_activity.busy);
        assert_eq!(settled_activity.active_prompt_count, 0);
        assert!(settled_activity.active_turn.is_none());
        // The public subscription emits a narrow activity delta when the session permits one.
        // Otherwise its actual transport loop falls back to the snapshot payload above. Check
        // the same selection boundary without manufacturing a fallback event in this test.
        match crate::transport::kernel_protocol::agent_activity_changed_event(
            &settled_snapshot,
            Some(&first_snapshot),
        ) {
            Some(crate::transport::kernel_protocol::KernelEvent::AgentActivityChanged {
                agent_activity,
                ..
            }) => {
                let activity = agent_activity
                    .get(agent.id())
                    .expect("idle event should include the settled local agent");
                assert_eq!(
                    activity.status,
                    crate::runtime::projection::AgentRuntimeStatus::Idle
                );
                assert!(!activity.busy);
                assert_eq!(activity.active_prompt_count, 0);
                assert!(activity.active_turn.is_none());
            }
            Some(event) => panic!("unexpected public settlement event: {event:?}"),
            None => {
                assert!(
                    crate::transport::kernel_protocol::provider_run_changed_event(
                        &settled_snapshot,
                        Some(&first_snapshot),
                    )
                    .is_none(),
                    "the public transport must use the full snapshot when no activity delta applies"
                );
                assert!(
                    crate::transport::kernel_protocol::session_metadata_changed_event(
                        &settled_snapshot,
                        Some(&first_snapshot),
                    )
                    .is_none(),
                    "the public transport must use the full snapshot when no metadata delta applies"
                );
                assert!(
                    crate::transport::kernel_protocol::runtime_interactions_changed_event(
                        &settled_snapshot,
                        Some(&first_snapshot),
                    )
                    .is_none(),
                    "the public transport must use the full snapshot when no interaction delta applies"
                );
                assert!(
                    crate::transport::kernel_protocol::workflow_run_updated_events(
                        &settled_snapshot,
                        Some(&first_snapshot),
                    )
                    .is_empty()
                        && !crate::transport::kernel_protocol::workflow_run_only_changed(
                            &settled_snapshot,
                            Some(&first_snapshot),
                        ),
                    "the public transport must use the full snapshot when no narrow projection applies"
                );
                let fallback_activity = settled_snapshot
                    .agent_activity
                    .get(agent.id())
                    .expect("full snapshot fallback should include the settled local agent");
                assert_eq!(
                    fallback_activity.status,
                    crate::runtime::projection::AgentRuntimeStatus::Idle
                );
                assert!(!fallback_activity.busy);
                assert_eq!(fallback_activity.active_prompt_count, 0);
                assert!(fallback_activity.active_turn.is_none());
            }
        }

        app.pty
            .remove_process(run.id())
            .expect("legacy provider PTY should stop");
    }
}
