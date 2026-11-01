//! MP-08 / MP-10 / MP-11: startup trust settlement runs through the mailbox owner.
use super::*;
use crate::app::{startup_readiness_run, StartupTrustBridge};
use crate::provider::ProviderNativeInteractionBridge;
use std::fs;

#[test]
fn claude_workspace_trust_rejection_settles_only_own_prompt_with_reason() {
    let worktree = crate::test_support::TestWorktree::new("claude-native-trust-rejection");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-startup-trust-rejection-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-startup-trust-rejection",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let (other_session, _other_default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-unrelated-agent",
            root.display().to_string(),
        ))
        .expect("unrelated session should be created");
    let other_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(other_session.id(), "dev-stub")
                .with_alias("unrelated-test-agent")
                .with_model("test-model"),
        )
        .expect("self-contained unrelated test agent should be created");
    let other_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            other_session.id(),
            "client-unrelated-agent",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("unrelated attachment should attach");
    app.record_native_prompt_started_with_attachments(
        other_session.id(),
        other_attachment.id(),
        other_attachment.id(),
        other_agent.id(),
        "unrelated-agent-task",
        Vec::new(),
    )
    .expect("unrelated agent prompt should remain active");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let capture_file = root.join("pty-input.log");
    let trust_frame =
        "Quick safety check\nDo you trust this folder?\n1. No, exit\n2. Yes, I trust this folder";
    let mut run = startup_readiness_run(
        session.id(),
        agent.id(),
        "provider-run-startup-trust-rejection",
        &context_file,
        &events_file,
        format!(
            "printf '%s' '{}'; tee {} >/dev/null",
            trust_frame,
            capture_file.display()
        ),
    );
    run.mark_running();
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    let prompt = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "rejected-startup-trust-task",
            Vec::new(),
        )
        .expect("rejected task should become active")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt outcome: {other:?}"),
    };
    // The replacement run is a stub process: a real Claude launch needs a login.
    app.agents_mut()
        .set_agent_runtime_profile_with_account_profile(
            agent.id(),
            "dev-stub",
            None,
            None,
            None,
            crate::provider::ProviderResumeState::default(),
        )
        .expect("the agent should switch to the stub provider");
    let queued = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "queued-behind-startup-trust-task",
            Vec::new(),
        )
        .expect("a second task should queue behind the trust question")
    {
        crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
        other => panic!("unexpected prompt outcome: {other:?}"),
    };
    let bridge = StartupTrustBridge::default();
    let bridge_ref: std::sync::Arc<dyn ProviderNativeInteractionBridge> =
        std::sync::Arc::new(bridge.clone());
    app.providers()
        .set_native_interaction_bridge(bridge_ref.clone());
    let app = Arc::new(Mutex::new(app));
    let _cleanup = InertPtyCleanup {
        app: Arc::clone(&app),
        provider_run_id: run.id().into(),
    };
    let executor = tokio::runtime::Runtime::new().unwrap();
    let runtime = executor.block_on(owned_runtime_state(&app));
    let interaction = bridge.wait_for_interaction_with_pump(|| {
        executor
            .block_on(runtime.pump_owned_provider_output(
                session.id(),
                run.id(),
                vec![attachment.id().to_string()],
                false,
            ))
            .expect("trust prompt should be projected by the kernel runtime pump");
    });
    assert_eq!(interaction.default_on_timeout(), Some("deny"));
    bridge.resolve_default_no();
    let mut settled = false;
    for _ in 0..500 {
        executor
            .block_on(runtime.pump_owned_provider_output(
                session.id(),
                run.id(),
                vec![attachment.id().to_string()],
                false,
            ))
            .expect("trust rejection should use the kernel runtime failure path");
        let primary_active = runtime
            .owned
            .session_snapshot(session.id())
            .unwrap()
            .active_prompt_for_agent(agent.id())
            .cloned();
        if primary_active.is_none_or(|active| active.id() != prompt.id()) {
            settled = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        settled,
        "rejected startup trust should settle its own prompt"
    );
    assert!(runtime
        .owned
        .session_snapshot(other_session.id())
        .unwrap()
        .active_prompt_for_agent(other_agent.id())
        .is_some());
    let rejected_run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("rejected provider run should remain inspectable");
    let diagnostic = rejected_run
        .terminal_diagnostic()
        .expect("rejection should preserve a terminal diagnostic");
    assert!(
        diagnostic.contains("workspace trust was denied"),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains(root.to_string_lossy().as_ref()));
    // The person sees the calm stopped state: no error row, the turn cancelled.
    let error_rows = runtime
        .owned
        .terminal_stream
        .output_records()
        .into_iter()
        .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderError)
        .map(|record| String::from_utf8_lossy(&record.bytes).into_owned())
        .collect::<Vec<_>>();
    assert!(error_rows.is_empty(), "{error_rows:?}");
    let settlement = runtime
        .owned
        .operational_history_store
        .load_prompt_settlement_event(session.id(), agent.id(), prompt.id())
        .unwrap()
        .expect("the declined turn settles");
    assert_eq!(
        settlement
            .metadata
            .get(crate::history::TURN_OUTCOME_METADATA_KEY),
        Some(&serde_json::to_value(crate::provider::TurnOutcome::cancelled()).unwrap())
    );
    assert_queued_prompt_runs_on_replacement(
        &runtime,
        session.id(),
        agent.id(),
        run.id(),
        queued.id(),
    );
    assert!(!fs::read_to_string(&capture_file)
        .unwrap_or_default()
        .contains("rejected-startup-trust-task"));
    let _ = executor
        .block_on(app.lock())
        .pty_mut()
        .remove_process(run.id());
    let _ = fs::remove_dir_all(root);
}

/// A prompt queued behind a denied trust question moves to a fresh provider run: the denied run
/// is over, so the queued prompt must not stay active on it.
fn assert_queued_prompt_runs_on_replacement(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    denied_run_id: &str,
    queued_prompt_id: &str,
) {
    let denied = runtime.owned.provider_store.get_run(denied_run_id).unwrap();
    assert_ne!(denied.state(), crate::provider::ProviderRunState::Running);
    let active = runtime
        .owned
        .session_snapshot(session_id)
        .unwrap()
        .active_prompt_for_agent(agent_id)
        .cloned()
        .expect("the queued prompt becomes the active prompt");
    assert_eq!(active.id(), queued_prompt_id);
    let replacement = runtime
        .owned
        .provider_store
        .get_run_for_agent(session_id, agent_id)
        .expect("the queued prompt has a provider run");
    assert_ne!(replacement.id(), denied_run_id);
    assert_eq!(
        replacement.state(),
        crate::provider::ProviderRunState::Running
    );
}

#[test]
fn a_trust_question_declined_during_dispatch_starts_the_queued_prompt_on_a_fresh_run() {
    declined_dispatch_with_queued_successor(false);
}

#[test]
fn a_declined_trust_question_starts_a_queued_workflow_node_on_a_workflow_capable_run() {
    declined_dispatch_with_queued_successor(true);
}

fn declined_dispatch_with_queued_successor(workflow_successor: bool) {
    let worktree = crate::test_support::TestWorktree::new("claude-native-trust-dispatch-denied");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-trust-dispatch-denied-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-trust-dispatch-denied",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let workflow_run = workflow_successor.then(|| {
        // The node's run exists before the ordinary turn takes the agent; only the node queues.
        app.agents_mut()
            .set_agent_runtime_profile_with_account_profile(
                agent.id(),
                "dev-stub",
                None,
                None,
                None,
                crate::provider::ProviderResumeState::default(),
            )
            .expect("the agent should switch to the stub provider");
        let workflow_id = app
            .sessions_mut()
            .create_workflow(session.id(), Some("wf-trust-denied".to_string()))
            .expect("workflow should exist")
            .id()
            .to_string();
        let node_id = app
            .sessions_mut()
            .add_workflow_node(session.id(), &workflow_id, agent.id())
            .expect("workflow node should be added")
            .id()
            .to_string();
        app.sessions_mut()
            .create_workflow_endpoint(session.id(), &workflow_id, &node_id, Some("entry".into()))
            .expect("workflow endpoint should exist");
        let (workflow_run, _, _) = app
            .invoke_workflow_endpoint_and_schedule(
                session.id(),
                &workflow_id,
                "entry",
                Some("start".to_string()),
            )
            .expect("workflow should invoke");
        app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
            .expect("the first node prompt should settle");
        app.end_agent_provider_run(session.id(), agent.id())
            .expect("the first node run should end");
        workflow_run
    });
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let mut run = startup_readiness_run(
        session.id(),
        agent.id(),
        "provider-run-trust-dispatch-denied",
        &context_file,
        &events_file,
        "sleep 30".to_string(),
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    // The replacement run is a stub process: a real Claude launch needs a login.
    app.agents_mut()
        .set_agent_runtime_profile_with_account_profile(
            agent.id(),
            "dev-stub",
            None,
            None,
            None,
            crate::provider::ProviderResumeState::default(),
        )
        .expect("the agent should switch to the stub provider");
    let mut submit = |text: &str| {
        app.record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            text,
            Vec::new(),
        )
        .expect("prompt should be accepted")
    };
    let crate::session::PromptSubmissionOutcome::Started { prompt } = submit("denied-task") else {
        panic!("the first task should start");
    };
    let queued = if let Some(workflow_run) = &workflow_run {
        // A workflow node waits behind the ordinary turn that raised the trust question.
        let node_prompt = crate::session::PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            agent.id(),
            "workflow node task",
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run.id(), workflow_run.node_runs()[0].id());
        let crate::session::PromptSubmissionOutcome::Queued { prompt: queued } = app
            .prompt_owner_submit_prepared_prompt(session.id(), node_prompt, false)
            .expect("the workflow node should queue")
        else {
            panic!("the workflow node should queue behind the ordinary turn");
        };
        queued
    } else {
        let crate::session::PromptSubmissionOutcome::Queued { prompt: queued } =
            submit("queued-task")
        else {
            panic!("the second task should queue");
        };
        queued
    };
    let app = Arc::new(Mutex::new(app));
    let _cleanup = InertPtyCleanup {
        app: Arc::clone(&app),
        provider_run_id: run.id().into(),
    };
    let executor = tokio::runtime::Runtime::new().unwrap();
    let runtime = executor.block_on(owned_runtime_state(&app));
    let dispatch = crate::app::KernelPromptDispatch {
        session_id: session.id().to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: agent.id().to_string(),
        prompt_id: prompt.id().to_string(),
        target_active_prompt_id: None,
        source_attachment_id: attachment.id().to_string(),
        prompt: "denied-task".to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        prompt_origin: prompt.prompt_origin(),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: false,
    };
    let denied = DaemonError::ProviderProtocol {
        provider_run_id: run.id().to_string(),
        operation: "submit Claude headless prompt",
        message: "Claude Code workspace trust was denied before the queued task was dispatched."
            .to_string(),
    };
    executor
        .block_on(runtime.fail_prompt_dispatch(dispatch, denied))
        .expect("a declined trust question ends the turn without an error");
    assert_queued_prompt_runs_on_replacement(
        &runtime,
        session.id(),
        agent.id(),
        run.id(),
        queued.id(),
    );
    if workflow_successor {
        let replacement = runtime
            .owned
            .provider_store
            .get_run_for_agent(session.id(), agent.id())
            .expect("the queued node has a provider run");
        assert!(
            replacement.workflow_tools_enabled(),
            "a workflow node must start on a workflow-capable run"
        );
    }
    let _ = fs::remove_dir_all(root);
}
