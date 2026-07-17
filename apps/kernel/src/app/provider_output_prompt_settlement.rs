use std::time::{Duration, Instant};

use crate::app::{ActivePromptState, ActiveTurnStore, DaemonApp, PromptActivityStore};
use crate::error::DaemonError;
use crate::provider::ProviderProcessServiceStore;
use crate::runtime::projection::AgentRuntimeProjectionStore;
use crate::session::{PromptQueueItem, PromptStatus};

const PTY_PROMPT_SETTLE_QUIET_FOR: Duration = Duration::from_millis(50);

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
        let Some(active_prompt_status) = self
            .active_prompt_for_settlement(session_id, provider_run_id)?
            .map(|prompt| prompt.status())
        else {
            return Ok(());
        };
        let completion_recorded =
            crate::transport::flow_control::prompt_completion_recorded(self.app, provider_run_id);
        let mut settlement_pending =
            crate::transport::flow_control::prompt_completion_settlement_pending(
                self.app,
                provider_run_id,
            );
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
        let Some(prompt) = self.active_prompt_for_settlement(session_id, provider_run_id)? else {
            self.clear_prompt_activity(provider_run_id);
            return Ok(());
        };
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
        let _ = self
            .app
            .complete_active_prompt(session_id, &agent_id, Some(provider_run_id))?;
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
                state.last_output_at = Some(Instant::now());
                state.saw_response_content = true;
                state.settlement_requested = true;
            })
            .or_insert(ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: false,
                settlement_requested: true,
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
            .agent_runtime_projection
            .get(&agent_id)
            .filter(|projection| projection.session_id == session_id)
            .and_then(|projection| projection.active_prompt)
        {
            if prompt.is_external() {
                return Ok(None);
            }
            return Ok(Some(prompt));
        }
        let active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?;
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
}
