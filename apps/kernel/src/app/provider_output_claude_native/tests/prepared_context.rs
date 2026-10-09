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
