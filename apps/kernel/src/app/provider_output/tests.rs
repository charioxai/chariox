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
