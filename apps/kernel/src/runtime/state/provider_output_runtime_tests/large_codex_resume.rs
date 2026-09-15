use super::*;

fn external_codex_run(
    session_id: &str,
    agent_id: &str,
    provider_run_id: &str,
) -> crate::provider::RuntimeProviderRun {
    let request = crate::provider::LaunchProviderRequest::new(
        session_id, "codex", "codex", "default", "gpt-5.6",
    )
    .with_agent_id(agent_id);
    let mut run = crate::provider::RuntimeProviderRun::new(
        provider_run_id,
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: format!("test-codex-{provider_run_id}"),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some(format!("test-codex-runtime-{provider_run_id}")),
        },
    );
    run.mark_running();
    run
}

fn submit_prompt_for_agent(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    agent_id: &str,
    prompt_text: &str,
) -> String {
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment_id,
        agent_id,
        prompt_text,
        crate::session::PromptStatus::Queued,
    );
    match app
        .prompt_owner_submit_prepared_prompt(session_id, prompt, false)
        .expect("prompt should start")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
        crate::session::PromptSubmissionOutcome::Queued { .. } => {
            panic!("prompt should start immediately")
        }
    }
}

#[tokio::test]
async fn oversized_codex_poll_failure_settles_failed_run_without_touching_healthy_run() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, failed_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-large-codex-resume",
            "worktree-large-codex-resume",
        ))
        .expect("session should be created");
    let healthy_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex")
                .with_alias("healthy-codex"),
        )
        .expect("healthy agent should be spawned");
    let failed_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-large-codex-failed",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("failed-run attachment should attach");
    let healthy_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-large-codex-healthy",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("healthy-run attachment should attach");

    let failed_run = external_codex_run(
        session.id(),
        failed_agent.id(),
        "provider-run-large-codex-failed",
    );
    let healthy_run = external_codex_run(
        session.id(),
        healthy_agent.id(),
        "provider-run-large-codex-healthy",
    );
    app.providers_mut().insert_run_for_test(failed_run.clone());
    app.providers_mut().insert_run_for_test(healthy_run.clone());
    app.update_provider_run_projection(failed_run.clone());
    app.update_provider_run_projection(healthy_run.clone());

    let failed_prompt_id = submit_prompt_for_agent(
        &mut app,
        session.id(),
        failed_attachment.id(),
        failed_agent.id(),
        "resume the large Codex history",
    );
    let healthy_prompt_id = submit_prompt_for_agent(
        &mut app,
        session.id(),
        healthy_attachment.id(),
        healthy_agent.id(),
        "keep the healthy Codex turn alive",
    );
    crate::transport::flow_control::note_prompt_started(&mut app, failed_run.id());
    crate::transport::flow_control::note_prompt_started(&mut app, healthy_run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let output_store = runtime.owned.structured_output_records.clone();
    // This is the exact bounded tungstenite error observed after a Codex resume history frame
    // exceeded the 16 MiB frame limit. The transport-level test supplies the real WebSocket
    // source of this error; this test exercises the shared background settlement path with a
    // healthy concurrent run still owned by the same runtime.
    let oversized_error_message = "Space limit exceeded: Message too long: 23622205 > 16777216";

    output_store.mark_poll_enqueued(healthy_run.id(), Some(healthy_prompt_id.clone()));
    for attempt in 1..=crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
        output_store.mark_poll_enqueued(failed_run.id(), Some(failed_prompt_id.clone()));
        app.lock()
            .await
            .providers_mut()
            .push_finished_structured_output_poll_for_test(
                failed_run.id().to_string(),
                Err(crate::error::DaemonError::ProviderProtocol {
                    provider_run_id: failed_run.id().to_string(),
                    operation: "codex_read",
                    message: oversized_error_message.to_string(),
                }),
            );

        let result = runtime
            .pump_owned_structured_provider_output(
                session.id(),
                healthy_run.id(),
                vec![healthy_attachment.id().to_string()],
            )
            .await;
        assert!(
            result.is_ok(),
            "a failed background poll must not fail a healthy requested run on attempt {attempt}: {result:?}"
        );
    }

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state
            .active_prompt_for_agent(failed_agent.id())
            .is_none(),
        "repeated oversized Codex poll failure must not leave its prompt active"
    );
    assert!(
        session_state
            .active_prompt_for_agent(healthy_agent.id())
            .is_some_and(|prompt| prompt.id() == healthy_prompt_id),
        "the healthy concurrent Codex prompt must remain active"
    );

    let activity = runtime.agent_activity_for_session(&session_state);
    let failed_activity = activity
        .get(failed_agent.id())
        .expect("failed agent activity should be projected");
    assert_eq!(
        failed_activity.status,
        crate::runtime::projection::AgentRuntimeStatus::Error,
        "a polling-abandoned run must not remain projected as working"
    );
    assert_eq!(
        failed_activity
            .last_completed_turn
            .as_ref()
            .expect("failed turn should remain visible")
            .settlement_status,
        crate::git_observer::CompletedTurnSettlementStatus::Failed
    );
    assert_eq!(
        failed_activity
            .last_completed_turn
            .as_ref()
            .and_then(|turn| turn.provider_termination.as_ref())
            .map(|termination| termination.category),
        Some(crate::provider::ProviderRunTerminationCategory::ExplicitProviderError),
        "an oversized poll must settle as an explicit provider error, not as silence"
    );
    let healthy_activity = activity
        .get(healthy_agent.id())
        .expect("healthy agent activity should be projected");
    assert_eq!(
        healthy_activity.status,
        crate::runtime::projection::AgentRuntimeStatus::Working
    );
    assert_eq!(healthy_activity.active_prompt_count, 1);

    let failed_run_after = runtime
        .owned
        .provider_store
        .get_run(failed_run.id())
        .expect("failed provider run should remain inspectable");
    assert_eq!(
        failed_run_after.state(),
        crate::provider::ProviderRunState::Ended
    );
    assert!(
        failed_run_after
            .terminal_diagnostic()
            .is_some_and(|diagnostic| diagnostic.contains("Message too long")),
        "the failed run must retain the actionable oversized-frame diagnostic"
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(healthy_run.id())
            .expect("healthy provider run should remain inspectable")
            .state(),
        crate::provider::ProviderRunState::Running
    );
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(healthy_run.id())
            .expect("healthy provider run should remain inspectable")
            .terminal_diagnostic(),
        None
    );
}

#[tokio::test]
async fn stale_oversized_codex_poll_cannot_settle_replacement_prompt() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-large-codex-stale-poll",
            "worktree-large-codex-stale-poll",
        ))
        .expect("session should be created");
    let healthy_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex")
                .with_alias("healthy-stale-poll-codex"),
        )
        .expect("healthy agent should be spawned");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-large-codex-stale-poll",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let healthy_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-large-codex-stale-poll-healthy",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("healthy attachment should attach");
    let run = external_codex_run(
        session.id(),
        agent.id(),
        "provider-run-large-codex-stale-poll",
    );
    let healthy_run = external_codex_run(
        session.id(),
        healthy_agent.id(),
        "provider-run-large-codex-stale-poll-healthy",
    );
    app.providers_mut().insert_run_for_test(run.clone());
    app.providers_mut().insert_run_for_test(healthy_run.clone());
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    app.update_provider_run_projection(healthy_run.clone());

    let old_prompt_id = submit_prompt_for_agent(
        &mut app,
        session.id(),
        attachment.id(),
        agent.id(),
        "start the original Codex resume",
    );
    let healthy_prompt_id = submit_prompt_for_agent(
        &mut app,
        session.id(),
        healthy_attachment.id(),
        healthy_agent.id(),
        "keep the healthy stale-poll run alive",
    );
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    crate::transport::flow_control::note_prompt_started(&mut app, healthy_run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let output_store = runtime.owned.structured_output_records.clone();
    let oversized_error_message = "Space limit exceeded: Message too long: 23600105 > 16777216";
    let mut replacement_prompt_id = None;
    let mut history_before = None;
    output_store.mark_poll_enqueued(healthy_run.id(), Some(healthy_prompt_id));

    for attempt in 1..=crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
        output_store.mark_poll_enqueued(run.id(), Some(old_prompt_id.clone()));
        app.lock()
            .await
            .providers_mut()
            .push_finished_structured_output_poll_for_test(
                run.id().to_string(),
                Err(crate::error::DaemonError::ProviderProtocol {
                    provider_run_id: run.id().to_string(),
                    operation: "codex_read",
                    message: oversized_error_message.to_string(),
                }),
            );

        if attempt == crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
            let mut app = app.lock().await;
            app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
                .expect("the original prompt should be replaceable");
            let replacement = submit_prompt_for_agent(
                &mut app,
                session.id(),
                attachment.id(),
                agent.id(),
                "continue with the replacement Codex prompt",
            );
            app.mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                &replacement,
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some(run.id().to_string()),
                run.provider_session_id().map(str::to_string),
            )
            .expect("replacement prompt should be delivered to the same provider run");
            history_before = Some(
                app.operational_history_store()
                    .load_session_events(session.id(), Some(agent.id()))
                    .expect("operational history should load"),
            );
            replacement_prompt_id = Some(replacement);
        }

        runtime
            .pump_owned_structured_provider_output(
                session.id(),
                healthy_run.id(),
                vec![healthy_attachment.id().to_string()],
            )
            .await
            .expect("a stale background poll must not fail the replacement prompt");
    }

    let replacement_prompt_id = replacement_prompt_id.expect("replacement prompt should exist");
    let history_before = history_before.expect("replacement history snapshot should exist");
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert_eq!(
        session_state
            .active_prompt_for_agent(agent.id())
            .expect("replacement prompt should remain active")
            .id(),
        replacement_prompt_id,
        "a late oversized poll for the old prompt must not settle its replacement"
    );
    let run_after = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should remain inspectable");
    assert_eq!(
        run_after.state(),
        crate::provider::ProviderRunState::Running,
        "a stale poll must not retire the provider run serving the replacement"
    );
    assert_eq!(
        run_after.terminal_diagnostic(),
        None,
        "a stale poll must not write a diagnostic onto the replacement run"
    );
    let projected_after = runtime
        .owned
        .provider_run_projection
        .get(run.id())
        .expect("provider projection should remain available");
    assert_eq!(
        projected_after.state(),
        crate::provider::ProviderRunState::Running
    );
    assert_eq!(projected_after.terminal_diagnostic(), None);
    let history_after = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .expect("operational history should load after stale poll");
    assert_eq!(
        history_after, history_before,
        "a stale poll must not append failure history for the replacement prompt"
    );

    assert!(
        output_store.poll_due(run.id(), u64::MAX),
        "a stale poll failure must not exhaust the replacement prompt's poll budget"
    );
    output_store.mark_poll_enqueued(run.id(), Some(replacement_prompt_id.clone()));
    let replacement_output = b"replacement Codex poll output".to_vec();
    app.lock()
        .await
        .providers_mut()
        .push_finished_structured_output_poll_for_test(
            run.id().to_string(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("replacement-codex-poll".to_string()),
                    bytes: replacement_output.clone(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            })),
        );
    let records = runtime
        .pump_owned_structured_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
        )
        .await
        .expect("the replacement poll should be admitted and delivered");
    assert_eq!(
        records
            .iter()
            .map(|record| record.bytes.clone())
            .collect::<Vec<_>>(),
        vec![replacement_output],
        "the replacement prompt must receive output after the stale poll"
    );
}

#[tokio::test]
async fn promptless_codex_poll_failure_reschedules_bound_prompt_and_delivers_output() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-large-codex-promptless-poll",
            "worktree-large-codex-promptless-poll",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-large-codex-promptless-poll",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = external_codex_run(
        session.id(),
        agent.id(),
        "provider-run-large-codex-promptless-poll",
    );
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let output_store = runtime.owned.structured_output_records.clone();
    let mut prompt_id = None;

    for attempt in 1..=crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
        output_store.mark_poll_enqueued(run.id(), None);
        app.lock()
            .await
            .providers_mut()
            .push_finished_structured_output_poll_for_test(
                run.id().to_string(),
                Err(crate::error::DaemonError::ProviderProtocol {
                    provider_run_id: run.id().to_string(),
                    operation: "codex_read",
                    message: "promptless poll failed before prompt ownership was observed"
                        .to_string(),
                }),
            );

        if attempt == crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
            let mut app = app.lock().await;
            let replacement = submit_prompt_for_agent(
                &mut app,
                session.id(),
                attachment.id(),
                agent.id(),
                "start the prompt after an idle poll began",
            );
            app.mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                &replacement,
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some(run.id().to_string()),
                run.provider_session_id().map(str::to_string),
            )
            .expect("the current prompt should be bound to the same provider run");
            prompt_id = Some(replacement);
        }

        runtime
            .pump_owned_structured_provider_output(
                session.id(),
                run.id(),
                vec![attachment.id().to_string()],
            )
            .await
            .expect("a promptless background poll failure should remain recoverable");
    }

    let prompt_id = prompt_id.expect("the current prompt should exist");
    assert!(
        output_store.poll_due(run.id(), u64::MAX),
        "a promptless failure must not exhaust the newly bound prompt's poll budget"
    );
    output_store.mark_poll_enqueued(run.id(), Some(prompt_id));
    let replacement_output = b"promptless poll replacement output".to_vec();
    app.lock()
        .await
        .providers_mut()
        .push_finished_structured_output_poll_for_test(
            run.id().to_string(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("promptless-codex-poll".to_string()),
                    bytes: replacement_output.clone(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            })),
        );
    let records = runtime
        .pump_owned_structured_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
        )
        .await
        .expect("the newly bound prompt poll should be delivered");
    assert_eq!(
        records
            .iter()
            .map(|record| record.bytes.clone())
            .collect::<Vec<_>>(),
        vec![replacement_output],
        "the newly bound prompt must receive output after promptless poll failures"
    );
}

#[tokio::test]
async fn promptless_codex_poll_failure_before_prompt_start_reschedules_and_delivers_output() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-large-codex-promptless-before-start",
            "worktree-large-codex-promptless-before-start",
        ))
        .expect("session should be created");
    let healthy_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex")
                .with_alias("healthy-promptless-before-start"),
        )
        .expect("healthy agent should be spawned");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-promptless-before-start",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let healthy_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-promptless-before-start-healthy",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("healthy attachment should attach");
    let run = external_codex_run(
        session.id(),
        agent.id(),
        "provider-run-promptless-before-start",
    );
    let healthy_run = external_codex_run(
        session.id(),
        healthy_agent.id(),
        "provider-run-promptless-before-start-healthy",
    );
    app.providers_mut().insert_run_for_test(run.clone());
    app.providers_mut().insert_run_for_test(healthy_run.clone());
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    app.update_provider_run_projection(healthy_run.clone());

    let healthy_prompt_id = submit_prompt_for_agent(
        &mut app,
        session.id(),
        healthy_attachment.id(),
        healthy_agent.id(),
        "keep the concurrent prompt alive while the first run is idle",
    );
    crate::transport::flow_control::note_prompt_started(&mut app, healthy_run.id());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let output_store = runtime.owned.structured_output_records.clone();
    output_store.mark_poll_enqueued(healthy_run.id(), Some(healthy_prompt_id));
    let mut replacement_prompt_id = None;

    for attempt in 1..=crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
        output_store.mark_poll_enqueued(run.id(), None);
        app.lock()
            .await
            .providers_mut()
            .push_finished_structured_output_poll_for_test(
                run.id().to_string(),
                Err(crate::error::DaemonError::ProviderProtocol {
                    provider_run_id: run.id().to_string(),
                    operation: "codex_read",
                    message: "promptless poll failed before prompt ownership was observed"
                        .to_string(),
                }),
            );

        runtime
            .pump_owned_structured_provider_output(
                session.id(),
                healthy_run.id(),
                vec![healthy_attachment.id().to_string()],
            )
            .await
            .expect("a promptless background poll failure should remain recoverable");

        if attempt == crate::app::provider_output::STRUCTURED_OUTPUT_POLL_FAILURE_RETRY_LIMIT {
            let session_state = runtime
                .owned
                .session_snapshot(session.id())
                .expect("session snapshot should exist");
            assert!(
                session_state.active_prompt_for_agent(agent.id()).is_none(),
                "the new prompt must start only after the third idle failure is processed"
            );
            let replacement = {
                let mut app = app.lock().await;
                submit_prompt_for_agent(
                    &mut app,
                    session.id(),
                    attachment.id(),
                    agent.id(),
                    "start only after the idle poll retry budget is exhausted",
                )
            };
            runtime
                .owned
                .mark_active_prompt_delivery(
                    session.id(),
                    agent.id(),
                    &replacement,
                    crate::session::DurablePromptDeliveryPhase::Delivered,
                    Some(run.id().to_string()),
                    run.provider_session_id().map(str::to_string),
                )
                .expect("the replacement prompt should bind to the same provider run");
            runtime.owned.note_prompt_started(run.id());
            replacement_prompt_id = Some(replacement);
        }
    }

    let replacement_prompt_id = replacement_prompt_id.expect("replacement prompt should exist");
    assert!(
        output_store.poll_due(run.id(), u64::MAX),
        "a prompt started after the third idle failure must receive a fresh poll admission"
    );
    let admitted = runtime
        .pump_owned_structured_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
        )
        .await
        .expect("the replacement poll should be admitted");
    assert!(
        admitted.is_empty(),
        "poll admission should not fabricate output"
    );
    assert!(
        !output_store.poll_due(run.id(), u64::MAX),
        "an admitted replacement poll should be tracked as in flight"
    );

    let replacement_output = b"output after a post-failure prompt start".to_vec();
    app.lock()
        .await
        .providers_mut()
        .push_finished_structured_output_poll_for_test(
            run.id().to_string(),
            Ok(Some(crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                    merge_key: Some("promptless-before-start".to_string()),
                    bytes: replacement_output.clone(),
                }],
                ..crate::provider::ProviderPromptSignalBatch::default()
            })),
        );
    let records = runtime
        .pump_owned_structured_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
        )
        .await
        .expect("the post-failure replacement poll should be delivered");
    assert_eq!(
        records
            .iter()
            .map(|record| record.bytes.clone())
            .collect::<Vec<_>>(),
        vec![replacement_output],
        "the prompt started after the third failure must receive output"
    );
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist after replacement output");
    assert_eq!(
        session_state
            .active_prompt_for_agent(agent.id())
            .expect("replacement prompt should remain active")
            .id(),
        replacement_prompt_id
    );
    let run_after = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should remain inspectable");
    assert_eq!(
        run_after.state(),
        crate::provider::ProviderRunState::Running
    );
    assert_eq!(run_after.terminal_diagnostic(), None);
}
