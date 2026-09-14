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
    let prompt_id = prompt.id().to_string();
    assert!(matches!(
        app.prompt_owner_submit_prepared_prompt(session_id, prompt, false)
            .expect("prompt should start"),
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    prompt_id
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
