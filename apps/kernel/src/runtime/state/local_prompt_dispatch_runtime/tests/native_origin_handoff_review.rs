//! MP-08 / MP-10 / MP-11: supplementary native hook regression through the runtime pump.
use super::*;

struct OwnNativePty {
    app: Arc<Mutex<DaemonApp>>,
    run_id: String,
}
impl Drop for OwnNativePty {
    fn drop(&mut self) {
        if let Ok(mut app) = self.app.try_lock() {
            let _ = app.pty_mut().remove_process(&self.run_id);
        }
    }
}

#[tokio::test]
async fn native_origin_first_turn_receives_handoff_once_after_session_start() {
    let root = crate::test_support::TestWorktree::new("native-origin-handoff-review");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = KernelSessionService::new(&mut app)
        .create_session(root.session_request())
        .unwrap();
    KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "native-origin-observer",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    app.agents_mut()
        .set_agent_runtime_profile_with_account_profile(
            agent.id(),
            "claude",
            Some("haiku".into()),
            None,
            Some("default".into()),
            crate::provider::ProviderResumeState::default(),
        )
        .unwrap();
    let history = app.operational_history_store();
    history
        .append(&crate::history::HistoryEvent::transcript(
            100,
            &crate::history::SessionHistoryEntry::user_prompt(
                session.id(),
                "prior",
                agent.id(),
                "Retain native-origin-amber-fact.",
            ),
            crate::history::HistoryEventTurnContext {
                prompt_id: Some("prior".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    history
        .append(&crate::history::HistoryEvent::transcript(
            101,
            &crate::history::SessionHistoryEntry::provider_output(
                session.id(),
                "old-codex-run",
                Some(agent.id()),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                None,
                "Recorded native-origin-amber-fact.",
            ),
            crate::history::HistoryEventTurnContext {
                prompt_id: Some("prior".into()),
                provider: Some("codex".into()),
                provider_session_id: Some("old-codex-session".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    let context = root.path().join("hidden-context.txt");
    let events = root.path().join("events.jsonl");
    std::fs::write(&context, "stale cached context").unwrap();
    std::fs::write(&events, [
        serde_json::json!({"hook_event_name":"SessionStart","session_id":"new-claude-session"}).to_string(),
        serde_json::json!({"hook_event_name":"UserPromptSubmit","prompt":"native continuation","hook_context_request_id":"native-first"}).to_string(),
    ].join("\n")).unwrap();
    let request = LaunchProviderRequest::new(session.id(), "claude", "claude", "default", "haiku")
        .with_agent_id(agent.id())
        .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "native-origin-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "supplementary-native-hook".into(),
            pty_target: Some("native-origin-pty".into()),
            pty_program: Some("/bin/cat".into()),
            pty_args: vec![],
            pty_env: std::collections::BTreeMap::from([
                ("CHARIOX_CLAUDE_NATIVE_CONTEXT_RECEIPTS".into(), "1".into()),
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                    context.display().to_string(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".into(),
                    events.display().to_string(),
                ),
            ]),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.pty_mut().spawn_for_run(&run).unwrap();
    crate::app::ProviderProcessTracker::new(&mut app)
        .register_managed_run(&run)
        .unwrap();
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(run.id().into()))
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let _pty_cleanup = OwnNativePty {
        app: app.clone(),
        run_id: run.id().into(),
    };
    let runtime = owned_runtime_state(&app).await;
    let outcome = runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![], true)
        .await;
    outcome
        .as_ref()
        .expect("native pump must reach its hook processing seam");
    let first =
        std::fs::read_to_string(root.path().join("hook-context-responses/native-first.txt"))
            .unwrap();
    let first_prompt = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(
            &runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap(),
            agent.id(),
        )
        .unwrap();
    let current_run = runtime.owned.provider_store.get_run(run.id()).unwrap();
    assert!(
        runtime
            .owned
            .context_handoff_for_dispatch(
                session.id(),
                agent.id(),
                &current_run,
                "retry-before-receipt",
                false,
            )
            .is_some(),
        "creating the response file prematurely consumed native context before delivery"
    );
    for (request_id, receipt_session) in [
        ("stale-native-request", "new-claude-session"),
        ("native-first", "old-claude-session"),
    ] {
        std::fs::write(
            &events,
            serde_json::json!({
                "hook_event_name":"CharioxContextDelivered",
                "hook_context_request_id":request_id, "session_id":receipt_session,
            })
            .to_string(),
        )
        .unwrap();
        runtime
            .pump_owned_provider_output(session.id(), run.id(), vec![], true)
            .await
            .unwrap();
        assert!(
            runtime
                .owned
                .context_handoff_for_dispatch(
                    session.id(),
                    agent.id(),
                    &current_run,
                    "retry-before-receipt",
                    false,
                )
                .is_some(),
            "a stale delivery receipt consumed the current native context"
        );
    }
    std::fs::write(
        &events,
        serde_json::json!({
            "hook_event_name":"CharioxContextDelivered", "hook_context_request_id":"native-first",
            "native_prompt_id":first_prompt.id(),
        })
        .to_string(),
    )
    .unwrap();
    runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![], true)
        .await
        .unwrap();
    runtime
        .owned
        .complete_local_prompt_without_advance(session.id(), agent.id(), Some(run.id()))
        .unwrap();
    std::fs::write(&events, serde_json::json!({"hook_event_name":"UserPromptSubmit","prompt":"native next turn","hook_context_request_id":"native-next"}).to_string()).unwrap();
    let next_outcome = runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![], true)
        .await;
    let next = std::fs::read_to_string(root.path().join("hook-context-responses/native-next.txt"))
        .unwrap();
    runtime
        .with_app_side_effect(|app| app.pty_mut().remove_process(run.id()))
        .await
        .unwrap();
    outcome.unwrap();
    next_outcome.unwrap();
    assert!(
        first.contains("native-origin-amber-fact"),
        "native-origin hook omitted the retained conversation"
    );
    assert!(!first.contains("stale cached context"));
    assert!(
        !next.contains("native-origin-amber-fact"),
        "accepted native handoff repeated on the following turn"
    );
    let prompts = history
        .load_session_events(session.id(), Some(agent.id()))
        .unwrap()
        .into_iter()
        .filter(|e| {
            e.kind == crate::history::HistoryEventKind::UserPrompt
                && e.prompt_id.as_deref() == Some(first_prompt.id())
        })
        .count();
    assert_eq!(prompts, 1, "native user prompt must be recorded once");
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .provider_session_id(),
        Some("new-claude-session")
    );
}
