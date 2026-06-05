use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{
    unix_epoch_ms, PromptQueueItem, WorkflowDefinition, WorkflowEndpointDefinition,
    WorkflowFailureEvent, WorkflowFailureKind, WorkflowQueuedPrompt, WorkflowQueuedPromptSource,
    WorkflowRun, WorkflowWatchdogTickPlan,
};
use std::collections::BTreeSet;

struct WorkflowProgression;

impl WorkflowProgression {
    fn is_workflow_prompt_attachment(attachment_id: &str) -> bool {
        crate::scheduler::runtime::is_workflow_prompt_attachment(attachment_id)
    }

    fn ensure_provider_run(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        crate::scheduler::runtime::ensure_workflow_provider_run_for_agent(app, session_id, agent_id)
    }

    fn preflight_local_provider_runs(
        app: &mut DaemonApp,
        session_id: &str,
        workflow: &WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        let mut seen_agents = BTreeSet::new();
        for node in workflow.nodes() {
            if !seen_agents.insert(node.agent_id().to_string()) {
                continue;
            }
            let agent = app.agents().get_agent(node.agent_id())?;
            if agent.remote_execution().is_some() {
                continue;
            }
            Self::ensure_provider_run(app, session_id, node.agent_id())?;
        }
        Ok(())
    }

    fn validate_agents(
        app: &DaemonApp,
        session_id: &str,
        workflow: &WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::validate_workflow_agents(app, session_id, workflow)
    }

    fn schedule_entry_node(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_run: &WorkflowRun,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::schedule_workflow_run_entry_node(app, session_id, workflow_run)
    }

    fn on_prompt_started(
        app: &mut DaemonApp,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::on_workflow_prompt_started(app, session_id, prompt)
    }

    fn on_prompt_completed(
        app: &mut DaemonApp,
        session_id: &str,
        prompt: &PromptQueueItem,
        provider_run_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::on_workflow_prompt_completed(
            app,
            session_id,
            prompt,
            provider_run_id,
        )
    }

    fn on_prompt_cancelled(
        app: &mut DaemonApp,
        session_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        crate::scheduler::runtime::on_workflow_prompt_cancelled(app, session_id, prompt)
    }

    fn retry_blocked_claims(app: &mut DaemonApp) -> BTreeSet<String> {
        crate::scheduler::runtime::retry_blocked_workflow_claims(app)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowLaunchOutcome {
    Started {
        workflow_run: WorkflowRun,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
    },
    Enqueued {
        queued_prompt: WorkflowQueuedPrompt,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
    },
}

impl DaemonApp {
    pub fn enqueue_workflow_prompt_and_maybe_start(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
        queue_ref: Option<&str>,
        publication_invocation: Option<crate::session::WorkflowPublicationInvocationEnvelope>,
    ) -> Result<WorkflowLaunchOutcome, DaemonError> {
        let workflow = self
            .sessions()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.sessions().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        WorkflowProgression::validate_agents(self, session_id, &workflow)?;
        let queued_prompt = self
            .sessions_mut()
            .enqueue_workflow_prompt_with_publication_invocation(
                session_id,
                workflow.id(),
                endpoint.id(),
                prompt,
                queue_ref,
                WorkflowQueuedPromptSource::Manual,
                None,
                publication_invocation,
            )?;
        if self
            .sessions()
            .get_session(session_id)?
            .has_active_workflow_run()
        {
            return Ok(WorkflowLaunchOutcome::Enqueued {
                queued_prompt,
                workflow,
                endpoint,
            });
        }
        self.start_next_queued_workflow_prompt(session_id)?
            .ok_or_else(|| DaemonError::WorkflowLaunchRejected {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                endpoint_id: endpoint.id().to_string(),
                message: "workflow prompt was enqueued but no dispatchable queue item was found"
                    .to_string(),
            })
    }

    pub fn invoke_workflow_endpoint_and_schedule(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<(WorkflowRun, WorkflowDefinition, WorkflowEndpointDefinition), DaemonError> {
        match self.enqueue_workflow_prompt_and_maybe_start(
            session_id,
            workflow_ref,
            endpoint_ref,
            prompt,
            None,
            None,
        )? {
            WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            } => Ok((workflow_run, workflow, endpoint)),
            WorkflowLaunchOutcome::Enqueued {
                workflow, endpoint, ..
            } => Err(DaemonError::WorkflowLaunchRejected {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                endpoint_id: endpoint.id().to_string(),
                message: "workflow launch was queued instead of started".to_string(),
            }),
        }
    }

    pub fn start_next_queued_workflow_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<Option<WorkflowLaunchOutcome>, DaemonError> {
        loop {
            let Some(queued_prompt) = self
                .sessions_mut()
                .dequeue_next_workflow_prompt(session_id)?
            else {
                return Ok(None);
            };
            if let Some(watchdog_id) = queued_prompt.watchdog_id() {
                let _ = self
                    .sessions_mut()
                    .mark_workflow_watchdog_pending_started(session_id, watchdog_id);
            }
            let outcome = self.invoke_queued_workflow_prompt(session_id, queued_prompt.clone());
            match outcome {
                Ok(outcome) => return Ok(Some(outcome)),
                Err(error) => {
                    self.record_failed_queued_workflow_prompt(session_id, &queued_prompt, &error);
                }
            }
        }
    }

    fn record_failed_queued_workflow_prompt(
        &mut self,
        session_id: &str,
        queued_prompt: &WorkflowQueuedPrompt,
        error: &DaemonError,
    ) {
        if let Some(watchdog_id) = queued_prompt.watchdog_id() {
            let _ = self.sessions_mut().mark_workflow_watchdog_failed(
                session_id,
                watchdog_id,
                error.to_string(),
            );
        }
        self.record_notice(
            session_id,
            None,
            self.attachments().list_session_attachment_ids(session_id),
            format!(
                "Queued workflow prompt `{}` failed: {}",
                queued_prompt.id(),
                error
            ),
        );
    }

    fn invoke_queued_workflow_prompt(
        &mut self,
        session_id: &str,
        queued_prompt: WorkflowQueuedPrompt,
    ) -> Result<WorkflowLaunchOutcome, DaemonError> {
        let workflow = self
            .sessions()
            .resolve_workflow_ref(session_id, queued_prompt.workflow_id())?;
        let endpoint = self.sessions().resolve_workflow_endpoint_ref(
            session_id,
            queued_prompt.workflow_id(),
            queued_prompt.endpoint_id(),
        )?;
        WorkflowProgression::validate_agents(self, session_id, &workflow)?;
        WorkflowProgression::preflight_local_provider_runs(self, session_id, &workflow)?;
        let workflow_run = self
            .sessions_mut()
            .invoke_workflow_endpoint_with_publication_invocation(
                session_id,
                workflow.id(),
                endpoint.id(),
                queued_prompt.prompt().map(str::to_string),
                queued_prompt.publication_invocation().cloned(),
            )?;
        if let Err(error) =
            WorkflowProgression::schedule_entry_node(self, session_id, &workflow_run)
        {
            if let Some(node_run) = workflow_run.node_runs().first() {
                let _ = self.sessions_mut().record_workflow_failure_event(
                    session_id,
                    workflow_run.id(),
                    WorkflowFailureEvent::new(
                        WorkflowFailureKind::TransportFailure,
                        node_run.id(),
                        Vec::new(),
                        error.to_string(),
                    ),
                );
                let _ = self.sessions_mut().fail_workflow_node_run(
                    session_id,
                    workflow_run.id(),
                    node_run.id(),
                );
            }
            return Err(error);
        }
        let workflow_run = self
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        if let Some(watchdog_id) = queued_prompt.watchdog_id() {
            let _ = self.sessions_mut().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        Ok(WorkflowLaunchOutcome::Started {
            workflow_run,
            workflow,
            endpoint,
        })
    }
}

pub(crate) fn is_workflow_prompt_source(attachment_id: &str) -> bool {
    WorkflowProgression::is_workflow_prompt_attachment(attachment_id)
}

pub(crate) fn pump_workflow_watchdogs(app: &mut DaemonApp) {
    let plans = match app
        .sessions_mut()
        .collect_due_workflow_watchdog_invocations(unix_epoch_ms())
    {
        Ok(plans) => plans,
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.app",
                "workflow watchdog collection failed",
                serde_json::json!({ "error": error.to_string() }),
            );
            return;
        }
    };
    for plan in plans {
        match invoke_watchdog_workflow_launch(app, plan) {
            Ok(()) => {}
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.app",
                    "workflow watchdog invoke failed",
                    serde_json::json!({ "error": error.to_string() }),
                );
            }
        }
    }
}

fn invoke_watchdog_workflow_launch(
    app: &mut DaemonApp,
    plan: WorkflowWatchdogTickPlan,
) -> Result<(), DaemonError> {
    let queued_prompt = app.sessions_mut().enqueue_workflow_prompt(
        &plan.session_id,
        &plan.workflow_id,
        &plan.endpoint_id,
        Some(plan.invocation_prompt.clone()),
        None,
        WorkflowQueuedPromptSource::Watchdog,
        Some(plan.watchdog_id.clone()),
    )?;
    if app
        .sessions()
        .get_session(&plan.session_id)?
        .has_active_workflow_run()
    {
        return Ok(());
    }
    match app.invoke_queued_workflow_prompt(&plan.session_id, queued_prompt) {
        Ok(WorkflowLaunchOutcome::Started { .. }) => Ok(()),
        Ok(WorkflowLaunchOutcome::Enqueued { .. }) => Ok(()),
        Err(error) => {
            let _ = app.sessions_mut().mark_workflow_watchdog_failed(
                &plan.session_id,
                &plan.watchdog_id,
                error.to_string(),
            );
            Err(error)
        }
    }
}

pub(crate) fn start_workflow_prompt_from_runtime(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    WorkflowProgression::on_prompt_started(app, session_id, prompt)
}

pub(crate) fn complete_workflow_prompt_from_runtime(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
    provider_run_id: Option<&str>,
) -> Result<(), DaemonError> {
    WorkflowProgression::on_prompt_completed(app, session_id, prompt, provider_run_id)
}

pub(crate) fn workflow_prompt_has_completion_output_from_runtime(
    app: &DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
    provider_run_id: Option<&str>,
) -> bool {
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return true;
    };
    let Ok(session) = app.sessions().get_session(session_id) else {
        return false;
    };
    if session
        .workflow_run(workflow_run_id)
        .and_then(|workflow_run| {
            workflow_run
                .node_runs()
                .iter()
                .find(|node_run| node_run.id() == workflow_node_run_id)
        })
        .is_some_and(|node_run| node_run.has_valid_pending_final_output())
    {
        return true;
    }
    let Ok(history) = crate::app::KernelSessionReadService::new(app).session_history(session_id)
    else {
        return false;
    };
    crate::scheduler::runtime::build_workflow_completion_snapshot_from_history(
        &session,
        history,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        provider_run_id.unwrap_or_default(),
    )
    .and_then(|snapshot| snapshot.output().cloned())
    .is_some()
}

pub(crate) fn cancel_workflow_prompt_from_runtime(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    WorkflowProgression::on_prompt_cancelled(app, session_id, prompt)
}

pub(crate) fn ensure_workflow_provider_run_from_runtime(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> Result<String, DaemonError> {
    WorkflowProgression::ensure_provider_run(app, session_id, agent_id)
}

pub(crate) fn retry_blocked_workflow_claims_from_runtime(app: &mut DaemonApp) {
    for session_id in WorkflowProgression::retry_blocked_claims(app) {
        let _ = crate::app::KernelSessionReadService::new(app).session_snapshot(&session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_workflow_prompt_preserves_agent_runtime_context() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(session.id(), "codex"))
            .expect("workflow-capable agent should be created");
        app.agents
            .set_agent_runtime_profile(
                agent.id(),
                "default",
                None,
                None,
                crate::provider::ProviderResumeState::from_codex_thread_id("thread-1"),
            )
            .expect("agent runtime profile should be set");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("queued".to_string()))
            .expect("workflow should be created");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .expect("workflow node should be created");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("endpoint should be created");
        let queued = app
            .sessions_mut()
            .enqueue_workflow_prompt(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("queued prompt".to_string()),
                None,
                WorkflowQueuedPromptSource::Manual,
                None,
            )
            .expect("workflow prompt should be queued");

        let _ = app.invoke_queued_workflow_prompt(session.id(), queued);
        let updated = app
            .agents()
            .get_agent(agent.id())
            .expect("agent should exist");
        assert_eq!(
            updated.provider_resume_state().codex_thread_id(),
            Some("thread-1"),
            "queued workflow delivery must not flush provider runtime context"
        );
    }

    #[test]
    fn queued_workflow_scheduler_continues_after_invalid_candidate() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(session.id(), "codex"))
            .expect("workflow-capable agent should be created");
        let bad_workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("bad".to_string()))
            .expect("bad workflow should be created");
        let bad_node = app
            .sessions_mut()
            .add_workflow_node(session.id(), bad_workflow.id(), "missing-agent")
            .expect("bad workflow node should be created");
        let bad_endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                bad_workflow.id(),
                bad_node.id(),
                Some("entry".to_string()),
            )
            .expect("bad endpoint should be created");
        let good_workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("good".to_string()))
            .expect("good workflow should be created");
        let good_node = app
            .sessions_mut()
            .add_workflow_node(session.id(), good_workflow.id(), agent.id())
            .expect("good workflow node should be created");
        let good_endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                good_workflow.id(),
                good_node.id(),
                Some("entry".to_string()),
            )
            .expect("good endpoint should be created");
        let high = app
            .sessions_mut()
            .create_workflow_prompt_queue(session.id(), bad_workflow.id(), "high".to_string(), 10)
            .expect("high queue should be created");
        let low = app
            .sessions_mut()
            .create_workflow_prompt_queue(session.id(), good_workflow.id(), "low".to_string(), 1)
            .expect("low queue should be created");
        app.sessions_mut()
            .enqueue_workflow_prompt(
                session.id(),
                bad_workflow.id(),
                bad_endpoint.id(),
                Some("bad".to_string()),
                Some(high.id()),
                WorkflowQueuedPromptSource::Manual,
                None,
            )
            .expect("bad prompt should queue");
        app.sessions_mut()
            .enqueue_workflow_prompt(
                session.id(),
                good_workflow.id(),
                good_endpoint.id(),
                Some("good".to_string()),
                Some(low.id()),
                WorkflowQueuedPromptSource::Manual,
                None,
            )
            .expect("good prompt should queue");

        let outcome = app
            .start_next_queued_workflow_prompt(session.id())
            .expect("scheduler should continue past invalid queued prompt")
            .expect("good queued prompt should start");

        match outcome {
            WorkflowLaunchOutcome::Started { workflow, .. } => {
                assert_eq!(workflow.id(), good_workflow.id());
            }
            WorkflowLaunchOutcome::Enqueued { .. } => panic!("expected queued prompt to start"),
        }
    }
}
