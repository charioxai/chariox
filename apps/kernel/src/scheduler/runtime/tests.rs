use std::fs;
use std::path::PathBuf;

use crate::agent::CreateAgentRequest;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::provider::LaunchProviderRequest;
use crate::session::{
    CreateSessionRequest, PromptSubmissionOutcome, RuntimeSession, WorkflowHandoffValidationPolicy,
    WorkflowMessage, WorkflowNodeRunStatus, WorkflowRun, WorkflowRunStatus,
};
use crate::{DaemonApp, DaemonConfig};

use super::prepare_workflow_turn_prompt;

fn create_scheduler_session_and_agent(
    app: &mut DaemonApp,
    client_id: &str,
) -> (RuntimeSession, String) {
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut *app)
        .create_session(CreateSessionRequest::new(
            "workspace-scheduler",
            "worktree-scheduler",
        ))
        .expect("session should exist");
    crate::app::KernelSessionService::new(app)
        .attach(AttachRequest::new(
            session.id(),
            client_id,
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let agent_id = crate::app::KernelSessionService::new(&mut *app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-scheduler")
                .with_model("test-model")
                .with_worktree("worktree-scheduler"),
        )
        .expect("agent should spawn")
        .id()
        .to_string();
    (session, agent_id)
}

fn create_workflow_node(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_alias: &str,
    agent_id: &str,
) -> (String, String) {
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session_id, Some(workflow_alias.to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session_id, &workflow_id, agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    (workflow_id, node_id)
}

fn invoke_workflow_node(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_id: &str,
    node_id: &str,
) -> WorkflowRun {
    app.sessions_mut()
        .set_workflow_flush_agent_context_before_run(session_id, &workflow_id, false)
        .expect("workflow flush context should update");
    app.sessions_mut()
        .create_workflow_endpoint(
            session_id,
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    let (workflow_run, _, _) = app
        .invoke_workflow_endpoint_and_schedule(
            session_id,
            &workflow_id,
            "entry",
            Some("start".to_string()),
        )
        .expect("workflow should invoke");
    workflow_run
}

fn prepare_active_workflow_run(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_id: &str,
    node_id: &str,
) -> WorkflowRun {
    app.sessions_mut()
        .set_workflow_flush_agent_context_before_run(session_id, workflow_id, false)
        .expect("workflow flush context should update");
    app.sessions_mut()
        .create_workflow_endpoint(session_id, workflow_id, node_id, Some("entry".to_string()))
        .expect("endpoint should exist");
    app.sessions_mut()
        .invoke_workflow_endpoint(session_id, workflow_id, "entry", Some("start".to_string()))
        .expect("workflow run should become active")
}

#[test]
fn direct_user_prompt_starts_before_active_workflow_reaches_idle_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-user-first-idle");
    let attachment_id = app.attachments().list_session_attachment_ids(session.id())[0].clone();
    let (workflow_id, node_id) =
        create_workflow_node(&mut app, session.id(), "wf-user-first-idle", &agent_id);
    let workflow_run = prepare_active_workflow_run(&mut app, session.id(), &workflow_id, &node_id);

    let PromptSubmissionOutcome::Started {
        prompt: user_prompt,
    } = app
        .submit_prompt(
            session.id(),
            &attachment_id,
            Some(&agent_id),
            "direct user turn",
            Vec::new(),
        )
        .expect("idle agent should start the direct user prompt")
    else {
        panic!("idle direct user prompt should start");
    };
    super::schedule_workflow_run_entry_node(&mut app, session.id(), &workflow_run)
        .expect("workflow turn should schedule behind the user turn");

    let state = app
        .sessions()
        .get_session(session.id())
        .expect("session should resolve");
    assert_eq!(
        state
            .active_prompt_for_agent(&agent_id)
            .map(|prompt| prompt.id()),
        Some(user_prompt.id())
    );
    let queued = state
        .queued_prompts_for_agent(&agent_id)
        .expect("workflow prompt should queue");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].workflow_run_id(), Some(workflow_run.id()));

    app.prompt_owner_complete_active_prompt_only(session.id(), &agent_id)
        .expect("user prompt should settle");
    let promoted = crate::app::KernelAgentService::new(&mut app)
        .advance_next_queued_prompt(session.id(), &agent_id, None)
        .expect("queued workflow prompt should advance")
        .expect("workflow prompt should resume");
    assert_eq!(promoted.workflow_run_id(), Some(workflow_run.id()));
}

#[test]
fn busy_agent_preserves_user_fifo_before_workflow_turn() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-user-first-busy");
    let attachment_id = app.attachments().list_session_attachment_ids(session.id())[0].clone();
    let (workflow_id, node_id) =
        create_workflow_node(&mut app, session.id(), "wf-user-first-busy", &agent_id);
    let workflow_run = prepare_active_workflow_run(&mut app, session.id(), &workflow_id, &node_id);

    let PromptSubmissionOutcome::Started { prompt: first_user } = app
        .submit_prompt(
            session.id(),
            &attachment_id,
            Some(&agent_id),
            "first user turn",
            Vec::new(),
        )
        .expect("first user prompt should start")
    else {
        panic!("first user prompt should start");
    };
    let PromptSubmissionOutcome::Queued {
        prompt: second_user,
    } = app
        .submit_prompt(
            session.id(),
            &attachment_id,
            Some(&agent_id),
            "second user turn",
            Vec::new(),
        )
        .expect("second user prompt should queue")
    else {
        panic!("second user prompt should queue");
    };
    super::schedule_workflow_run_entry_node(&mut app, session.id(), &workflow_run)
        .expect("workflow turn should queue behind both user turns");

    let state = app
        .sessions()
        .get_session(session.id())
        .expect("session should resolve");
    assert_eq!(
        state
            .active_prompt_for_agent(&agent_id)
            .map(|prompt| prompt.id()),
        Some(first_user.id())
    );
    let queued = state
        .queued_prompts_for_agent(&agent_id)
        .expect("queued prompts should exist");
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].id(), second_user.id());
    assert!(queued[0].workflow_run_id().is_none());
    assert_eq!(queued[1].workflow_run_id(), Some(workflow_run.id()));

    app.prompt_owner_complete_active_prompt_only(session.id(), &agent_id)
        .expect("first user prompt should settle");
    let promoted_user = crate::app::KernelAgentService::new(&mut app)
        .advance_next_queued_prompt(session.id(), &agent_id, None)
        .expect("second user prompt should advance")
        .expect("second user prompt should resume");
    assert_eq!(promoted_user.prompt(), second_user.prompt());
    assert!(promoted_user.workflow_run_id().is_none());
    app.prompt_owner_complete_active_prompt_only(session.id(), &agent_id)
        .expect("second user prompt should settle");
    let promoted_workflow = crate::app::KernelAgentService::new(&mut app)
        .advance_next_queued_prompt(session.id(), &agent_id, None)
        .expect("workflow prompt should advance")
        .expect("workflow prompt should resume");
    assert_eq!(promoted_workflow.workflow_run_id(), Some(workflow_run.id()));
}

#[test]
fn user_prompts_from_every_pane_jump_ahead_of_queued_workflow_follow_up() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-freeform-priority");
    let freeform_attachment =
        app.attachments().list_session_attachment_ids(session.id())[0].clone();
    let workflow_trace_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-workflow-trace-priority",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("workflow trace attachment should attach")
        .id()
        .to_string();
    let slice_trace_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-slice-trace-priority",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("slice trace attachment should attach")
        .id()
        .to_string();
    let (workflow_id, node_id) =
        create_workflow_node(&mut app, session.id(), "wf-user-priority", &agent_id);
    let active_workflow = invoke_workflow_node(&mut app, session.id(), &workflow_id, &node_id);
    let active_workflow_prompt = app
        .sessions()
        .get_session(session.id())
        .expect("session should resolve")
        .active_prompt_for_agent(&agent_id)
        .expect("workflow prompt should be active")
        .clone();
    assert_eq!(
        active_workflow_prompt.workflow_run_id(),
        Some(active_workflow.id())
    );

    let queued_workflow_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(active_workflow.id()),
        &agent_id,
        "workflow follow-up",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(active_workflow.id(), active_workflow.node_runs()[0].id());
    let PromptSubmissionOutcome::Queued {
        prompt: queued_workflow,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_workflow_prompt, false)
        .expect("workflow follow-up should queue")
    else {
        panic!("workflow follow-up should not replace the active workflow prompt");
    };

    let mut user_prompts = Vec::new();
    for (attachment_id, text) in [
        (&freeform_attachment, "freeform user prompt"),
        (&workflow_trace_attachment, "workflow trace user prompt"),
        (&slice_trace_attachment, "slice trace user prompt"),
    ] {
        let PromptSubmissionOutcome::Queued { prompt } = app
            .submit_prompt(
                session.id(),
                attachment_id,
                Some(&agent_id),
                text,
                Vec::new(),
            )
            .expect("busy agent should queue the user prompt")
        else {
            panic!("active workflow prompt should remain active");
        };
        user_prompts.push(prompt);
    }

    let state = app
        .sessions()
        .get_session(session.id())
        .expect("session should resolve");
    assert_eq!(
        state
            .active_prompt_for_agent(&agent_id)
            .map(|prompt| prompt.id()),
        Some(active_workflow_prompt.id())
    );
    let queued = state
        .queued_prompts_for_agent(&agent_id)
        .expect("queued prompts should exist");
    assert_eq!(queued.len(), 4);
    assert_eq!(
        queued.iter().map(|prompt| prompt.id()).collect::<Vec<_>>(),
        vec![
            user_prompts[0].id(),
            user_prompts[1].id(),
            user_prompts[2].id(),
            queued_workflow.id(),
        ],
        "user prompts should preserve FIFO across panes and precede workflow follow-up",
    );

    app.prompt_owner_complete_active_prompt_only(session.id(), &agent_id)
        .expect("active workflow prompt should settle");
    for expected in &user_prompts {
        let promoted = crate::app::KernelAgentService::new(&mut app)
            .advance_next_queued_prompt(session.id(), &agent_id, None)
            .expect("user prompt promotion should succeed")
            .expect("user prompt should promote");
        assert_eq!(promoted.prompt(), expected.prompt());
        assert!(promoted.workflow_run_id().is_none());
        app.prompt_owner_complete_active_prompt_only(session.id(), &agent_id)
            .expect("promoted user prompt should settle");
    }
    let promoted_workflow = crate::app::KernelAgentService::new(&mut app)
        .advance_next_queued_prompt(session.id(), &agent_id, None)
        .expect("workflow follow-up promotion should succeed")
        .expect("workflow follow-up should promote after user prompts");
    assert_eq!(promoted_workflow.prompt(), queued_workflow.prompt());
    assert_eq!(
        promoted_workflow.workflow_run_id(),
        Some(active_workflow.id())
    );
}

#[test]
fn workflow_start_preflights_local_provider_runs_for_all_nodes() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, first_agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-preflight");
    let second_agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("second-scheduler-agent")
                .with_model("test-model")
                .with_worktree("worktree-scheduler"),
        )
        .expect("second agent should spawn")
        .id()
        .to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf-scheduler-preflight".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    app.sessions_mut()
        .set_workflow_flush_agent_context_before_run(session.id(), &workflow_id, false)
        .expect("preflight test should preserve provider context");
    let first_node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &first_agent_id)
        .expect("first node should be added")
        .id()
        .to_string();
    let second_node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &second_agent_id)
        .expect("second node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .add_workflow_edge(
            session.id(),
            &workflow_id,
            &first_node_id,
            &second_node_id,
            None,
            None,
        )
        .expect("workflow edge should be added");
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &first_node_id,
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should exist");

    let workflow_run = app
        .invoke_workflow_endpoint_and_schedule(
            session.id(),
            &workflow_id,
            "entry",
            Some("start".to_string()),
        )
        .expect("workflow should invoke")
        .0;

    let first_provider_run = app
        .providers()
        .get_run_for_agent(session.id(), &first_agent_id)
        .expect("entry agent provider should be preflighted");
    let second_provider_run = app
        .providers()
        .get_run_for_agent(session.id(), &second_agent_id)
        .expect("downstream agent provider should be preflighted");
    assert_ne!(first_provider_run.id(), second_provider_run.id());
    assert!(first_provider_run.workflow_tools_enabled());
    assert!(second_provider_run.workflow_tools_enabled());
    assert_eq!(
        app.providers()
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == session.id()
                    && matches!(
                        run.agent_instance_id(),
                        Some(id) if id == first_agent_id || id == second_agent_id
                    )
            })
            .count(),
        2,
        "cold workflow admission must create exactly one provider run per agent"
    );
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_provider_run_id(),
        Some(first_provider_run.id()),
        "entry node should remain the active workflow provider after preflight"
    );
    assert_eq!(workflow_run.node_runs().len(), 1);
    assert_eq!(workflow_run.node_runs()[0].node_id(), first_node_id);
}

#[test]
fn workflow_notice_uses_current_run_after_dispatch_failure() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-failed-notice");
    let (workflow_id, node_id) =
        create_workflow_node(&mut app, session.id(), "wf-failed-notice", &agent_id);
    let stale_workflow_run = invoke_workflow_node(&mut app, session.id(), &workflow_id, &node_id);
    let node_run_id = stale_workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .fail_workflow_node_run(session.id(), stale_workflow_run.id(), &node_run_id)
        .expect("workflow node should fail");

    let current = super::lifecycle::current_workflow_run_for_notice(
        &app,
        session.id(),
        stale_workflow_run.clone(),
    );

    assert_eq!(stale_workflow_run.status(), WorkflowRunStatus::Running);
    assert_eq!(current.status(), WorkflowRunStatus::Failed);
    assert_eq!(
        super::lifecycle::workflow_run_status_notice_suffix(current.status()),
        "failed"
    );
}

#[test]
fn provider_completion_without_structured_output_fails_without_automatic_retry() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-missing-output");
    let (workflow_id, node_id) = create_workflow_node(
        &mut app,
        session.id(),
        "wf-scheduler-missing-output",
        &agent_id,
    );
    app.sessions_mut()
        .set_workflow_node_can_complete_run(session.id(), &workflow_id, &node_id, true)
        .expect("node completion setting should update");
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            &workflow_id,
            "entry",
            Some("return challenge SCHEDULER-FAILURE".to_string()),
        )
        .expect("workflow should invoke");
    super::schedule_workflow_run_entry_node(&mut app, session.id(), &workflow_run)
        .expect("entry prompt should schedule");
    let first_node_run_id = workflow_run.node_runs()[0].id().to_string();
    let completed_prompt = app
        .prompt_owner_complete_active_prompt_only(session.id(), &agent_id)
        .expect("entry prompt should complete without advancing");

    super::on_workflow_prompt_completed(&mut app, session.id(), &completed_prompt, None)
        .expect("missing structured output should become a visible failure");

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should resolve");
    assert!(session_state.workflow_run(workflow_run.id()).is_none());
    let resolved_run = app
        .durable_state_store()
        .resolve_workflow_run(session.host_daemon_id(), session.id(), workflow_run.id())
        .expect("durable workflow run should load")
        .expect("failed workflow run should be archived");
    assert_eq!(resolved_run.status(), WorkflowRunStatus::Failed);
    assert_eq!(resolved_run.node_runs().len(), 1);
    assert_eq!(
        resolved_run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::Failed
    );
    assert!(resolved_run.failure_events().iter().any(|event| {
        event.kind() == crate::session::WorkflowFailureKind::MissingStructuredOutput
            && event.source_node_run_id() == first_node_run_id
    }));
    assert!(session_state.active_prompt_for_agent(&agent_id).is_none());
}

#[test]
fn terminal_provider_completion_releases_claim_and_retries_blocked_workflow() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (failed_session, failed_agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-terminal-provider-failure");
    let (failed_workflow_id, failed_node_id) = create_workflow_node(
        &mut app,
        failed_session.id(),
        "wf-terminal-provider-failure",
        &failed_agent_id,
    );
    let _failed_run = invoke_workflow_node(
        &mut app,
        failed_session.id(),
        &failed_workflow_id,
        &failed_node_id,
    );
    let failed_provider_run_id = app
        .providers()
        .get_run_for_agent(failed_session.id(), &failed_agent_id)
        .expect("failed workflow provider should exist")
        .id()
        .to_string();

    let (blocked_session, blocked_agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-blocked-after-provider-failure");
    let (blocked_workflow_id, blocked_node_id) = create_workflow_node(
        &mut app,
        blocked_session.id(),
        "wf-blocked-after-provider-failure",
        &blocked_agent_id,
    );
    let blocked_run = invoke_workflow_node(
        &mut app,
        blocked_session.id(),
        &blocked_workflow_id,
        &blocked_node_id,
    );
    let blocked_before_failure = app
        .sessions()
        .resolve_workflow_run_ref(blocked_session.id(), blocked_run.id())
        .expect("blocked workflow should resolve");
    assert_eq!(blocked_before_failure.status(), WorkflowRunStatus::Waiting);
    assert_eq!(
        blocked_before_failure.node_runs()[0].status(),
        WorkflowNodeRunStatus::BlockedOnWorkspaceClaim,
    );

    app.providers()
        .record_terminal_diagnostic(
            &failed_provider_run_id,
            "Provider reported a resource limit".to_string(),
        )
        .expect("terminal diagnostic should be recorded");
    let completed_prompt = app
        .prompt_owner_complete_active_prompt_only(failed_session.id(), &failed_agent_id)
        .expect("failed workflow prompt should settle");
    super::on_workflow_prompt_completed(
        &mut app,
        failed_session.id(),
        &completed_prompt,
        Some(&failed_provider_run_id),
    )
    .expect("terminal provider completion should settle the workflow");

    let retried_run = app
        .sessions()
        .resolve_workflow_run_ref(blocked_session.id(), blocked_run.id())
        .expect("blocked workflow should resolve after claim release");
    assert_eq!(retried_run.status(), WorkflowRunStatus::Running);
    assert_eq!(
        retried_run.node_runs()[0].status(),
        WorkflowNodeRunStatus::Running,
    );
}

#[test]
fn workflow_completion_ignores_provider_output_recorded_before_prompt_dispatch() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-dispatch-boundary");
    let (workflow_id, node_id) = create_workflow_node(
        &mut app,
        session.id(),
        "wf-scheduler-dispatch-boundary",
        &agent_id,
    );
    app.sessions_mut()
        .set_workflow_node_can_complete_run(session.id(), &workflow_id, &node_id, true)
        .expect("node completion setting should update");
    let workflow_run = invoke_workflow_node(&mut app, session.id(), &workflow_id, &node_id);
    let provider_run = app
        .providers()
        .get_run_for_agent(session.id(), &agent_id)
        .expect("workflow provider should exist");
    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should resolve");
    let node_run = session_state
        .workflow_run(workflow_run.id())
        .expect("workflow run should resolve")
        .node_runs()
        .iter()
        .find(|node_run| node_run.node_id() == node_id)
        .expect("workflow node run should resolve");
    let dispatched_at_ms = node_run
        .turn_envelope()
        .and_then(|envelope| envelope.dispatched_at_ms())
        .expect("workflow turn should be dispatched");
    let mut prior_turn_output = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        provider_run.id(),
        Some(&agent_id),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        Some("prior-turn-output".to_string()),
        r#"{"summary":"wrong turn","output":{"message":"prior review"}}"#,
    );
    prior_turn_output.timestamp_ms = dispatched_at_ms.saturating_sub(1);

    let completion = super::completion::build_workflow_completion_snapshot_from_history(
        &session_state,
        vec![prior_turn_output],
        session.id(),
        workflow_run.id(),
        node_run.id(),
        provider_run.id(),
    );

    assert!(
        completion.is_none(),
        "output from the previous provider turn must not complete the newly dispatched workflow",
    );
}

#[test]
fn workflow_instruction_reference_is_written_under_kernel_state_root() {
    let _guard = crate::env_lock::lock();
    let config = DaemonConfig::for_tests();
    let runtime_root = config.workflow_runtime_artifact_root();
    let chariox_home = runtime_root
        .parent()
        .expect("test runtime root should have a parent")
        .join("prompt-home");
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent_id) = create_scheduler_session_and_agent(&mut app, "client-scheduler");

    let workdir = std::env::temp_dir().join(format!(
        "chariox-workflow-runtime-test-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&workdir);
    fs::create_dir_all(&workdir).expect("workdir should exist");
    let previous_chariox_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &chariox_home);
    app.launch_provider(
        LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "test-model",
        )
        .with_agent_id(agent_id.clone())
        .with_working_directory(workdir.clone()),
    )
    .expect("provider run should launch");

    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf-scheduler".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .update_workflow_node_instructions(
            session.id(),
            &workflow_id,
            &node_id,
            Some("Read me from a workspace-local hidden file.".to_string()),
        )
        .expect("instructions should update");
    app.sessions_mut()
        .set_workflow_flush_agent_context_before_run(session.id(), &workflow_id, false)
        .expect("workflow flush context should update");
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    let (workflow_run, _, _) = app
        .invoke_workflow_endpoint_and_schedule(
            session.id(),
            &workflow_id,
            "entry",
            Some("start".to_string()),
        )
        .expect("workflow should invoke");
    let node_run_id = workflow_run
        .node_runs()
        .first()
        .expect("node run should exist")
        .id()
        .to_string();

    let prompt = prepare_workflow_turn_prompt(
        &app,
        session.id(),
        workflow_run.id(),
        &node_run_id,
        &node_id,
        "start",
        Option::<&[WorkflowMessage]>::None,
    )
    .expect("prompt should build");

    let prefix = runtime_root
        .join(session.id())
        .join(workflow_run.id())
        .join("workflow-instructions");
    let prefix_string = prefix.to_string_lossy().to_string();
    assert!(
        prompt.contains(&prefix_string),
        "prompt should reference a file under the kernel state root: {prompt}"
    );
    let expected_file = prefix.join(format!("node-{node_id}.md"));
    assert!(expected_file.exists(), "instruction file should be written");
    let contents = fs::read_to_string(&expected_file).expect("instruction file should read");
    assert!(contents.contains("Read me from a workspace-local hidden file."));
    assert!(
        !workdir.join(".chariox").exists(),
        "automatic workflow runtime state must not create a project-local .chariox directory"
    );
    let expected_prompt_template = chariox_home
        .join("prompts")
        .join("workflow")
        .join("turn.md");
    assert!(
        expected_prompt_template.exists(),
        "workflow system prompt template should be materialized"
    );
    let prompt_template_contents =
        fs::read_to_string(&expected_prompt_template).expect("template should read");
    assert!(prompt_template_contents.contains("ack_workflow_turn"));
    assert!(prompt_template_contents.contains("Do not ask the user which workflow runtime tool"));
    assert!(
        prompt.contains("If you do not remember them exactly, read that file before continuing.")
    );
    assert!(prompt.contains("Do not ask the user which workflow runtime tool"));
    if let Some(previous_chariox_home) = previous_chariox_home {
        std::env::set_var("CHARIOX_HOME", previous_chariox_home);
    } else {
        std::env::remove_var("CHARIOX_HOME");
    }
    let _ = fs::remove_dir_all(PathBuf::from(workdir));
}

#[test]
fn workflow_node_prompt_lists_public_multi_edge_routing_contracts() {
    let _guard = crate::env_lock::lock();
    let home = std::env::temp_dir().join(format!(
        "chariox-workflow-routing-prompt-test-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).expect("test home should exist");
    let previous_chariox_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &home);
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, router_agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-routing");
    let analyst_agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("analyst-agent")
                .with_model("test-model"),
        )
        .expect("analyst agent should spawn")
        .id()
        .to_string();
    let reviewer_agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("reviewer-agent")
                .with_model("test-model"),
        )
        .expect("reviewer agent should spawn")
        .id()
        .to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf-routing-contracts".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let router_node_id = app
        .sessions_mut()
        .add_workflow_node_owned(
            session.id(),
            &workflow_id,
            &router_agent_id,
            "local".to_string(),
            "local".to_string(),
            "Router".to_string(),
        )
        .expect("router node should be added")
        .id()
        .to_string();
    let analyst_node_id = app
        .sessions_mut()
        .add_workflow_node_owned(
            session.id(),
            &workflow_id,
            &analyst_agent_id,
            "local".to_string(),
            "local".to_string(),
            "Analyst".to_string(),
        )
        .expect("analyst node should be added")
        .id()
        .to_string();
    let reviewer_node_id = app
        .sessions_mut()
        .add_workflow_node_owned(
            session.id(),
            &workflow_id,
            &reviewer_agent_id,
            "local".to_string(),
            "local".to_string(),
            "Reviewer".to_string(),
        )
        .expect("reviewer node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .update_workflow_node_instructions(
            session.id(),
            &workflow_id,
            &analyst_node_id,
            Some("Analyze quantitative evidence and route only analysis tasks here.".to_string()),
        )
        .expect("analyst instructions should update");
    app.sessions_mut()
        .update_workflow_node_instructions(
            session.id(),
            &workflow_id,
            &reviewer_node_id,
            Some(
                "Review wording, risks, and acceptance criteria for routed review tasks."
                    .to_string(),
            ),
        )
        .expect("reviewer instructions should update");
    let analyst_edge_id = app
        .sessions_mut()
        .add_workflow_edge(
            session.id(),
            &workflow_id,
            &router_node_id,
            &analyst_node_id,
            Some("schema:analysis".to_string()),
            Some(WorkflowHandoffValidationPolicy::Halt),
        )
        .expect("analysis edge should be added")
        .id()
        .to_string();
    let reviewer_edge_id = app
        .sessions_mut()
        .add_workflow_edge(
            session.id(),
            &workflow_id,
            &router_node_id,
            &reviewer_node_id,
            Some("schema:review".to_string()),
            Some(WorkflowHandoffValidationPolicy::Warn),
        )
        .expect("review edge should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .set_workflow_flush_agent_context_before_run(session.id(), &workflow_id, false)
        .expect("workflow flush context should update");
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &router_node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    let workflow_run = app
        .invoke_workflow_endpoint_and_schedule(
            session.id(),
            &workflow_id,
            "entry",
            Some("route this task".to_string()),
        )
        .expect("workflow should invoke")
        .0;
    let node_run_id = workflow_run.node_runs()[0].id().to_string();

    let prompt = prepare_workflow_turn_prompt(
        &app,
        session.id(),
        workflow_run.id(),
        &node_run_id,
        &router_node_id,
        "route this task",
        Option::<&[WorkflowMessage]>::None,
    )
    .expect("prompt should build");

    assert!(prompt.contains("<outgoing-edge-contracts>"));
    assert!(prompt.contains(&format!(
        "- edge {analyst_edge_id} -> {analyst_node_id} (Analyst), handoff_schema_ref: schema:analysis, validation_policy: halt"
    )));
    assert!(prompt.contains(&format!(
        "- edge {reviewer_edge_id} -> {reviewer_node_id} (Reviewer), handoff_schema_ref: schema:review, validation_policy: warn"
    )));
    assert!(!prompt.contains("Analyze quantitative evidence and route only analysis tasks here."));
    assert!(
        !prompt.contains("Review wording, risks, and acceptance criteria for routed review tasks.")
    );
    assert!(!prompt.contains("target_instructions"));
    assert!(prompt.contains("workflow_handoffs"));
    assert!(prompt.contains("edge_id"));
    assert!(prompt.contains("to_node_id"));
    assert!(prompt.contains("validate only the routed message inside each selected edge entry"));
    assert!(prompt.contains("do not validate the outer routing wrapper"));
    if let Some(previous_chariox_home) = previous_chariox_home {
        std::env::set_var("CHARIOX_HOME", previous_chariox_home);
    } else {
        std::env::remove_var("CHARIOX_HOME");
    }
    let _ = fs::remove_dir_all(home);
}

#[test]
fn terminating_nodes_receive_completion_and_last_turn_prompt_blocks() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-terminating");

    let (workflow_id, node_id) = create_workflow_node(
        &mut app,
        session.id(),
        "wf-scheduler-terminating",
        &agent_id,
    );
    app.sessions_mut()
        .set_workflow_node_can_complete_run(session.id(), &workflow_id, &node_id, true)
        .expect("node completion setting should update");
    app.sessions_mut()
        .set_workflow_node_max_turns(session.id(), &workflow_id, &node_id, Some(1))
        .expect("node max turns should update");
    let workflow_run = invoke_workflow_node(&mut app, session.id(), &workflow_id, &node_id);
    let node_run_id = workflow_run
        .node_runs()
        .first()
        .expect("node run should exist")
        .id()
        .to_string();
    let prompt = prepare_workflow_turn_prompt(
        &app,
        session.id(),
        workflow_run.id(),
        &node_run_id,
        &node_id,
        "start",
        Option::<&[WorkflowMessage]>::None,
    )
    .expect("prompt should build");

    assert!(prompt.contains("This node is authorized to complete the workflow run."));
    assert!(prompt.contains("This is turn 1 for this node in the current workflow run."));
    assert!(
        prompt.contains("This is the last allowed turn for this node in the current workflow run.")
    );
    assert!(prompt.contains("validate_and_submit_workflow_run_output"));
}

#[test]
fn non_last_turn_nodes_still_receive_turn_index_prompt_block() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent_id) =
        create_scheduler_session_and_agent(&mut app, "client-scheduler-turn-index");

    let (workflow_id, node_id) =
        create_workflow_node(&mut app, session.id(), "wf-scheduler-turn-index", &agent_id);
    app.sessions_mut()
        .set_workflow_node_max_turns(session.id(), &workflow_id, &node_id, Some(3))
        .expect("node max turns should update");
    let workflow_run = invoke_workflow_node(&mut app, session.id(), &workflow_id, &node_id);
    let node_run_id = workflow_run
        .node_runs()
        .first()
        .expect("node run should exist")
        .id()
        .to_string();
    let prompt = prepare_workflow_turn_prompt(
        &app,
        session.id(),
        workflow_run.id(),
        &node_run_id,
        &node_id,
        "start",
        Option::<&[WorkflowMessage]>::None,
    )
    .expect("prompt should build");

    assert!(prompt.contains("This is turn 1 for this node in the current workflow run."));
    assert!(prompt.contains("- node max turns: 3"));
    assert!(!prompt
        .contains("This is the last allowed turn for this node in the current workflow run."));
}
