//! MP-08 / MP-10 / MP-11 S09: a finished claude-headless turn settles its durable task.
use super::*;
use std::fs;

#[tokio::test]
async fn finished_headless_turn_marks_its_task_done() {
    let worktree = crate::test_support::TestWorktree::new("headless-task-settlement");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "task-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let context = worktree.path().join("context.txt");
    let events = worktree.path().join("events.jsonl");
    fs::write(&context, "").unwrap();
    fs::write(&events, "").unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        "default",
        "haiku",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "headless-task-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "headless-task".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                    context.display().to_string(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".into(),
                    events.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    spawn_inert_pty_for_run(&mut app, run.id());
    let app = Arc::new(Mutex::new(app));
    let _cleanup = InertPtyCleanup {
        app: Arc::clone(&app),
        provider_run_id: run.id().into(),
    };
    let runtime = owned_runtime_state(&app).await;
    let started = crate::app::KernelPreparedPromptSubmission {
        session_id: session.id().into(),
        prompt: crate::session::PromptQueueItem::new(
            "headless-task-prompt",
            attachment.id(),
            agent.id(),
            "do the work",
            crate::session::PromptStatus::Queued,
        ),
        force_queue: false,
        refresh_projection: true,
    };
    runtime
        .owned
        .submit_local_prepared_prompt_with_queue_policy(&started, false)
        .unwrap()
        .unwrap();
    // The pump remembers the admitted turn before the Stop hook can end it.
    runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![], true)
        .await
        .unwrap();
    let stored = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    assert!(runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&stored, agent.id())
        .is_some());
    runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![], true)
        .await
        .unwrap();
    let tasks = runtime
        .owned
        .durable_state_store
        .agent_tasks(Some(session.id()), Some(agent.id()))
        .unwrap();
    let task = tasks
        .iter()
        .find(|task| task.prompt_id == "headless-task-prompt")
        .expect("the turn's task is recorded");
    assert_eq!(
        task.state,
        crate::durable_state::agent_lifecycle::ExecutionState::Done,
        "a finished headless turn left its task for the sweep to fail"
    );
}
