//! MP-08 / MP-09 / MP-10 / MP-11: one output reminder per workflow node attempt.

use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::runtime::prompt_state::PromptStateOwner;
use crate::session::{
    DurablePromptDeliveryPhase, PromptQueueItem, PromptStatus, SessionStateStore,
    WorkflowNodeRunStatus, WorkflowRunStatus,
};

pub(crate) use crate::session::WORKFLOW_OUTPUT_FOLLOW_UP_PREFIX as FOLLOW_UP_PREFIX;
pub(crate) const EXHAUSTED_REASON: &str =
    "Workflow output is missing after one follow-up. Check the output instructions and start a new workflow run.";
const REMINDER: &str = "Your turn ended without the required structured output. If helpers or background tasks are still running, wait for them, then give the final output now, in the required schema.";

/// The prompt identity is the durable retry budget. Retain the node envelope and
/// provider session so background helpers and their pending output remain usable.
pub(crate) fn prepare_follow_up(
    sessions: &SessionStateStore,
    prompts: &PromptStateOwner,
    durable: &DurableKernelStateStore,
    session_id: &str,
    previous: &PromptQueueItem,
) -> Result<Option<PromptQueueItem>, DaemonError> {
    let (Some(run_id), Some(node_id)) =
        (previous.workflow_run_id(), previous.workflow_node_run_id())
    else {
        return Ok(None);
    };
    if previous.status() != PromptStatus::Running || previous.id().starts_with(FOLLOW_UP_PREFIX) {
        return Ok(None);
    }
    durable.with_workflow_runtime_transition_lock(|| {
        let mut sessions = sessions.write();
        let session = sessions.get_session(session_id)?;
        let Some(run) = session.workflow_run(run_id).filter(|run| {
            matches!(
                run.status(),
                WorkflowRunStatus::Running | WorkflowRunStatus::Waiting
            )
        }) else {
            return Ok(None);
        };
        if !run.node_runs().iter().any(|node| {
            node.id() == node_id
                && matches!(
                    node.status(),
                    WorkflowNodeRunStatus::Running | WorkflowNodeRunStatus::Waiting
                )
                && !node.has_valid_pending_final_output()
        }) {
            return Ok(None);
        }
        // An explicit kernel-owned wait/block is already supervised; ending the
        // provider turn does not authorize a reminder to resume that task.
        if durable
            .agent_tasks(Some(session_id), Some(previous.target_agent_id()))?
            .iter()
            .any(|task| {
                task.prompt_id == previous.id()
                    && (task.state
                        != crate::durable_state::agent_lifecycle::ExecutionState::Working
                        || task.wait.is_some()
                        || task.pending_prompt_id.is_some())
            })
        {
            return Ok(None);
        }
        let id = format!("{FOLLOW_UP_PREFIX}{}", previous.id());
        let mut next = PromptQueueItem::new(
            &id,
            previous.source_attachment_id(),
            previous.target_agent_id(),
            "Continue the workflow turn.",
            PromptStatus::Running,
        )
        .with_workflow_context(run_id, node_id)
        .with_hidden_system_context(format!(
            "{}\n\n{REMINDER}",
            previous.hidden_system_context()
        ))
        .with_durable_operation(&id, id.clone());
        next.set_durable_delivery(DurablePromptDeliveryPhase::Accepted, None, None);
        let agent_id = previous.target_agent_id();
        if !prompts.replace_active_prompt_if_matches(&session, agent_id, previous, next.clone()) {
            return Ok(None);
        }
        let (active, queued) = prompts.state_parts(&session, agent_id);
        let persisted = sessions
            .mirror_agent_prompt_state(session_id, agent_id, active, queued.clone())
            .and_then(|session| {
                durable.persist_workflow_prompt_transition(
                    &session,
                    agent_id,
                    "workflow_output_follow_up",
                )
            });
        if let Err(error) = persisted {
            prompts.replace_active_prompt_if_matches(&session, agent_id, &next, previous.clone());
            sessions.mirror_agent_prompt_state(
                session_id,
                agent_id,
                Some(previous.clone()),
                queued,
            )?;
            return Err(error);
        }
        Ok(Some(next))
    })
}

/// Yield is admitted while the native turn is still Working. Its wait, or
/// already prepared wake, owns continuation until the task ledger settles it.
pub(crate) fn supervised_task(
    durable: &DurableKernelStateStore,
    session_id: &str,
    prompt: &PromptQueueItem,
) -> Result<Option<crate::durable_state::agent_lifecycle::AgentTaskExecution>, DaemonError> {
    use crate::durable_state::agent_lifecycle::ExecutionState;
    Ok(durable
        .agent_tasks(Some(session_id), Some(prompt.target_agent_id()))?
        .into_iter()
        .find(|task| {
            task.prompt_id == prompt.id()
                && !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
                && (task.wait.is_some()
                    || task.pending_prompt_id.is_some()
                    || matches!(
                        task.state,
                        ExecutionState::Waiting | ExecutionState::Blocked
                    ))
        }))
}

/// The one rule for a workflow turn that ended without output, shared by the
/// local, app and home-lease settlement paths: the node fails unless the
/// provider is healthy and a recorded wait/Yield still supervises the task.
pub(crate) fn missing_output_is_final(
    durable: &DurableKernelStateStore,
    session_id: &str,
    prompt: &PromptQueueItem,
    provider_failed: bool,
) -> Result<bool, DaemonError> {
    Ok(provider_failed || supervised_task(durable, session_id, prompt)?.is_none())
}

/// The reminder turn is the last chance: it ending without output and
/// without a wait ends the node. Ending a task that still has an obligation
/// leaves it Blocked with `EXHAUSTED_REASON`; a settlement retried after a
/// failed durable write must keep reading that as exhausted, not as supervision.
pub(crate) fn output_follow_up_exhausted(
    durable: &DurableKernelStateStore,
    session_id: &str,
    prompt: &PromptQueueItem,
    has_output: bool,
    provider_failed: bool,
) -> Result<bool, DaemonError> {
    if has_output || !prompt.id().starts_with(FOLLOW_UP_PREFIX) {
        return Ok(false);
    }
    if durable
        .agent_tasks(Some(session_id), Some(prompt.target_agent_id()))?
        .iter()
        .any(|task| {
            task.prompt_id == prompt.id()
                && task.state == crate::durable_state::agent_lifecycle::ExecutionState::Blocked
                && task.reason == EXHAUSTED_REASON
        })
    {
        return Ok(true);
    }
    missing_output_is_final(durable, session_id, prompt, provider_failed)
}

pub(crate) fn local_dispatch(
    session_id: &str,
    provider_run_id: &str,
    prompt: &PromptQueueItem,
) -> crate::app::KernelPromptDispatch {
    crate::app::KernelPromptDispatch {
        session_id: session_id.into(),
        provider_run_id: provider_run_id.into(),
        agent_id: prompt.target_agent_id().into(),
        prompt_id: prompt.id().into(),
        target_active_prompt_id: None,
        source_attachment_id: prompt.source_attachment_id().into(),
        prompt: prompt.prompt().into(),
        hidden_system_context: prompt.hidden_system_context().into(),
        attachments: Vec::new(),
        prompt_origin: prompt.prompt_origin(),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: false,
    }
}

impl crate::app::DaemonApp {
    pub(crate) fn follow_up_missing_workflow_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        previous: &PromptQueueItem,
    ) -> Result<bool, DaemonError> {
        if self
            .provider_run_projection
            .get(provider_run_id)
            .is_some_and(|run| {
                run.state() == crate::provider::ProviderRunState::Ended
                    || run.terminal_diagnostic().is_some()
            })
        {
            return Ok(false);
        }
        let Some(prompt) = prepare_follow_up(
            &self.sessions,
            &self.prompt_state_owner,
            &self.durable_state_store(),
            session_id,
            previous,
        )?
        else {
            return Ok(false);
        };
        crate::transport::flow_control::clear_prompt_activity(self, provider_run_id);
        let agent = self.agents.get_agent(prompt.target_agent_id())?;
        if let Some(remote) = agent.remote_execution() {
            let workflow_context = crate::app::RemoteWorkflowTurnContextResolver::new(self)
                .remote_workflow_turn_context_for_prompt(session_id, agent.id(), &prompt)?;
            let session = self.sessions.get_session(session_id)?;
            let dispatch = crate::app::KernelRemotePromptDispatch {
                session_id: session_id.into(),
                agent_id: agent.id().into(),
                prompt_id: prompt.id().into(),
                worker_kernel_id: remote.worker_kernel_id.clone(),
                leased_agent_id: remote.leased_agent_id.clone(),
                relay_url: remote.relay_url.clone(),
                relay_token: remote.relay_token.clone(),
                source_attachment_id: prompt.source_attachment_id().into(),
                prompt: prompt.prompt().into(),
                hidden_system_context: prompt.hidden_system_context().into(),
                attachments: Vec::new(),
                workspace_live_sync_mode: Some(
                    crate::provider::provider_workspace_live_sync_mode_for_session(
                        agent.provider(),
                        self.config(),
                        Some(&session),
                    ),
                ),
                prompt_origin: prompt.prompt_origin(),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                workflow_context: Some(workflow_context),
            };
            self.defer_remote_prompt_dispatch_after_app_side_effect(dispatch);
        } else {
            crate::app::KernelAgentService::new(self).finish_compat_prompt_dispatch(Some(
                local_dispatch(session_id, provider_run_id, &prompt),
            ))?;
        }
        let _ = crate::app::KernelSessionReadService::new(self).session_snapshot(session_id)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        crate::app::DaemonApp,
        String,
        PromptQueueItem,
        crate::test_support::TestWorktree,
    ) {
        let worktree = crate::test_support::TestWorktree::new("workflow-output-follow-up");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), None)
            .unwrap();
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .unwrap();
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(session.id(), workflow.id(), node.id(), None)
            .unwrap();
        let run = app
            .sessions_mut()
            .invoke_workflow_endpoint(session.id(), workflow.id(), endpoint.id(), None)
            .unwrap();
        let node_id = run.node_runs()[0].id();
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                run.id(),
                node_id,
                "delivery".into(),
                "original workflow prompt".into(),
                None,
                None,
            )
            .unwrap();
        app.sessions_mut()
            .start_workflow_node_run(session.id(), run.id(), node_id)
            .unwrap();
        let prompt = PromptQueueItem::new(
            "first",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(run.id()),
            agent.id(),
            "original workflow prompt",
            PromptStatus::Running,
        )
        .with_workflow_context(run.id(), node_id);
        let crate::session::PromptSubmissionOutcome::Started { prompt } = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .unwrap()
        else {
            panic!()
        };
        (app, session.id().into(), prompt, worktree)
    }

    fn prepare(
        app: &crate::app::DaemonApp,
        session: &str,
        prompt: &PromptQueueItem,
    ) -> Option<PromptQueueItem> {
        prepare_follow_up(
            &app.sessions,
            &app.prompt_state_owner,
            &app.durable_state_store(),
            session,
            prompt,
        )
        .unwrap()
    }

    fn register_helper(
        app: &crate::app::DaemonApp,
        session: &str,
        prompt: &PromptQueueItem,
        run: &str,
    ) {
        use crate::durable_state::agent_lifecycle::Operation;
        app.durable_state_store()
            .agent_lifecycle(Operation::RegisterObligation {
                owner: "local".into(),
                room: session.into(),
                agent: prompt.target_agent_id().into(),
                prompt: prompt.id().into(),
                run: Some(run.into()),
                id: "helper".into(),
                kind: "delegate".into(),
                resource: Some("helper-agent".into()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
    }

    /// Subscribe and Yield on the helper from `prompt`'s turn. `task_id` is the id
    /// of the task the turn belongs to: the first prompt's id, also for the reminder.
    fn yield_to_helper(
        app: &crate::app::DaemonApp,
        session: &str,
        task_id: &str,
        prompt: &PromptQueueItem,
    ) {
        use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Registration};
        let store = app.durable_state_store();
        let now = crate::session::unix_epoch_ms();
        store
            .agent_lifecycle(Operation::Subscribe {
                task: task_id.into(),
                prompt: prompt.id().into(),
                registration: Registration {
                    id: "helper-wait".into(),
                    task_id: task_id.into(),
                    source_id: "helper-agent".into(),
                    obligation_id: Some("helper".into()),
                    source_cursor: 0,
                    live: true,
                },
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Yield {
                task: task_id.into(),
                prompt: prompt.id().into(),
                registrations: vec!["helper-wait".into()],
                cursor: 0,
                deadline: now + 60_000,
                reason: "Wait for helper".into(),
                now,
            })
            .unwrap();
        let task = store
            .agent_tasks(Some(session), Some(prompt.target_agent_id()))
            .unwrap()
            .remove(0);
        assert_eq!(task.state, ExecutionState::Working);
        assert!(task.wait.is_some());
    }

    fn yield_first_turn(
        app: &crate::app::DaemonApp,
        session: &str,
        prompt: &PromptQueueItem,
        run: &str,
    ) {
        register_helper(app, session, prompt, run);
        yield_to_helper(app, session, prompt.id(), prompt);
    }

    #[test]
    fn workflow_output_follow_up_skips_yield_before_native_settlement() {
        let (app, session, prompt, _worktree) = fixture();
        yield_first_turn(&app, &session, &prompt, "run");
        assert!(
            prepare(&app, &session, &prompt).is_none(),
            "Yield must retain the supervised turn"
        );
    }

    fn launch_delivered_run(
        app: &mut crate::app::DaemonApp,
        session: &str,
        prompt: &PromptQueueItem,
    ) -> String {
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session,
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(prompt.target_agent_id()),
            )
            .unwrap();
        app.update_provider_run_projection(run.clone());
        app.mark_active_prompt_delivery(
            session,
            prompt.target_agent_id(),
            prompt.id(),
            DurablePromptDeliveryPhase::Delivered,
            Some(run.id().into()),
            None,
        )
        .unwrap();
        crate::transport::flow_control::note_prompt_started(app, run.id());
        run.id().into()
    }

    fn settle_app_completion(app: &mut crate::app::DaemonApp, session: &str, run: &str) {
        let providers = app.providers.clone();
        let turns = app.active_turns.clone();
        let activity = app.prompt_activity.clone();
        let projection = app.agent_runtime_projection_store();
        let mut settlement =
            crate::app::provider_output_prompt_settlement::ProviderOutputPromptSettlement::new(
                app, providers, turns, activity, projection,
            );
        settlement
            .settle_structured_completion(session, run, true, false)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(
            crate::app::provider_output::STRUCTURED_OUTPUT_EMPTY_POLL_BACKOFF_MS + 75,
        ));
        settlement
            .settle_structured_completion(session, run, false, false)
            .unwrap();
    }

    /// The reminder turn runs under the first turn's task; dispatch moves the
    /// task onto the reminder's prompt id.
    fn begin_reminder_turn(
        app: &crate::app::DaemonApp,
        session: &str,
        reminder: &PromptQueueItem,
        run: &str,
    ) {
        assert!(reminder.id().starts_with(FOLLOW_UP_PREFIX));
        app.durable_state_store()
            .agent_lifecycle(crate::durable_state::agent_lifecycle::Operation::Begin {
                owner: "local".into(),
                room: session.into(),
                agent: reminder.target_agent_id().into(),
                prompt: reminder.id().into(),
                run: Some(run.into()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
    }

    #[test]
    fn workflow_yield_wait_survives_app_settlement() {
        let (mut app, session, prompt, _worktree) = fixture();
        let run = launch_delivered_run(&mut app, &session, &prompt);
        yield_first_turn(&app, &session, &prompt, &run);
        settle_app_completion(&mut app, &session, &run);
        assert_supervised_wait(&app, &session, &prompt, 0);
    }

    #[test]
    fn workflow_yield_during_reminder_survives_app_settlement() {
        let (mut app, session, prompt, _worktree) = fixture();
        let run = launch_delivered_run(&mut app, &session, &prompt);
        register_helper(&app, &session, &prompt, &run);
        settle_app_completion(&mut app, &session, &run);
        let reminder = app
            .prompt_owner_active_prompt_for_agent(&session, prompt.target_agent_id())
            .unwrap()
            .expect("the first miss admits the reminder");
        begin_reminder_turn(&app, &session, &reminder, &run);
        yield_to_helper(&app, &session, prompt.id(), &reminder);
        settle_app_completion(&mut app, &session, &run);
        assert_supervised_wait(&app, &session, &reminder, 1);
    }

    #[test]
    fn workflow_reminder_without_wait_fails_once_on_app_path() {
        let (mut app, session, prompt, _worktree) = fixture();
        let run = launch_delivered_run(&mut app, &session, &prompt);
        register_helper(&app, &session, &prompt, &run);
        settle_app_completion(&mut app, &session, &run);
        let reminder = app
            .prompt_owner_active_prompt_for_agent(&session, prompt.target_agent_id())
            .unwrap()
            .expect("the first miss admits the reminder");
        begin_reminder_turn(&app, &session, &reminder, &run);
        settle_app_completion(&mut app, &session, &run);
        assert_failed_once(&app, &session, &prompt);
    }

    fn bind_leased_agent(app: &mut crate::app::DaemonApp, prompt: &PromptQueueItem) {
        app.agents
            .bind_remote_execution(
                prompt.target_agent_id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker".into(),
                    worker_machine_id: "machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "leased".into(),
                    active_worker_provider_run_id: Some("run".into()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
    }

    fn project_lease_completion(
        app: &mut crate::app::DaemonApp,
        session: &str,
        agent: &str,
        home_prompt_id: &str,
    ) -> usize {
        crate::app::RemoteLeaseRuntime::new(app)
            .project_remote_runtime_projection(
                crate::runtime::relay_peer_authority::test_projection_authority("worker"),
                crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                    home_session_id: session.into(),
                    home_agent_id: agent.into(),
                    provider_run_id: "run".into(),
                    provider_run: None,
                    prompts: Vec::new(),
                    output_chunks: Vec::new(),
                    notices: Vec::new(),
                    completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
                        message_id: format!("completion-{home_prompt_id}"),
                        completed_at_ms: crate::session::unix_epoch_ms(),
                        home_prompt_id: Some(home_prompt_id.into()),
                        provider_termination: None,
                        failure: None,
                    }],
                },
            )
            .unwrap()
            .completions
            .len()
    }

    #[test]
    fn workflow_yield_wait_survives_home_lease_projection() {
        let (mut app, session, prompt, _worktree) = fixture();
        bind_leased_agent(&mut app, &prompt);
        let projected = crate::provider::projected_leased_provider_run_id("leased", "run");
        yield_first_turn(&app, &session, &prompt, &projected);
        let completions =
            project_lease_completion(&mut app, &session, prompt.target_agent_id(), prompt.id());
        assert_eq!(completions, 1);
        assert!(app
            .take_deferred_workflow_remote_prompt_dispatches()
            .is_empty());
        assert_supervised_wait(&app, &session, &prompt, 0);
    }

    #[test]
    fn workflow_yield_during_reminder_survives_home_lease_projection() {
        let (mut app, session, prompt, _worktree) = fixture();
        bind_leased_agent(&mut app, &prompt);
        let projected = crate::provider::projected_leased_provider_run_id("leased", "run");
        register_helper(&app, &session, &prompt, &projected);
        project_lease_completion(&mut app, &session, prompt.target_agent_id(), prompt.id());
        let reminder = app
            .prompt_owner_active_prompt_for_agent(&session, prompt.target_agent_id())
            .unwrap()
            .expect("the first miss admits the reminder");
        assert_eq!(
            app.take_deferred_workflow_remote_prompt_dispatches().len(),
            1
        );
        begin_reminder_turn(&app, &session, &reminder, &projected);
        yield_to_helper(&app, &session, prompt.id(), &reminder);
        project_lease_completion(&mut app, &session, prompt.target_agent_id(), reminder.id());
        assert!(app
            .take_deferred_workflow_remote_prompt_dispatches()
            .is_empty());
        assert_supervised_wait(&app, &session, &reminder, 1);
    }

    #[test]
    fn workflow_reminder_without_wait_fails_once_on_home_lease_path() {
        let (mut app, session, prompt, _worktree) = fixture();
        bind_leased_agent(&mut app, &prompt);
        let projected = crate::provider::projected_leased_provider_run_id("leased", "run");
        register_helper(&app, &session, &prompt, &projected);
        project_lease_completion(&mut app, &session, prompt.target_agent_id(), prompt.id());
        let reminder = app
            .prompt_owner_active_prompt_for_agent(&session, prompt.target_agent_id())
            .unwrap()
            .expect("the first miss admits the reminder");
        begin_reminder_turn(&app, &session, &reminder, &projected);
        project_lease_completion(&mut app, &session, prompt.target_agent_id(), reminder.id());
        assert_failed_once(&app, &session, &prompt);
    }

    fn assert_supervised_wait(
        app: &crate::app::DaemonApp,
        session: &str,
        prompt: &PromptQueueItem,
        follow_ups: usize,
    ) {
        let state = app.sessions.get_session(session).unwrap();
        assert!(
            app.prompt_owner_active_prompt_for_agent_snapshot(session, prompt.target_agent_id())
                .unwrap()
                .is_none(),
            "a yielded workflow turn must settle without a reminder"
        );
        let run = state
            .workflow_run(prompt.workflow_run_id().unwrap())
            .unwrap();
        assert!(!run.status().is_terminal());
        assert!(run.failure_events().is_empty());
        let store = app.durable_state_store();
        let task = store
            .agent_tasks(Some(session), Some(prompt.target_agent_id()))
            .unwrap()
            .remove(0);
        assert_eq!(
            task.state,
            crate::durable_state::agent_lifecycle::ExecutionState::Waiting
        );
        assert!(task.wait.is_some());
        let events = store
            .load_events_by_kind("workflow.runtime.updated")
            .unwrap();
        assert!(events
            .iter()
            .any(|event| event.payload["reason"] == "workflow_task_waiting"));
        assert_eq!(
            events
                .iter()
                .filter(|event| event.payload["reason"] == "workflow_output_follow_up")
                .count(),
            follow_ups
        );
    }

    fn assert_failed_once(app: &crate::app::DaemonApp, session: &str, prompt: &PromptQueueItem) {
        assert!(app
            .prompt_owner_active_prompt_for_agent_snapshot(session, prompt.target_agent_id())
            .unwrap()
            .is_none());
        let state = app.sessions.get_session(session).unwrap();
        let run = state
            .workflow_run(prompt.workflow_run_id().unwrap())
            .cloned()
            .or_else(|| {
                app.durable_state_store()
                    .resolve_workflow_run(
                        state.host_daemon_id(),
                        session,
                        prompt.workflow_run_id().unwrap(),
                    )
                    .unwrap()
            })
            .unwrap();
        assert_eq!(run.status(), WorkflowRunStatus::Failed);
        assert_eq!(
            run.failure_events().last().unwrap().kind(),
            crate::session::WorkflowFailureKind::MissingStructuredOutput
        );
        assert_eq!(
            app.durable_state_store()
                .load_events_by_kind("workflow.runtime.updated")
                .unwrap()
                .iter()
                .filter(|event| event.payload["reason"] == "workflow_output_follow_up")
                .count(),
            1
        );
    }

    #[test]
    fn workflow_output_follow_up_is_durable_and_bounded() {
        let (app, session, prompt, _worktree) = fixture();
        let next = prepare(&app, &session, &prompt).unwrap();
        assert!(!next.prompt().trim().is_empty());
        assert!(crate::session::is_internal_prompt_echo_id(next.id()));
        assert!(!next.prompt().contains(REMINDER));
        assert!(next.hidden_system_context().contains(REMINDER));
        assert_eq!(next.workflow_node_run_id(), prompt.workflow_node_run_id());
        assert!(prepare(&app, &session, &next).is_none());
        assert!(
            prepare(&app, &session, &prompt).is_none(),
            "stale completion cannot send twice"
        );
        assert_eq!(
            app.durable_state_store()
                .load_events_by_kind("workflow.runtime.updated")
                .unwrap()
                .iter()
                .filter(|event| event.payload["reason"] == "workflow_output_follow_up")
                .count(),
            1
        );
    }

    #[test]
    fn workflow_output_follow_up_skips_cancellation() {
        let (app, session, prompt, _worktree) = fixture();
        let state = app.sessions.get_session(&session).unwrap();
        let cancelled = app
            .prompt_state_owner
            .begin_cancelling_active_prompt(&state, prompt.target_agent_id())
            .unwrap();
        assert!(prepare(&app, &session, &cancelled).is_none());
        assert!(prepare(&app, &session, &prompt).is_none());
    }

    #[test]
    fn workflow_output_follow_up_skips_stopped_run() {
        let (app, session, prompt, _worktree) = fixture();
        app.sessions_mut()
            .stop_workflow_run_with_final_output(
                &session,
                prompt.workflow_run_id().unwrap(),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(prepare(&app, &session, &prompt).is_none());
    }

    #[test]
    fn workflow_output_follow_up_remote_projection_is_bounded_and_ignores_replay() {
        let (mut app, session, prompt, _worktree) = fixture();
        app.agents
            .bind_remote_execution(
                prompt.target_agent_id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker".into(),
                    worker_machine_id: "machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "leased".into(),
                    active_worker_provider_run_id: Some("run".into()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        let project = |app: &mut crate::app::DaemonApp, id: &str| {
            crate::app::RemoteLeaseRuntime::new(app)
                .project_remote_runtime_projection(
                    crate::runtime::relay_peer_authority::test_projection_authority("worker"),
                    crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                        home_session_id: session.clone(),
                        home_agent_id: prompt.target_agent_id().into(),
                        provider_run_id: "run".into(),
                        provider_run: None,
                        prompts: Vec::new(),
                        output_chunks: Vec::new(),
                        notices: Vec::new(),
                        completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
                            message_id: format!("completion-{id}"),
                            completed_at_ms: crate::session::unix_epoch_ms(),
                            home_prompt_id: Some(id.into()),
                            provider_termination: None,
                            failure: None,
                        }],
                    },
                )
                .unwrap()
        };
        project(&mut app, prompt.id());
        let next = app
            .prompt_owner_active_prompt_for_agent(&session, prompt.target_agent_id())
            .unwrap()
            .unwrap();
        assert!(next.id().starts_with(FOLLOW_UP_PREFIX));
        assert_eq!(
            app.take_deferred_workflow_remote_prompt_dispatches().len(),
            1
        );
        project(&mut app, prompt.id());
        assert!(app
            .take_deferred_workflow_remote_prompt_dispatches()
            .is_empty());
        assert_eq!(
            app.prompt_owner_active_prompt_for_agent(&session, prompt.target_agent_id())
                .unwrap()
                .unwrap()
                .id(),
            next.id()
        );
        project(&mut app, next.id());
        assert!(app
            .take_deferred_workflow_remote_prompt_dispatches()
            .is_empty());
        assert!(app
            .prompt_owner_active_prompt_for_agent(&session, prompt.target_agent_id())
            .unwrap()
            .is_none());
        let run = app
            .sessions
            .get_session(&session)
            .unwrap()
            .workflow_run(prompt.workflow_run_id().unwrap())
            .cloned()
            .or_else(|| {
                app.durable_state_store()
                    .resolve_workflow_run(
                        app.sessions.get_session(&session).unwrap().host_daemon_id(),
                        &session,
                        prompt.workflow_run_id().unwrap(),
                    )
                    .unwrap()
            })
            .unwrap();
        assert_eq!(run.status(), WorkflowRunStatus::Failed);
        assert_eq!(
            run.failure_events().last().unwrap().kind(),
            crate::session::WorkflowFailureKind::MissingStructuredOutput
        );
    }
}
