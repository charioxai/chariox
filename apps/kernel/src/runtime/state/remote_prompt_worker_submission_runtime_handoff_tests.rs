// MP-08 / MP-10 / MP-11: transport-envelope regression; supplementary, not live acceptance.
use super::*;

async fn assert_home_handoff_envelope(active_run: bool, current_lease_output: bool, protect: bool) {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(CANCELLATION_GUARD_RELAY_URL.to_string());
    config.relay_token = Some("recreated-handoff-test-token".to_string());
    let home_public_key = config.relay_public_key.clone();
    let worker_config = crate::config::DaemonConfig::for_tests();

    let mut app = DaemonApp::bootstrap(config).expect("home app should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "recreated-handoff-workspace",
            "recreated-handoff-worktree",
        ))
        .expect("home session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "recreated-handoff-client",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("home attachment should be created");
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.to_string(),
                worker_machine_id: "worker-machine-recreated-handoff".to_string(),
                execution_lease_id: "lease-recreated-handoff".to_string(),
                leased_agent_id: CANCELLATION_GUARD_LEASED_AGENT_ID.to_string(),
                active_worker_provider_run_id: active_run.then(|| "worker-current-run".to_string()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .expect("home agent should bind to the fake worker");

    // The worker restarted: its new lease has no native conversation, while
    // the home still owns the earlier worker's user and assistant records.
    let prior_run = crate::provider::projected_leased_provider_run_id(
        if current_lease_output {
            CANCELLATION_GUARD_LEASED_AGENT_ID
        } else {
            "old-leased-agent"
        },
        "provider-run-1",
    );
    let history = app.operational_history_store();
    for (sequence, entry) in [
        (
            1,
            crate::history::SessionHistoryEntry::user_prompt(
                session.id(),
                attachment.id(),
                agent.id(),
                "accepted goal: audit continuity; private-room-fixture",
            ),
        ),
        (
            2,
            crate::history::SessionHistoryEntry::provider_output(
                session.id(),
                &prior_run,
                Some(agent.id()),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                None,
                "assistant proposal: preserve named decisions",
            ),
        ),
    ] {
        history
            .append(&crate::history::HistoryEvent::transcript(
                sequence,
                &entry,
                crate::history::HistoryEventTurnContext {
                    provider: Some(agent.provider().to_string()),
                    provider_run_id: Some(prior_run.clone()),
                    prompt_id: Some("prior-home-prompt".to_string()),
                    ..crate::history::HistoryEventTurnContext::default()
                },
            ))
            .unwrap();
    }

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    if protect {
        runtime
            .owned
            .room_secret_observations
            .register(session.id(), "private-room-fixture")
            .unwrap();
    }
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let mut submission = runtime
        .owned
        .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: PromptQueueItem::new(
                "pending:recreated-handoff",
                &attachment_id,
                &agent_id,
                "hold the worker response while manifest sync runs",
                PromptStatus::Queued,
            ),
            force_queue: false,
            refresh_projection: true,
        })
        .expect("remote prompt should be admitted")
        .expect("remote prompt should produce a dispatch");
    let mut dispatch = submission
        .remote_dispatch
        .take()
        .expect("accepted remote prompt should carry a dispatch");
    let submitted_prompt = PromptQueueItem::new(
        format!("worker-{}", dispatch.prompt_id),
        "worker-attachment",
        &dispatch.leased_agent_id,
        dispatch.prompt.clone(),
        PromptStatus::Running,
    );

    let relay_state = Arc::clone(&runtime.owned.relay_state);
    let (outgoing_tx, mut peer_requests, _event_rx) =
        crate::transport::relay_client::RelayOutgoingSender::channel(8);
    {
        let mut relay = relay_state.write().await;
        relay.test_set_connected_sender(outgoing_tx, CANCELLATION_GUARD_RELAY_URL);
        relay.remember_peer_public_key(
            CANCELLATION_GUARD_WORKER_ID,
            worker_config.relay_public_key.clone(),
        );
    }

    let submit_runtime = runtime.clone();
    let submit_prompt = dispatch.prompt.clone();
    let submit_task = tokio::spawn(async move {
        submit_remote_prompt_to_worker_with_binding_refresh(
            &submit_runtime,
            &mut dispatch,
            submit_prompt,
            Vec::new(),
        )
        .await
    });

    let (submit_request_id, request) =
        receive_fake_worker_request(&mut peer_requests, &worker_config.relay_private_key).await;
    let crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
        prompt,
        hidden_system_context,
        ..
    } = request
    else {
        panic!("expected the worker prompt envelope")
    };
    assert_eq!(prompt, "hold the worker response while manifest sync runs");
    if active_run || current_lease_output {
        assert!(
            hidden_system_context.is_empty(),
            "same-lease native continuity must skip home replay"
        );
    } else {
        assert!(
            hidden_system_context.contains("accepted goal: audit continuity"),
            "a recreated worker must receive the home's earlier user history"
        );
        assert_eq!(
            hidden_system_context.contains("assistant proposal: preserve named decisions"),
            !protect,
            "current Room policy withholds old provider observations when protection begins",
        );
        assert!(hidden_system_context.len() <= 24_000);
        assert_eq!(
            hidden_system_context.contains("private-room-fixture"),
            !protect,
            "current Room policy must protect the actual outgoing envelope"
        );
    }

    let submit_response = crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
        provider_run_id: "provider-run-recreated-handoff".to_string(),
        outcome: crate::session::PromptSubmissionOutcome::Started {
            prompt: submitted_prompt,
        },
    };
    let encrypted_submit_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &worker_config.relay_private_key,
        &home_public_key,
        &serde_json::to_vec(&submit_response).expect("submit response should encode"),
    )
    .expect("fake worker should encrypt the submit response");
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &relay_state,
        submit_request_id,
        CANCELLATION_GUARD_WORKER_ID.to_string(),
        encrypted_submit_response,
    )
    .await;
    assert_eq!(
        submit_task
            .await
            .expect("prompt submission task should not panic")
            .expect("prompt submission should succeed"),
        "provider-run-recreated-handoff"
    );
}

#[tokio::test]
async fn recreated_lease_replays_home_history_in_hidden_context() {
    assert_home_handoff_envelope(false, false, false).await;
}

#[tokio::test]
async fn accepted_worker_run_does_not_replay_home_history() {
    assert_home_handoff_envelope(true, false, false).await;
}

#[tokio::test]
async fn current_lease_history_keeps_native_continuity() {
    assert_home_handoff_envelope(false, true, false).await;
}

#[tokio::test]
async fn recreated_lease_handoff_uses_current_room_protection() {
    assert_home_handoff_envelope(false, false, true).await;
}
