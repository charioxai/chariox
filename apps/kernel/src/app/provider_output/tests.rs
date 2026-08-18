use super::*;

#[test]
fn promptless_pty_output_is_projected_for_failures_and_transient_native_terminals() {
    assert!(!should_project_pty_output(false, None, false));
    assert!(should_project_pty_output(true, None, false));
    assert!(should_project_pty_output(
        false,
        Some("provider credits exhausted"),
        false,
    ));
    assert!(should_project_pty_output(false, None, true));
}

#[test]
fn exited_pty_is_drained_before_liveness_settlement() {
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-exited-pty",
            "worktree-exited-pty",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-exited-pty",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        "default",
        "haiku",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-exited-pty",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-exited-pty".to_string(),
            pty_target: Some("test-exited-pty".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                "printf '%s\\n' 'Error: --dangerously-skip-permissions cannot be used with root/sudo privileges'; exit 1".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "validate provider startup",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");

    for _ in 0..50 {
        if matches!(
            app.pty.poll_process_state(run.id()),
            Ok(crate::pty::PtyProcessState::Exited)
        ) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    ProviderOutputPump::new(&mut app)
        .pump_provider_output(ProviderOutputPumpRequest {
            session_id: session.id(),
            provider_run_id: run.id(),
            recipient_attachment_ids: vec![attachment.id().to_string()],
            initial_liveness_already_checked: false,
        })
        .expect("provider output pump should preserve the terminal failure");

    let run = app
        .providers()
        .get_run(run.id())
        .expect("provider run should still exist");
    let diagnostic = run
        .terminal_diagnostic()
        .expect("provider terminal diagnostic should be recorded");
    assert!(diagnostic.contains("terminal permission error"));
    assert!(diagnostic.contains("--dangerously-skip-permissions"));
}

#[test]
fn raw_provider_output_does_not_promote_framed_reviewer_prose_to_a_terminal_error() {
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-reviewer-prose",
            "worktree-reviewer-prose",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-reviewer-prose",
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
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-reviewer-prose",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-reviewer-prose".to_string(),
            pty_target: Some("test-reviewer-prose".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                "printf '%s\\n' 'Error: this classifier is under review.' 'The phrase unsupported model is reviewer prose.'; exit 0".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "review the classifier",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");

    for _ in 0..50 {
        if matches!(
            app.pty.poll_process_state(run.id()),
            Ok(crate::pty::PtyProcessState::Exited)
        ) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    ProviderOutputPump::new(&mut app)
        .pump_provider_output(ProviderOutputPumpRequest {
            session_id: session.id(),
            provider_run_id: run.id(),
            recipient_attachment_ids: vec![attachment.id().to_string()],
            initial_liveness_already_checked: false,
        })
        .expect("reviewer output should remain ordinary provider output");

    let run = app
        .providers()
        .get_run(run.id())
        .expect("provider run should remain available");
    assert!(run.terminal_diagnostic().is_none());
}

#[test]
fn idle_claude_native_tui_projects_startup_terminal_output() {
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-idle-claude-native",
            "worktree-idle-claude-native",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-idle-claude-native",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-idle-claude-native",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-idle-claude-native".to_string(),
            pty_target: Some("test-idle-claude-native".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                "printf '\\033[?2004hClaude Code\\n'; sleep 5".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let records = loop {
        let records = ProviderOutputPump::new(&mut app)
            .pump_provider_output(ProviderOutputPumpRequest {
                session_id: session.id(),
                provider_run_id: run.id(),
                recipient_attachment_ids: vec![attachment.id().to_string()],
                initial_liveness_already_checked: false,
            })
            .expect("provider output pump should preserve the startup frame");
        if !records.is_empty() {
            break records;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the Claude native startup frame"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    app.pty
        .remove_process(run.id())
        .expect("test provider PTY should stop");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, TerminalOutputKind::ProviderTerminal);
    let output = String::from_utf8_lossy(&records[0].bytes);
    assert!(output.contains("Claude Code"));
    assert!(output.contains("\u{1b}[?2004h"));
}

fn structured_provider_test_app() -> (DaemonApp, String, String, String) {
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-structured-poll",
            "worktree-structured-poll",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-structured-poll",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "zen",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-structured-poll",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-structured-poll".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    (
        app,
        session.id().to_string(),
        attachment.id().to_string(),
        run.id().to_string(),
    )
}

fn pump_structured_test_run(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    provider_run_id: &str,
) {
    let recipients = app.attachments.list_session_attachment_ids(session_id);
    ProviderOutputPump::new(app)
        .pump_provider_output(ProviderOutputPumpRequest {
            session_id,
            provider_run_id,
            recipient_attachment_ids: recipients,
            initial_liveness_already_checked: false,
        })
        .expect("structured provider output pump should succeed");
    let _ = attachment_id;
}

#[test]
fn provider_terminal_is_transient_and_does_not_wake_meta_traces() {
    let (app, session_id, attachment_id, provider_run_id) = structured_provider_test_app();
    let session = app
        .sessions
        .get_session(&session_id)
        .expect("session should exist");
    let agent_id = app
        .providers
        .get_run(&provider_run_id)
        .expect("provider run should exist")
        .agent_instance_id()
        .expect("provider run should belong to an agent")
        .to_string();
    let trace_store = app.metaagent_trace_subscription_store();
    let subscription = trace_store.subscribe(
        &session_id,
        "meta-agent",
        &agent_id,
        crate::runtime::metaagent_trace::MetaagentTraceMode::Verbose,
    );
    let history_count = app
        .load_session_history_entries(&session, Some(&agent_id))
        .expect("history should load")
        .len();

    let record = ProviderOutputFanout::new(&app).fan_out(
        &session_id,
        &provider_run_id,
        TerminalOutputKind::ProviderTerminal,
        None,
        vec![attachment_id.clone()],
        b"\x1b[2Jfullscreen redraw",
    );

    assert_eq!(record.kind, TerminalOutputKind::ProviderTerminal);
    assert!(record.recipient_attachment_ids.contains(&attachment_id));
    assert!(!record
        .recipient_attachment_ids
        .contains(&subscription.recipient_attachment_id));
    assert_eq!(
        trace_store.target_activity_sequence(&session_id, &agent_id),
        0
    );
    assert_eq!(
        app.load_session_history_entries(&session, Some(&agent_id))
            .expect("history should still load")
            .len(),
        history_count
    );
}

fn pending_structured_output_record(
    session_id: &str,
    provider_run_id: &str,
    attachment_id: &str,
) -> TerminalOutputRecord {
    TerminalOutputRecord {
        record_id: None,
        timestamp_ms: 1_000,
        session_id: session_id.to_string(),
        provider_run_id: provider_run_id.to_string(),
        agent_id: None,
        prompt_id: None,
        prompt_origin: None,
        source_attachment_id: None,
        kind: TerminalOutputKind::ProviderOutput,
        merge_key: None,
        recipient_attachment_ids: vec![attachment_id.to_string()],
        bytes: b"completed output".to_vec(),
        pending_recipient_attachment_ids: vec![attachment_id.to_string()],
        external_observation_metadata: None,
    }
}

fn assert_pending_structured_output_drains_after_state_change(
    transition: impl FnOnce(&mut crate::provider::RuntimeProviderRun),
) {
    let (mut app, session_id, attachment_id, provider_run_id) = structured_provider_test_app();
    let expected = pending_structured_output_record(&session_id, &provider_run_id, &attachment_id);
    app.structured_output_record_store()
        .append(provider_run_id.clone(), vec![expected.clone()]);
    let mut run = app
        .providers()
        .get_run(&provider_run_id)
        .expect("provider run should exist");
    transition(&mut run);
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run);

    let records = ProviderOutputPump::new(&mut app)
        .pump_provider_output(ProviderOutputPumpRequest {
            session_id: &session_id,
            provider_run_id: &provider_run_id,
            recipient_attachment_ids: vec![attachment_id],
            initial_liveness_already_checked: true,
        })
        .expect("structured provider output pump should succeed");

    assert_eq!(records, vec![expected]);
    assert!(app
        .structured_output_record_store()
        .take(&provider_run_id)
        .is_empty());
}

#[test]
fn parked_structured_run_drains_completed_pending_output() {
    assert_pending_structured_output_drains_after_state_change(|run| run.mark_parked());
}

#[test]
fn ended_structured_run_drains_completed_pending_output() {
    assert_pending_structured_output_drains_after_state_change(|run| run.mark_ended());
}

#[test]
fn active_prompt_belongs_only_to_its_durable_delivery_provider_run() {
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-run-bound-prompt",
            "worktree-run-bound-prompt",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-run-bound-prompt",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let launch = || {
        crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "test-model",
        )
        .with_agent_id(agent.id())
    };
    let stale = app
        .launch_provider(launch())
        .expect("first provider should launch");
    let current = app
        .launch_provider(launch())
        .expect("replacement provider should launch");
    assert_eq!(stale.state(), crate::provider::ProviderRunState::Running);
    assert_eq!(
        app.providers()
            .get_run(stale.id())
            .expect("stale provider should remain addressable")
            .state(),
        crate::provider::ProviderRunState::Parked
    );

    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "review this exact revision",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start")
    else {
        panic!("prompt should become active");
    };
    app.mark_active_prompt_delivery(
        session.id(),
        agent.id(),
        prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(current.id().to_string()),
        None,
    )
    .expect("prompt delivery should bind to the replacement provider");

    assert!(!app
        .provider_run_has_active_prompt(
            session.id(),
            &app.providers()
                .get_run(stale.id())
                .expect("stale provider should resolve")
        )
        .expect("stale prompt ownership should resolve"));
    assert!(app
        .provider_run_has_active_prompt(
            session.id(),
            &app.providers()
                .get_run(current.id())
                .expect("current provider should resolve")
        )
        .expect("current prompt ownership should resolve"));
}

#[test]
fn pump_active_prompt_outputs_ignores_projected_remote_active_run() {
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    app.sessions
        .set_active_provider_run(
            session.id(),
            Some("remote-projected-provider-run-1".to_string()),
        )
        .expect("active provider run should be recorded");

    let pumped = pump_active_prompt_outputs(&mut app);

    assert!(
        pumped.is_empty(),
        "projected remote provider runs are not local PTY pump targets"
    );
}

#[test]
fn pump_active_prompt_outputs_skips_idle_running_chariox_provider_run() {
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "zen",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-idle",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-idle".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");

    let pumped = pump_active_prompt_outputs(&mut app);

    assert!(
        pumped.is_empty(),
        "idle running Chariox provider runs should not keep the background pump active"
    );
}

#[test]
fn legacy_pump_reaps_inactive_provider_turn() {
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-legacy-inactivity-timeout",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "zen",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-legacy-inactivity-timeout",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "test-opencode-timeout".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-opencode-runtime".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "start, emit a tool, then stall\n",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    crate::transport::flow_control::note_prompt_response_content(&mut app, run.id());
    app.active_turns.mark_streaming(run.id());
    if let Some(state) = app.prompt_activity.write().get_mut(run.id()) {
        state.last_output_at =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(11 * 60));
        state.saw_response_content = true;
    } else {
        panic!("prompt activity should exist for the active run");
    }

    let _ = pump_terminal_output_for_attachment(&mut app, session.id(), attachment.id())
        .expect("legacy provider output pump should reap inactive provider turn");

    let session = app
        .sessions
        .get_session(session.id())
        .expect("session should still exist");
    assert!(
        session.active_prompt_for_agent(agent.id()).is_none(),
        "legacy inactivity timeout must close the active prompt"
    );
    let run = app
        .providers
        .get_run(run.id())
        .expect("provider run should still exist");
    assert!(run
        .terminal_diagnostic()
        .expect("timeout diagnostic should be recorded")
        .contains("Provider prompt produced no output"));
}

#[test]
fn app_side_structured_pump_defers_empty_poll_reenqueue() {
    let (mut app, session_id, attachment_id, provider_run_id) = structured_provider_test_app();
    app.providers_mut()
        .push_finished_structured_output_poll_for_test(provider_run_id.clone(), Ok(None));

    pump_structured_test_run(&mut app, &session_id, &attachment_id, &provider_run_id);

    let store = app.structured_output_record_store();
    let first_due_at = store
        .poll_due_at_ms(&provider_run_id)
        .expect("empty poll should schedule a next due time");
    assert!(
        !store.poll_due(&provider_run_id, crate::session::unix_epoch_ms()),
        "empty poll should back off instead of immediately re-enqueueing"
    );

    pump_structured_test_run(&mut app, &session_id, &attachment_id, &provider_run_id);

    assert_eq!(
        store.poll_due_at_ms(&provider_run_id),
        Some(first_due_at),
        "second app-side pump before due time must not alter the poll schedule"
    );
}

#[test]
fn provider_dispatch_marks_new_workflow_prompt_before_draining_stale_completion() {
    let (mut app, session_id, attachment_id, provider_run_id) = structured_provider_test_app();
    let agent_id = app
        .providers
        .get_run(&provider_run_id)
        .expect("provider run should exist")
        .agent_instance_id()
        .expect("provider run should belong to an agent")
        .to_string();
    let workflow = app
        .sessions_mut()
        .create_workflow(&session_id, Some("stale-completion-dispatch".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(&session_id, workflow.id(), &agent_id)
        .expect("workflow node should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            &session_id,
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            &session_id,
            workflow.id(),
            endpoint.id(),
            Some("finish the workflow".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            &session_id,
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    let workflow_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        &agent_id,
        "workflow prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(&session_id, workflow_prompt, false)
        .expect("workflow prompt should start")
    else {
        panic!("workflow prompt should be active");
    };
    app.structured_output_record_store()
        .mark_poll_enqueued(&provider_run_id, Some(prompt.id().to_string()));
    app.providers_mut()
        .push_finished_structured_output_poll_for_test(
            provider_run_id.clone(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "stale-previous-turn-completion".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            })),
        );

    crate::app::ProviderPromptDispatcher::new(&mut app)
        .dispatch_prompt_to_provider(
            &session_id,
            &provider_run_id,
            prompt.id(),
            &attachment_id,
            prompt.prompt(),
            prompt.hidden_system_context(),
            prompt.attachments(),
        )
        .expect("new workflow prompt should enter provider dispatch");

    let active = app
        .prompt_owner_active_prompt_for_agent_snapshot(&session_id, &agent_id)
        .expect("active prompt should load")
        .expect("stale completion must not settle the new workflow prompt");
    assert_eq!(active.id(), prompt.id());
    assert_eq!(
        active.durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
    );
    let current_run = app
        .sessions()
        .resolve_workflow_run_ref(&session_id, workflow_run.id())
        .expect("workflow run should remain available");
    assert_eq!(
        current_run.node_runs()[0].status(),
        crate::session::WorkflowNodeRunStatus::Ready
    );
}

#[test]
fn app_side_duplicate_completion_before_promoted_workflow_dispatch_is_ignored() {
    let (mut app, session_id, attachment_id, provider_run_id) = structured_provider_test_app();
    let agent_id = app
        .providers
        .get_run(&provider_run_id)
        .expect("provider run should exist")
        .agent_instance_id()
        .expect("provider run should belong to an agent")
        .to_string();
    let direct = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        &attachment_id,
        &agent_id,
        "direct user prompt",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { .. } = app
        .prompt_owner_submit_prepared_prompt(&session_id, direct, false)
        .expect("direct prompt should start")
    else {
        panic!("direct prompt should be active");
    };
    let workflow = app
        .sessions_mut()
        .create_workflow(&session_id, Some("queued-after-user".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(&session_id, workflow.id(), &agent_id)
        .expect("workflow node should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            &session_id,
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            &session_id,
            workflow.id(),
            endpoint.id(),
            Some("finish the workflow".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            &session_id,
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    let workflow_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        &agent_id,
        "workflow prompt",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(&session_id, workflow_prompt, false)
        .expect("workflow prompt should queue")
    else {
        panic!("workflow prompt should remain queued");
    };
    app.complete_active_prompt(&session_id, &agent_id, Some(&provider_run_id))
        .expect("direct prompt should complete and promote workflow prompt");
    let promoted_prompt_id = app
        .prompt_owner_active_prompt_for_agent_snapshot(&session_id, &agent_id)
        .expect("active prompt should load")
        .expect("workflow prompt should be active")
        .id()
        .to_string();
    crate::transport::flow_control::note_prompt_started(&mut app, &provider_run_id);
    app.structured_output_record_store()
        .mark_poll_enqueued(&provider_run_id, Some(promoted_prompt_id.clone()));
    app.providers_mut()
        .push_finished_structured_output_poll_for_test(
            provider_run_id.clone(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                completions: vec![crate::provider::ProviderAssistantCompletion {
                    message_id: "stale-direct-completion".to_string(),
                    completed_at_ms: crate::session::unix_epoch_ms(),
                }],
                prompt_completed: true,
                ..crate::provider::ProviderPromptSignalBatch::default()
            })),
        );

    ProviderOutputPumpContext::new(&mut app)
        .drain_finished_structured_output_jobs_for_run(
            &session_id,
            &provider_run_id,
            vec![attachment_id.clone()],
        )
        .expect("legacy structured poll should drain");
    ProviderOutputPumpContext::new(&mut app)
        .settle_structured_prompt_completion(&session_id, &provider_run_id, true, false)
        .expect("dispatching prompt should reject duplicate completion settlement");
    std::thread::sleep(std::time::Duration::from_millis(75));
    ProviderOutputPumpContext::new(&mut app)
        .settle_structured_prompt_completion(&session_id, &provider_run_id, false, false)
        .expect("pending duplicate settlement should remain rejected before delivery");

    let active_prompt_id = app
        .prompt_owner_active_prompt_for_agent_snapshot(&session_id, &agent_id)
        .expect("active prompt should load")
        .expect("stale completion must not settle the workflow prompt")
        .id()
        .to_string();
    assert_eq!(active_prompt_id, promoted_prompt_id);
}

#[test]
fn metadata_only_structured_batch_backs_off_polling() {
    let (mut app, session_id, attachment_id, provider_run_id) = structured_provider_test_app();
    app.providers_mut()
        .push_finished_structured_output_poll_for_test(
            provider_run_id.clone(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                resolved_model: Some("resolved-zen".to_string()),
                resolved_variant: Some("plan".to_string()),
                resolved_usage_tokens_total: Some(42),
                ..crate::provider::ProviderPromptSignalBatch::default()
            })),
        );

    pump_structured_test_run(&mut app, &session_id, &attachment_id, &provider_run_id);

    let store = app.structured_output_record_store();
    assert!(
        !store.poll_due(&provider_run_id, crate::session::unix_epoch_ms()),
        "metadata-only updates should not trigger immediate re-polling"
    );
    let run = app
        .providers
        .get_run(&provider_run_id)
        .expect("provider run should still exist");
    assert_eq!(run.model(), "resolved-zen");
    assert_eq!(run.variant(), Some("plan"));
    assert_eq!(run.usage_tokens_total(), Some(42));
}

#[test]
fn structured_output_record_store_clear_removes_records_and_schedule() {
    let store = StructuredOutputRecordStore::default();
    store.schedule_next_poll("provider-run-1".to_string(), 1_500);
    store.append(
        "provider-run-1".to_string(),
        vec![TerminalOutputRecord {
            record_id: None,
            timestamp_ms: 1_000,
            session_id: "session-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            agent_id: None,
            prompt_id: None,
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: None,
            recipient_attachment_ids: Vec::new(),
            bytes: b"pending".to_vec(),
            pending_recipient_attachment_ids: Vec::new(),
            external_observation_metadata: None,
        }],
    );

    store.clear("provider-run-1");

    assert_eq!(store.poll_due_at_ms("provider-run-1"), None);
    assert!(store.take("provider-run-1").is_empty());
}
