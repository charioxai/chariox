// MP-08 / MP-10 / MP-11: only the dispatcher prepares a native turn's context.
use super::*;

#[test]
fn native_output_pump_does_not_skip_context_preparation() {
    let worktree = crate::test_support::TestWorktree::new("native-context-preparation");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let source = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "native-context-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let context_file = worktree.path().join("hidden-context.txt");
    fs::write(&context_file, "").unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "haiku",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "native-context-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native-context-test".into(),
            pty_target: Some("native-context-pty".into()),
            pty_program: Some("/bin/sh".into()),
            pty_args: vec!["-c".into(), "trap '' INT; exec cat".into()],
            pty_env: std::collections::BTreeMap::from([(
                "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                context_file.display().to_string(),
            )]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.pty.spawn_for_run(&run).unwrap();
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(
            session.id(),
            crate::session::PromptQueueItem::new(
                "native-context-prompt",
                source.id(),
                agent.id(),
                "continue our audit",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .unwrap()
    else {
        panic!("prompt must start")
    };
    let context = context_file.to_str().unwrap();
    let result = (|| {
        let mut bridge = ProviderOutputClaudeNativeBridge::new(&mut app);
        // The runtime pumps liveness/output before preparing the dispatch.
        bridge.inject_pending_prompt(session.id(), run.id(), agent.id(), context, &run)?;
        bridge.inject_prompt(
            session.id(),
            run.id(),
            agent.id(),
            context,
            &run,
            &ClaudeNativePromptInjection {
                id: prompt.id(),
                origin_prompt_id: prompt.id(),
                prompt: prompt.prompt(),
                hidden_system_context: "",
                attachments: &[],
                context_handoff: &|_| {
                    "<chariox_context_handoff>retained audit facts</chariox_context_handoff>".into()
                },
            },
        )
    })();
    let hidden = fs::read_to_string(&context_file).unwrap();
    app.prompt_owner_begin_cancelling_active_prompt(session.id(), agent.id())
        .unwrap();
    let waiting = format!("submit-wait:{}:0", prompt.id());
    write_claude_native_marker(context, &waiting);
    let retry = ProviderOutputClaudeNativeBridge::new(&mut app).inject_pending_prompt(
        session.id(),
        run.id(),
        agent.id(),
        context,
        &run,
    );
    let marker = claude_native_marker(context);
    app.pty.remove_process(run.id()).unwrap();
    result.unwrap();
    retry.unwrap();
    assert!(
        hidden.contains("retained audit facts"),
        "output injection bypassed the dispatcher context"
    );
    assert_eq!(
        marker.as_deref(),
        Some(waiting.as_str()),
        "cancelled delivery must not send a deferred Enter"
    );
}

#[test]
fn native_session_start_updates_identity_before_the_first_transcript_exists() {
    let worktree = crate::test_support::TestWorktree::new("native-session-start-identity");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let context = worktree.path().join("context.txt");
    let events = worktree.path().join("events.jsonl");
    fs::write(&context, "").unwrap();
    fs::write(&events, serde_json::json!({"hook_event_name":"SessionStart", "session_id":"new-native-session", "transcript_path": worktree.path().join("not-yet-created.jsonl").display().to_string()}).to_string()).unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "haiku",
    )
    .with_agent_id(agent.id())
    .with_resume_state(ProviderResumeState::from_claude_session_id(
        "old-native-session",
    ))
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "native-session-start-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native-session-start-test".into(),
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
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .unwrap();
    let observed = app.providers.get_run(run.id()).unwrap();
    assert_eq!(observed.provider_session_id(), Some("new-native-session"));
}

#[test]
fn native_session_start_survives_consumed_events() {
    let worktree = crate::test_support::TestWorktree::new("native-session-start-identity");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let context = worktree.path().join("context.txt");
    let events = worktree.path().join("events.jsonl");
    fs::write(&context, "").unwrap();
    // The event consumer already drained this batch; the hook receipt persists.
    fs::write(&events, "").unwrap();
    fs::write(
        format!("{}.session-start-id", context.display()),
        "new-native-session",
    )
    .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "haiku",
    )
    .with_agent_id(agent.id())
    .with_resume_state(ProviderResumeState::from_claude_session_id(
        "old-native-session",
    ))
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "native-session-start-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native-session-start-test".into(),
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
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .unwrap();
    let observed = app.providers.get_run(run.id()).unwrap();
    assert_eq!(observed.provider_session_id(), Some("new-native-session"));
}
