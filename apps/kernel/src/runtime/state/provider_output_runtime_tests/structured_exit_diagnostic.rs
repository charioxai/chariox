use super::*;

const SAFE_DIAGNOSTIC_CONTEXT: &str = "safe-codex-startup-context";
const SYNTHETIC_API_KEY: &str = "sk-synthetic-structured-exit-secret";
const SYNTHETIC_TOKEN: &str = "synthetic-structured-exit-token";

#[tokio::test]
async fn managed_structured_codex_exit_preserves_redacted_diagnostic_without_replay() {
    let mut app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests())
        .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-structured-exit-diagnostic",
            "worktree-structured-exit-diagnostic",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-structured-exit-diagnostic",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-5.4",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-structured-exit-diagnostic",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "codex:app-server-test".to_string(),
            pty_target: None,
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                format!(
                    "printf '%s\\n' 'Codex startup diagnostic: {SAFE_DIAGNOSTIC_CONTEXT} api_key={SYNTHETIC_API_KEY} token={SYNTHETIC_TOKEN}' >&2; sleep 0.1; exit 1"
                ),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("ws://127.0.0.1:45000".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    crate::app::ProviderLaunchProcessRuntime::new(&mut app)
        .spawn_for_launch(&run)
        .expect("managed Codex-shaped PTY should start");

    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(
            session.id(),
            crate::session::PromptQueueItem::new(
                "prompt-structured-exit-diagnostic",
                attachment.id(),
                agent.id(),
                "do not replay this prompt",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .expect("prompt should start")
    else {
        panic!("the diagnostic regression prompt should start immediately");
    };
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if matches!(
            app.pty_mut().poll_process_state(run.id()),
            Ok(crate::pty::PtyProcessState::Exited {
                exit_code: Some(1),
                signal: None,
            })
        ) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the managed Codex-shaped process did not exit with status 1"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .record_started_user_prompt(session.id(), attachment.id(), &prompt)
        .expect("started prompt should enter canonical history");

    runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            false,
        )
        .await
        .expect("owned provider lifecycle should settle the exit");

    let provider_run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should remain observable");
    let diagnostic = provider_run
        .terminal_diagnostic()
        .expect("structured PTY diagnostic should survive liveness settlement");
    assert!(diagnostic.contains(SAFE_DIAGNOSTIC_CONTEXT), "{diagnostic}");
    assert!(!diagnostic.contains(SYNTHETIC_API_KEY), "{diagnostic}");
    assert!(!diagnostic.contains(SYNTHETIC_TOKEN), "{diagnostic}");

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should remain available");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "the failed prompt must settle without being replayed"
    );
    let activity = runtime.agent_activity_for_session(&session_state);
    let termination = activity
        .get(agent.id())
        .and_then(|activity| activity.last_completed_turn.as_ref())
        .and_then(|turn| turn.provider_termination.as_ref())
        .expect("failed turn should retain typed process termination");
    assert_eq!(
        termination.category,
        crate::provider::ProviderRunTerminationCategory::ProcessExit
    );
    assert_eq!(termination.reason, "provider process exited with status 1");

    let terminal_records = runtime
        .owned
        .terminal_stream
        .drain_output_records(session.id(), attachment.id());
    let terminal_errors = terminal_records
        .iter()
        .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderError)
        .collect::<Vec<_>>();
    assert_eq!(terminal_errors.len(), 1, "{terminal_records:?}");
    let terminal_error = String::from_utf8_lossy(&terminal_errors[0].bytes);
    assert!(terminal_error.contains(SAFE_DIAGNOSTIC_CONTEXT), "{terminal_error}");
    assert!(!terminal_error.contains(SYNTHETIC_API_KEY), "{terminal_error}");
    assert!(!terminal_error.contains(SYNTHETIC_TOKEN), "{terminal_error}");

    let history = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .expect("operational history should load");
    let user_prompts = history
        .iter()
        .filter(|event| event.kind == crate::history::HistoryEventKind::UserPrompt)
        .collect::<Vec<_>>();
    assert_eq!(user_prompts.len(), 1, "the prompt must not be replayed: {history:?}");
    let provider_errors = history
        .iter()
        .filter(|event| event.kind == crate::history::HistoryEventKind::ProviderError)
        .collect::<Vec<_>>();
    assert_eq!(provider_errors.len(), 1, "{history:?}");
    let provider_error = provider_errors[0]
        .content
        .as_deref()
        .expect("provider error history should have content");
    assert!(provider_error.contains(SAFE_DIAGNOSTIC_CONTEXT), "{provider_error}");
    assert!(!provider_error.contains(SYNTHETIC_API_KEY), "{provider_error}");
    assert!(!provider_error.contains(SYNTHETIC_TOKEN), "{provider_error}");

    runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            false,
        )
        .await
        .expect("repeated liveness observation should be idempotent");
    let repeated_history = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .expect("repeated history should load");
    assert_eq!(
        repeated_history
            .iter()
            .filter(|event| event.kind == crate::history::HistoryEventKind::UserPrompt)
            .count(),
        1,
        "repeated exit observation must not replay the prompt"
    );
    assert_eq!(
        repeated_history
            .iter()
            .filter(|event| event.kind == crate::history::HistoryEventKind::ProviderError)
            .count(),
        1,
        "repeated exit observation must not duplicate the diagnostic"
    );
}
