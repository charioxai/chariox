//! MP-08 / MP-09 / MP-10 / MP-11: a parent's deadline reaches a leased child's worker.
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};

/// Answers every worker lookup and leased-prompt cancellation through the fake relay.
async fn serve_fake_worker(
    listener: TcpListener,
    worker_public_key: String,
    worker_private_key: String,
    home_public_key: String,
    cancelled: mpsc::UnboundedSender<RelayPeerRequest>,
) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let (worker_public_key, worker_private_key, home_public_key, cancelled) = (
            worker_public_key.clone(),
            worker_private_key.clone(),
            home_public_key.clone(),
            cancelled.clone(),
        );
        tokio::spawn(async move {
            let Ok(mut socket) = accept_async(stream).await else {
                return;
            };
            while let Some(Ok(message)) = socket.next().await {
                let Ok(text) = message.to_text() else {
                    continue;
                };
                let Ok(envelope) = serde_json::from_str::<RelayEnvelope>(text) else {
                    continue;
                };
                match envelope {
                    RelayEnvelope::ClientMetadataRequest { request_id, .. } => {
                        let presence = serde_json::from_value(serde_json::json!({
                            "kernel_id": WORKER_ID,
                            "machine_id": "worker-machine-deadline",
                            "public_key": worker_public_key,
                        }))
                        .unwrap();
                        send_temporary_relay_envelope(
                            &mut socket,
                            RelayEnvelope::ClientMetadataResponse {
                                request_id,
                                machines: None,
                                kernels: None,
                                kernel: Some(presence),
                                error: None,
                            },
                        )
                        .await;
                    }
                    envelope @ RelayEnvelope::DaemonPeerRequest { .. } => {
                        let (request_id, _, request) =
                            decode_peer_request(envelope, &worker_private_key);
                        let RelayPeerRequest::CancelLeasedPrompt { home_prompt_id, .. } = &request
                        else {
                            if let RelayPeerRequest::SetLeasedAgentDeadline {
                                leased_agent_id,
                                ..
                            } = request
                            {
                                let encrypted_response =
                                    crate::transport::relay_crypto::encrypt_payload_for_peer(
                                        &worker_private_key,
                                        &home_public_key,
                                        &serde_json::to_vec(
                                            &RelayPeerResponse::LeasedAgentDeadlineSet {
                                                leased_agent_id,
                                            },
                                        )
                                        .unwrap(),
                                    )
                                    .unwrap();
                                send_temporary_relay_envelope(
                                    &mut socket,
                                    RelayEnvelope::DaemonPeerResponse {
                                        request_id,
                                        from_daemon_id: WORKER_ID.to_string(),
                                        encrypted_response: Some(encrypted_response),
                                        error: None,
                                    },
                                )
                                .await;
                            }
                            continue;
                        };
                        let prompt = crate::session::PromptQueueItem::new(
                            home_prompt_id,
                            "worker-attachment",
                            LEASED_AGENT_ID,
                            "worker turn",
                            crate::session::PromptStatus::Cancelling,
                        );
                        let _ = cancelled.send(request);
                        let response = RelayPeerResponse::LeasedPromptCancelled {
                            cancellation: crate::session::PromptCancellation {
                                prompt,
                                started_next: None,
                            },
                        };
                        let encrypted_response =
                            crate::transport::relay_crypto::encrypt_payload_for_peer(
                                &worker_private_key,
                                &home_public_key,
                                &serde_json::to_vec(&response).unwrap(),
                            )
                            .unwrap();
                        send_temporary_relay_envelope(
                            &mut socket,
                            RelayEnvelope::DaemonPeerResponse {
                                request_id,
                                from_daemon_id: WORKER_ID.to_string(),
                                encrypted_response: Some(encrypted_response),
                                error: None,
                            },
                        )
                        .await;
                    }
                    _ => {}
                }
            }
        });
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_deadline_cancels_running_leased_child_on_its_worker() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{}", listener.local_addr().unwrap()));
    config.relay_token = Some("deadline-test-token".to_string());
    config.relay_request_timeout_ms = 2_000;
    config.room_agent_tools = true;
    let home_public_key = config.relay_public_key.clone();
    let worker_config = crate::config::DaemonConfig::for_tests();
    let (cancelled_tx, mut cancelled_rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(serve_fake_worker(
        listener,
        worker_config.relay_public_key.clone(),
        worker_config.relay_private_key.clone(),
        home_public_key,
        cancelled_tx,
    ));

    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let (session, parent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "deadline-workspace",
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ))
        .unwrap();
    crate::test_support::admit_room_test_turn(&mut app, session.id(), parent.id());
    let child = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_parent_agent_id(parent.id()),
        )
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "deadline-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    app.agents
        .bind_remote_execution(
            child.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: WORKER_ID.to_string(),
                worker_machine_id: "worker-machine-deadline".to_string(),
                execution_lease_id: "worker-lease-deadline".to_string(),
                leased_agent_id: LEASED_AGENT_ID.to_string(),
                active_worker_provider_run_id: Some(WORKER_RUN_ID.to_string()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
                worker_worktree_id: None,
            },
        )
        .unwrap();
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(
            session.id(),
            crate::session::PromptQueueItem::new(
                "home-prompt-deadline",
                attachment.id(),
                child.id(),
                "run on the worker",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .unwrap()
    else {
        panic!("the leased child's turn should be running");
    };
    app.mark_active_prompt_delivery(
        session.id(),
        child.id(),
        prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(WORKER_RUN_ID.to_string()),
        None,
    )
    .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let store = &runtime.owned.durable_state_store;
    let Outcome::Task(child_task) = store
        .agent_lifecycle(Operation::Begin {
            owner: session.owner_user_id().into(),
            room: session.id().into(),
            agent: child.id().into(),
            prompt: prompt.id().into(),
            run: Some(crate::provider::projected_leased_provider_run_id(
                LEASED_AGENT_ID,
                WORKER_RUN_ID,
            )),
            now: 1,
        })
        .unwrap()
    else {
        panic!("child task");
    };

    let deadline_ms = crate::session::unix_epoch_ms() + 200;
    runtime
        .set_agent_deadline(
            session.owner_user_id(),
            crate::local::SetAgentDeadlineRequest {
                session_id: session.id().into(),
                agent_id: parent.id().into(),
                deadline_ms,
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let first = runtime.tick_agent_deadlines(deadline_ms + 1).await;

    let request = tokio::time::timeout(std::time::Duration::from_secs(5), cancelled_rx.recv())
        .await
        .expect("the parent's deadline must send the leased child's cancellation to its worker")
        .unwrap();
    assert!(matches!(
        request,
        RelayPeerRequest::CancelLeasedPrompt {
            leased_agent_id,
            home_prompt_id,
            worker_provider_run_id,
        } if leased_agent_id == LEASED_AGENT_ID
            && home_prompt_id == prompt.id()
            && worker_provider_run_id == WORKER_RUN_ID
    ));
    assert!(
        matches!(&first, Err(DaemonError::MachineRequest { code, .. })
            if code == "agent_deadline_cleanup_pending"),
        "cleanup stays pending until the worker settles the turn: {first:?}"
    );
    let current = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    let active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&current, child.id())
        .expect("the worker has not settled the leased turn yet");
    assert_eq!(active.status(), crate::session::PromptStatus::Cancelling);
    let unsettled = store.agent_task(&child_task.task_id).unwrap().unwrap();
    assert_ne!(
        unsettled.state,
        ExecutionState::Cancelled,
        "the leased child is not reported cancelled while its worker still runs"
    );
    for agent in [parent.id(), child.id()] {
        assert!(store
            .agent_deadline(session.id(), agent)
            .unwrap()
            .unwrap()
            .stopped_at_ms
            .is_none());
    }

    // The worker confirms the leased turn stopped.
    runtime
        .owned
        .finalize_remote_prompt_cancellation_after_worker_settled(
            session.id(),
            child.id(),
            attachment.id(),
        )
        .unwrap();
    runtime.tick_agent_deadlines(deadline_ms + 2).await.unwrap();
    assert_eq!(
        store
            .agent_task(&child_task.task_id)
            .unwrap()
            .unwrap()
            .state,
        ExecutionState::Cancelled
    );
    for agent in [parent.id(), child.id()] {
        assert!(store
            .agent_deadline(session.id(), agent)
            .unwrap()
            .unwrap()
            .stopped_at_ms
            .is_some());
    }
    server.abort();
}
