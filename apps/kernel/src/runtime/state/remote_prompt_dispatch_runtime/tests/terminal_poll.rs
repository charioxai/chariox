// MP-08/MP-10/MP-11: a delayed worker projection must not block client polling.
use super::*;

#[tokio::test]
async fn terminal_poll_keeps_one_background_drain_and_delivers_late_completion() {
    let relay_url = "ws://127.0.0.1:1";
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(relay_url.to_string());
    config.relay_token = Some("synthetic-terminal-poll".to_string());
    let worker_config = crate::config::DaemonConfig::for_tests();
    let home_public_key = config.relay_public_key.clone();
    let mut app = DaemonApp::bootstrap(config).expect("home should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "poll-workspace",
            "poll-worktree",
        ))
        .expect("session should exist");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "poll-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should exist");
    let outcome = app
        .prompt_owner_submit_prepared_prompt(
            session.id(),
            crate::session::PromptQueueItem::new(
                "poll-prompt",
                attachment.id(),
                agent.id(),
                "synthetic remote prompt",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .expect("prompt should start");
    let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome else {
        panic!("prompt should start")
    };
    let (mut binding, provider_run_id, output_event, mut completion_event) =
        actual_worker_output_then_completion(session.id(), agent.id(), prompt.id());
    binding.active_worker_provider_run_id = Some(provider_run_id.clone());
    app.agents
        .bind_remote_execution(agent.id(), binding.clone())
        .expect("remote binding should exist");
    app.mark_active_prompt_delivery(
        session.id(),
        agent.id(),
        prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(provider_run_id.clone()),
        None,
    )
    .expect("delivered identity should persist");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let (sender, mut requests, _events) =
        crate::transport::relay_client::RelayOutgoingSender::channel(8);
    {
        let mut state = runtime.owned.relay_state.write().await;
        state.test_set_connected_sender(sender, relay_url);
        state.remember_peer_public_key(
            &binding.worker_kernel_id,
            worker_config.relay_public_key.clone(),
        );
    }
    // No worker response exists yet. Foreground terminal reads must remain local.
    let first = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        runtime.pump_terminal_output_with_snapshot(session.id(), attachment.id()),
    )
    .await
    .expect("MP-08/MP-10/MP-11 client polling must not await a worker response")
    .expect("poll should succeed");
    assert!(
        first
            .0
            .iter()
            .all(|record| record.kind != crate::terminal::TerminalOutputKind::ProviderOutput),
        "no remote output may be invented before its authoritative projection"
    );
    let envelope = tokio::time::timeout(std::time::Duration::from_secs(1), requests.recv())
        .await
        .expect("background projection drain should be scheduled")
        .expect("request should exist");
    let RelayEnvelope::DaemonPeerRequest {
        request_id,
        encrypted_request,
        ..
    } = envelope
    else {
        panic!("expected peer request")
    };
    let decoded = crate::transport::relay_crypto::decrypt_payload_for_private_key(
        &worker_config.relay_private_key,
        &encrypted_request,
    )
    .expect("synthetic worker should decrypt its request");
    assert!(
        matches!(serde_json::from_slice::<RelayPeerRequest>(&decoded.plaintext).expect("request should decode"),
        RelayPeerRequest::DrainLeasedRuntimeProjection { leased_agent_id, provider_run_id: id, pump_output: true }
            if leased_agent_id == binding.leased_agent_id && id == provider_run_id)
    );
    tokio::time::timeout(
        std::time::Duration::from_millis(250),
        runtime.pump_terminal_output_with_snapshot(session.id(), attachment.id()),
    )
    .await
    .expect("second poll must not start a competing foreground drain")
    .expect("second poll should succeed");
    assert!(
        requests.try_recv().is_err(),
        "only the claimed background drain may contact the worker"
    );

    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks,
        notices,
        ..
    } = output_event;
    let RelayPeerEvent::LeasedRuntimeProjection {
        output_chunks: completed_output,
        notices: completed_notices,
        ..
    } = &mut completion_event;
    completed_output.extend(output_chunks);
    completed_notices.extend(notices);
    let response =
        crate::transport::relay_peer::RelayPeerResponse::LeasedRuntimeProjectionDrained {
            event: Some(completion_event),
        };
    let encrypted_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &worker_config.relay_private_key,
        &home_public_key,
        &serde_json::to_vec(&response).expect("response should encode"),
    )
    .expect("response should encrypt");
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &runtime.owned.relay_state,
        request_id,
        binding.worker_kernel_id.clone(),
        encrypted_response,
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let current = runtime
                .owned
                .session_store
                .get_session(session.id())
                .expect("session should remain");
            if runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&current, agent.id())
                .is_none()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("late authoritative completion should settle the same home prompt");
    let records = runtime
        .pump_terminal_output_with_snapshot(session.id(), attachment.id())
        .await
        .expect("late output should drain")
        .0;
    assert!(
        !records.is_empty(),
        "late output must be delivered before completion becomes visible"
    );
    assert!(
        runtime
            .pump_terminal_output_with_snapshot(session.id(), attachment.id())
            .await
            .expect("repeat poll should succeed")
            .0
            .is_empty(),
        "authoritative output must be delivered once"
    );
    assert!(
        requests.try_recv().is_err(),
        "settlement must not replay a submitted prompt or drain"
    );
}
