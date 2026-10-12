//! MP-08 / MP-10 / MP-11: hook rejection settles a prompt, preserving the live native provider.
use super::*;
use std::fs;

#[tokio::test]
async fn native_context_rejection_keeps_the_run_idle_and_allows_retry() {
    rejection_scenario(false, false, false).await;
}

#[tokio::test]
async fn native_context_rejection_survives_subscription_mailbox_interleaving() {
    rejection_scenario(true, false, true).await;
}

#[tokio::test]
async fn native_context_rejection_notifies_metaagent_once() {
    rejection_scenario(false, true, false).await;
}

#[tokio::test]
async fn native_context_rejection_survives_subscription_failure_interleaving() {
    rejection_scenario(true, false, false).await;
}

async fn rejection_scenario(subscription: bool, with_meta: bool, before_submit: bool) {
    let worktree = crate::test_support::TestWorktree::new("native-context-failure");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "context-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let metaagent = if with_meta {
        let meta = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("meta"),
            )
            .unwrap();
        Some(
            app.agents_mut()
                .activate_agent_meta_mode(meta.id(), None)
                .unwrap(),
        )
    } else {
        None
    };
    let history = app.operational_history_store();
    history
        .append(&crate::history::HistoryEvent::transcript(
            1,
            &crate::history::SessionHistoryEntry::user_prompt(
                session.id(),
                attachment.id(),
                agent.id(),
                "remember amber-kestrel",
            ),
            crate::history::HistoryEventTurnContext {
                prompt_id: Some("old-prompt".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    history
        .append(&crate::history::HistoryEvent::transcript(
            2,
            &crate::history::SessionHistoryEntry::provider_output(
                session.id(),
                "old-codex-run",
                Some(agent.id()),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                None,
                "amber-kestrel recorded",
            ),
            crate::history::HistoryEventTurnContext {
                provider: Some("codex".into()),
                model: Some("gpt-6".into()),
                provider_run_id: Some("old-codex-run".into()),
                provider_session_id: Some("old-codex-thread".into()),
                prompt_id: Some("old-prompt".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    let context = worktree.path().join("context.txt");
    let events = worktree.path().join("events.jsonl");
    fs::write(&context, "").unwrap();
    fs::create_dir(worktree.path().join("hook-context-responses")).unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "haiku",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "context-failure-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native-context-test".into(),
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
                ("CHARIOX_CLAUDE_NATIVE_CONTEXT_RECEIPTS".into(), "1".into()),
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
    fs::write(&events, serde_json::json!({"hook_event_name":"UserPromptSubmit", "prompt":"first native request", "hook_context_request_id":"failed-request", "hook_context_deadline_ms":crate::session::unix_epoch_ms() + 5000}).to_string()).unwrap();
    if subscription && before_submit {
        let mut locked = app.lock().await;
        crate::runtime_transport::watch_subscription_state(
            &mut locked,
            session.id(),
            attachment.id(),
            false,
            None,
            0,
        );
        assert!(
            !fs::read_to_string(&events).unwrap().is_empty(),
            "subscription consumed the native hook mailbox"
        );
    }
    runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![], true)
        .await
        .unwrap();
    let hidden = fs::read_to_string(&context).unwrap();
    assert!(
        hidden.contains("amber-kestrel"),
        "native prompt lost the conversation handoff"
    );
    if subscription {
        let mut locked = app.lock().await;
        crate::runtime_transport::watch_subscription_state(
            &mut locked,
            session.id(),
            attachment.id(),
            false,
            None,
            0,
        );
    }
    assert!(runtime
        .owned
        .session_snapshot(session.id())
        .unwrap()
        .active_prompt_for_agent(agent.id())
        .is_some());
    fs::write(&events, serde_json::json!({"hook_event_name":"CharioxContextFailed", "hook_context_request_id":"failed-request"}).to_string()).unwrap();
    if subscription {
        let mut locked = app.lock().await;
        crate::runtime_transport::watch_subscription_state(
            &mut locked,
            session.id(),
            attachment.id(),
            false,
            None,
            0,
        );
        assert!(
            !fs::read_to_string(&events).unwrap().is_empty(),
            "subscription consumed the native hook mailbox"
        );
    }
    runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![], true)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(run.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Running,
        "a blocked hook killed healthy Claude"
    );
    assert!(runtime
        .owned
        .session_snapshot(session.id())
        .unwrap()
        .active_prompt_for_agent(agent.id())
        .is_none());
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .unwrap()
            .state(),
        crate::agent::AgentState::Idle
    );
    if let Some(meta) = &metaagent {
        let failures =
            runtime
                .owned
                .metaagent_events
                .list(meta.id(), Some("agent.turn.failed"), None, 10);
        assert_eq!(
            failures.len(),
            1,
            "hook rejection must notify the Meta agent"
        );
        assert_eq!(failures[0].source_agent_id.as_deref(), Some(agent.id()));
        assert!(failures[0].summary.contains("context delivery failed"));
        // A duplicated receipt must not emit another failure.
        fs::write(&events, serde_json::json!({"hook_event_name":"CharioxContextFailed", "hook_context_request_id":"failed-request"}).to_string()).unwrap();
        runtime
            .pump_owned_provider_output(session.id(), run.id(), vec![], true)
            .await
            .unwrap();
        assert_eq!(
            runtime
                .owned
                .metaagent_events
                .list(meta.id(), Some("agent.turn.failed"), None, 10)
                .len(),
            1
        );
    }
    fs::write(&events, serde_json::json!({"hook_event_name":"UserPromptSubmit", "prompt":"retry native request", "hook_context_request_id":"retry-request", "hook_context_deadline_ms":crate::session::unix_epoch_ms() + 5000}).to_string()).unwrap();
    runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![], true)
        .await
        .unwrap();
    assert!(runtime
        .owned
        .session_snapshot(session.id())
        .unwrap()
        .active_prompt_for_agent(agent.id())
        .is_some());
}
