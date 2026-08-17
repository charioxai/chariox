use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{
    PromptQueueItem, WorkflowDefinition, WorkflowEndpointDefinition, WorkflowFailureEvent,
    WorkflowFailureKind, WorkflowQueuedPrompt, WorkflowQueuedPromptSource, WorkflowRun,
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
        event_reply_enabled: bool,
        event_context_enabled: bool,
        event_actions_enabled: bool,
    ) -> Result<String, DaemonError> {
        crate::scheduler::runtime::ensure_workflow_provider_run_for_agent_with_event_reply(
            app,
            session_id,
            agent_id,
            event_reply_enabled,
            event_context_enabled,
            event_actions_enabled,
        )
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
            Self::ensure_provider_run(app, session_id, node.agent_id(), false, false, false)?;
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
        workflow_run: Box<WorkflowRun>,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
    },
    Enqueued {
        queued_prompt: Box<WorkflowQueuedPrompt>,
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
                queued_prompt: Box::new(queued_prompt),
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
            } => Ok((*workflow_run, workflow, endpoint)),
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
            .invoke_queued_workflow_endpoint(session_id, &queued_prompt)?;
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
            workflow_run: Box::new(workflow_run),
            workflow,
            endpoint,
        })
    }
}

pub(crate) fn is_workflow_prompt_source(attachment_id: &str) -> bool {
    WorkflowProgression::is_workflow_prompt_attachment(attachment_id)
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
    WorkflowProgression::ensure_provider_run(app, session_id, agent_id, false, false, false)
}

pub(crate) fn ensure_workflow_provider_run_for_prompt_from_runtime(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    prompt: &PromptQueueItem,
) -> Result<String, DaemonError> {
    let (event_reply_enabled, event_context_enabled, event_actions_enabled) =
        workflow_event_capabilities_for_prompt_from_runtime(app, session_id, prompt)?;
    ensure_workflow_provider_run_with_event_capabilities_from_runtime(
        app,
        session_id,
        agent_id,
        event_reply_enabled,
        event_context_enabled,
        event_actions_enabled,
    )
}

pub(crate) fn ensure_workflow_provider_run_with_event_capabilities_from_runtime(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    event_reply_enabled: bool,
    event_context_enabled: bool,
    event_actions_enabled: bool,
) -> Result<String, DaemonError> {
    WorkflowProgression::ensure_provider_run(
        app,
        session_id,
        agent_id,
        event_reply_enabled,
        event_context_enabled,
        event_actions_enabled,
    )
}

pub(crate) fn workflow_event_capabilities_for_prompt_from_runtime(
    app: &DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(bool, bool, bool), DaemonError> {
    let Some(workflow_run_id) = prompt.workflow_run_id() else {
        return Ok((false, false, false));
    };
    let workflow_run = app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)?;
    let Some(invocation) = workflow_run.publication_invocation() else {
        return Ok((false, false, false));
    };
    if invocation.transport != "event" {
        return Ok((false, false, false));
    }
    let Some(binding_id) = invocation.hook_id.as_deref() else {
        return Ok((false, false, false));
    };
    let session = app.sessions().get_session(session_id)?;
    let Some(binding) = session.workflow_event_binding(binding_id) else {
        return Ok((false, false, false));
    };
    let reply_enabled = matches!(binding.reply_mode.as_deref(), Some("thread" | "channel"));
    let context_enabled = binding.active()
        && invocation
            .input
            .get("reply_context")
            .is_some_and(|context| !context.is_null());
    Ok((
        reply_enabled,
        context_enabled,
        !binding.action_ids.is_empty(),
    ))
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
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "codex")
                    .with_account_profile("profile-b"),
            )
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
        let run_id = ensure_workflow_provider_run_from_runtime(&mut app, session.id(), agent.id())
            .expect("workflow provider ensure should launch a provider run");
        let run = app
            .providers()
            .get_run(&run_id)
            .expect("workflow prompt should launch a provider run");
        assert_eq!(run.account_profile(), "profile-b");
    }

    #[test]
    fn queued_event_prompt_derives_reply_and_context_capabilities_independently() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-event",
                "worktree-event",
            ))
            .expect("session should be created");
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(session.id(), "codex"))
            .expect("agent should be created");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("event-reply".to_string()))
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
        let publication = app
            .sessions_mut()
            .create_workflow_publication_idempotent(
                session.id(),
                workflow.id(),
                endpoint.id(),
                None,
                None,
                Some("default".to_string()),
                Some("event".to_string()),
                Some(crate::session::WORKFLOW_PUBLICATION_KIND_EVENT_BASED.to_string()),
                None,
                Vec::new(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                vec![agent.clone()],
                "local".to_string(),
            )
            .expect("event publication should be created");
        let binding = app
            .sessions_mut()
            .create_workflow_event_binding(
                session.id(),
                publication.id(),
                "dev.chariox.github".to_string(),
                "1".to_string(),
                "manifest".to_string(),
                "connection".to_string(),
                "repo".to_string(),
                "pull_request.opened".to_string(),
                1,
                serde_json::json!({}),
                None,
                Some("default".to_string()),
                Some("disabled".to_string()),
                vec!["slack.message.permalink".to_string()],
            )
            .expect("event binding should be created");
        let invocation = crate::session::WorkflowPublicationInvocationEnvelope {
            publication_id: publication.id().to_string(),
            hook_id: Some(binding.id.clone()),
            invocation_id: "event-1".to_string(),
            transport: "event".to_string(),
            endpoint_id: endpoint.id().to_string(),
            queue_ref: Some("default".to_string()),
            input: serde_json::json!({
                "prompt": "review",
                "reply_context": {
                    "provider": "slack",
                    "team_id": "T123",
                    "channel_id": "C123",
                    "message_ts": "123.456"
                }
            }),
            artifacts: Vec::new(),
            mode: None,
            caller: serde_json::json!({ "type": "event" }),
        };
        let (_queued, claimed) = app
            .sessions_mut()
            .enqueue_workflow_prompt_and_maybe_create_run(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("review".to_string()),
                Some("default"),
                crate::session::WorkflowQueuedPromptSource::Event,
                None,
                Some(invocation),
            )
            .expect("event prompt should be claimed");
        let (_claimed_prompt, run, _workflow, _endpoint) =
            claimed.expect("event prompt should create a workflow run");
        let node_run = run
            .node_runs()
            .first()
            .expect("event workflow should create a node run");
        let prompt = PromptQueueItem::new(
            "event-prompt",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(run.id()),
            agent.id(),
            "review",
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(run.id(), node_run.id());
        let capabilities =
            workflow_event_capabilities_for_prompt_from_runtime(&app, session.id(), &prompt)
                .expect("binding capabilities should resolve");
        assert_eq!(capabilities, (false, true, true));
        let ordinary_provider_run_id = ensure_workflow_provider_run_from_runtime(
            &mut app,
            session.id(),
            agent.id(),
        )
        .expect("ordinary provider run should launch before the event run");
        let ordinary_provider_run = app
            .providers()
            .get_run(&ordinary_provider_run_id)
            .expect("ordinary provider run should resolve");
        assert!(!ordinary_provider_run.workflow_event_actions_enabled());
        let provider_run_id = ensure_workflow_provider_run_for_prompt_from_runtime(
            &mut app,
            session.id(),
            agent.id(),
            &prompt,
        )
        .expect("event provider should launch");
        assert_ne!(provider_run_id, ordinary_provider_run_id);
        let provider_run = app
            .providers()
            .get_run(&provider_run_id)
            .expect("event provider should resolve");
        assert!(!provider_run.workflow_event_reply_enabled());
        assert!(provider_run.workflow_event_context_enabled());
        assert!(provider_run.workflow_event_actions_enabled());
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

    #[test]
    fn app_workflow_completion_persists_terminal_session_snapshot() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-durable-workflow-completion",
                "worktree-durable-workflow-completion",
            ))
            .expect("session should be created");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("durable-completion".to_string()))
            .expect("workflow should be created");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .expect("workflow node should be created");
        app.sessions_mut()
            .set_workflow_node_can_complete_run(session.id(), workflow.id(), node.id(), true)
            .expect("workflow node should be allowed to complete the run");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("workflow endpoint should be created");
        let workflow_run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("finish durably".to_string()),
            )
            .expect("workflow run should be created");
        let node_run_id = workflow_run.node_runs()[0].id().to_string();
        let delivery_token = format!("workflow-ack:{node_run_id}");
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                workflow_run.id(),
                &node_run_id,
                delivery_token.clone(),
                "complete the workflow".to_string(),
                None,
                None,
            )
            .expect("workflow turn should be prepared");
        app.sessions_mut()
            .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
            .expect("workflow node run should start");
        app.sessions_mut()
            .mark_workflow_turn_dispatched(session.id(), workflow_run.id(), &node_run_id)
            .expect("workflow turn should dispatch");
        app.sessions_mut()
            .ack_workflow_turn(
                session.id(),
                workflow_run.id(),
                &node_run_id,
                &delivery_token,
            )
            .expect("workflow turn should acknowledge");
        app.sessions_mut()
            .submit_workflow_run_final_output(
                session.id(),
                workflow_run.id(),
                &node_run_id,
                crate::session::WorkflowOutputPayload::new(r#"{"summary":"done"}"#, Vec::new()),
                true,
                None,
            )
            .expect("final output should submit");
        let prompt = crate::session::PromptQueueItem::new(
            "prompt-durable-workflow-completion",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            agent.id(),
            "complete the workflow",
            crate::session::PromptStatus::Completed,
        )
        .with_workflow_context(workflow_run.id(), &node_run_id);

        complete_workflow_prompt_from_runtime(&mut app, session.id(), &prompt, None)
            .expect("workflow prompt should complete");

        let durable_session = app
            .durable_state_store()
            .load_events_by_kind("session.updated")
            .expect("durable session events should load")
            .into_iter()
            .rev()
            .find(|event| {
                event.subject_id.as_deref() == Some(session.id())
                    && event
                        .payload
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        == Some("workflow_prompt_completed")
            })
            .and_then(|event| event.payload.get("session").cloned())
            .map(serde_json::from_value::<crate::session::RuntimeSession>)
            .transpose()
            .expect("durable workflow session should decode")
            .expect("workflow completion should persist its session");
        let durable_run = durable_session
            .workflow_run(workflow_run.id())
            .expect("durable workflow run should exist");
        assert_eq!(
            durable_run.status(),
            crate::session::WorkflowRunStatus::Completed,
            "a kernel restart must not restore the completed workflow as running"
        );
        assert_eq!(
            durable_run.node_runs()[0].status(),
            crate::session::WorkflowNodeRunStatus::Completed
        );
        assert!(durable_run.active_node_run_id().is_none());
    }
}
