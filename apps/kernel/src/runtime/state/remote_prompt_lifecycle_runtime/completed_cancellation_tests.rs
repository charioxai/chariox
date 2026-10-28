use super::*;

// MP-08 / MP-09 / MP-10 / MP-11: a completed worker clears its active-run pointer.
#[tokio::test]
async fn completed_remote_cancellation_recovers_after_active_run_was_cleared() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(RELAY_URL.to_string());
    config.relay_token = Some("cancel-ack-test-token".to_string());
    config.relay_request_timeout_ms = 2_000;
    let home_public_key = config.relay_public_key.clone();
    let worker_config = crate::config::DaemonConfig::for_tests();
    let worker_private_key = worker_config.relay_private_key.clone();

    let mut app = crate::app::DaemonApp::bootstrap(config).expect("home app should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "cancel-ack-workspace",
            "cancel-ack-worktree",
        ))
        .expect("home session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "cancel-ack-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("home attachment should be created");
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: WORKER_ID.to_string(),
                worker_machine_id: "worker-machine-cancel-ack".to_string(),
                execution_lease_id: "worker-lease-cancel-ack".to_string(),
                leased_agent_id: LEASED_AGENT_ID.to_string(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .expect("home agent should bind to the fake worker");

    let home_prompt = crate::session::PromptQueueItem::new(
        "home-prompt-dispatch-cancel-ack",
        attachment.id(),
        agent.id(),
        "submit then cancel",
        crate::session::PromptStatus::Queued,
    );
    let submission = app
        .prompt_owner_submit_prepared_prompt(session.id(), home_prompt, false)
        .expect("home prompt should be admitted");
    let crate::session::PromptSubmissionOutcome::Started { prompt } = submission else {
        panic!("remote prompt should become active immediately");
    };
    assert_eq!(
        prompt.durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Accepted),
        "an admitted remote prompt starts in Accepted before dispatch claims it"
    );
    let prompt_id = prompt.id().to_string();
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let dispatch = crate::app::KernelRemotePromptDispatch {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        prompt_id: prompt_id.clone(),
        worker_kernel_id: WORKER_ID.to_string(),
        leased_agent_id: LEASED_AGENT_ID.to_string(),
        relay_url: None,
        relay_token: None,
        source_attachment_id: attachment_id.clone(),
        prompt: prompt.prompt().to_string(),
        hidden_system_context: prompt.hidden_system_context().to_string(),
        attachments: prompt.attachments().to_vec(),
        workspace_live_sync_mode: None,
        prompt_origin: prompt.prompt_origin(),
        external_provider: prompt.external_provider().map(str::to_string),
        external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
        external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
        workflow_context: None,
    };

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let relay_state = Arc::clone(&runtime.owned.relay_state);
    let (outgoing_tx, mut peer_requests, _event_rx) =
        crate::transport::relay_client::RelayOutgoingSender::channel(8);
    {
        let mut relay = relay_state.write().await;
        relay.test_set_connected_sender(outgoing_tx, RELAY_URL);
        relay.remember_peer_public_key(WORKER_ID, worker_config.relay_public_key.clone());
    }

    runtime
        .owned
        .mark_active_prompt_delivery(
            &session_id,
            &agent_id,
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(WORKER_RUN_ID.to_string()),
            None,
        )
        .unwrap();
    runtime
        .owned
        .begin_remote_prompt_cancellation_if_matches(
            &session_id,
            &agent_id,
            &attachment_id,
            Some(&prompt_id),
        )
        .unwrap()
        .unwrap();
    runtime
        .owned
        .agent_store
        .set_remote_execution_active_worker_provider_run_id(
            &agent_id,
            Some("newer-worker-run".into()),
        )
        .unwrap();
    assert!(runtime
        .resume_delivered_remote_prompt_cancellation_after_restart(&dispatch, Some(WORKER_RUN_ID),)
        .await
        .is_err());
    assert!(
        peer_requests.try_recv().is_err(),
        "a newer worker run must remain untouched"
    );
    runtime
        .owned
        .agent_store
        .set_remote_execution_active_worker_provider_run_id(&agent_id, None)
        .unwrap();
    let recovering = runtime.clone();
    let expected = dispatch.clone();
    let recovery = tokio::spawn(async move {
        recovering
            .resume_delivered_remote_prompt_cancellation_after_restart(
                &expected,
                Some(WORKER_RUN_ID),
            )
            .await
    });
    let (id, target, request) = next_peer_request(
        &mut peer_requests,
        &worker_private_key,
        "completed cancellation receipt",
    )
    .await;
    assert_eq!(target, WORKER_ID);
    assert!(
        matches!(request, RelayPeerRequest::GetLeasedPromptReceipt { home_prompt_id, .. } if home_prompt_id == prompt_id)
    );
    acknowledge_peer_request(
        &relay_state,
        id,
        &worker_private_key,
        &home_public_key,
        RelayPeerResponse::LeasedPromptReceiptQueried {
            receipt: Some(crate::transport::relay_peer::LeasedPromptReceipt {
                home_prompt_id: prompt_id.clone(),
                worker_provider_run_id: WORKER_RUN_ID.into(),
                phase: crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed,
                target_home_prompt_id: None,
                execution_lease_id: Some("worker-lease-cancel-ack".into()),
            }),
        },
    )
    .await;
    let (id, target, request) = next_peer_request(
        &mut peer_requests,
        &worker_private_key,
        "completed cancellation projection",
    )
    .await;
    assert_eq!(target, WORKER_ID);
    assert!(
        matches!(request, RelayPeerRequest::DrainLeasedRuntimeProjection { provider_run_id, .. } if provider_run_id == WORKER_RUN_ID)
    );
    acknowledge_peer_request(
        &relay_state,
        id,
        &worker_private_key,
        &home_public_key,
        RelayPeerResponse::LeasedRuntimeProjectionDrained {
            event: Some(
                crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                    home_session_id: session_id.clone(),
                    home_agent_id: agent_id.clone(),
                    provider_run_id: WORKER_RUN_ID.into(),
                    provider_run: None,
                    prompts: Vec::new(),
                    output_chunks: Vec::new(),
                    notices: Vec::new(),
                    completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
                        message_id: "completed-cancel-recovery".into(),
                        completed_at_ms: 1,
                        home_prompt_id: Some(prompt_id.clone()),
                        provider_termination: None,
                    }],
                },
            ),
        },
    )
    .await;
    recovery.await.unwrap().unwrap();
    let current = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    assert!(runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&current, &agent_id)
        .is_none());
    assert!(
        peer_requests.try_recv().is_err(),
        "a completed run is drained, never replayed or cancelled again"
    );
}
