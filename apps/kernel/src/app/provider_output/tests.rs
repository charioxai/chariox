use super::*;

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
fn pump_active_prompt_outputs_skips_idle_running_arroba_provider_run() {
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
        "idle running Arroba provider runs should not keep the background pump active"
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
    assert!(
        run.terminal_diagnostic()
            .expect("timeout diagnostic should be recorded")
            .contains("Provider prompt produced no output")
    );
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
