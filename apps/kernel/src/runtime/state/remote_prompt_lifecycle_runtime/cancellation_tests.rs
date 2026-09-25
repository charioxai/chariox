use super::*;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::RelayEnvelope;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const RELAY_URL: &str = "ws://127.0.0.1:41327";
const WORKER_ID: &str = "worker-kernel-cancel-ack";
const LEASED_AGENT_ID: &str = "leased-agent-cancel-ack";
const WORKER_RUN_ID: &str = "worker-run-after-submit-ack";

async fn owned_runtime_state(app: &Arc<Mutex<crate::app::DaemonApp>>) -> KernelRuntimeState {
    let (
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    ) = {
        let app = app.lock().await;
        (
            app.config_projection_store(),
            app.session_state_store(),
            app.agents().clone(),
            app.attachments().clone(),
            app.providers().clone(),
            app.provider_process_tracking_store(),
            app.slices(),
            app.session_state_projection_store(),
            app.provider_run_projection_store(),
            app.operational_history_store(),
            app.durable_state_store(),
            app.prompt_state_owner(),
            app.active_turn_store(),
            app.prompt_activity_store(),
            app.prompt_workspace_claim_store(),
            app.structured_output_record_store(),
            app.terminal_stream_store(),
            app.workflow_design_event_store(),
            app.metaagent_event_store(),
            app.workspace_coordinator(),
        )
    };
    KernelRuntimeState::new_with_owned_state(
        Arc::clone(app),
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    )
}

async fn next_peer_request(
    receiver: &mut mpsc::Receiver<RelayEnvelope>,
    worker_private_key: &str,
) -> (String, String, RelayPeerRequest) {
    let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv())
        .await
        .expect("fake relay should receive the peer request")
        .expect("fake relay request channel should remain open");
    let RelayEnvelope::DaemonPeerRequest {
        request_id,
        target,
        encrypted_request,
    } = envelope
    else {
        panic!("expected a daemon peer request");
    };
    let target_id = target
        .daemon_id
        .or(target.daemon_alias)
        .expect("request should target a worker kernel");
    let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
        worker_private_key,
        &encrypted_request,
    )
    .expect("fake worker should decrypt the peer request");
    let request =
        serde_json::from_slice(&decrypted.plaintext).expect("fake worker request should decode");
    (request_id, target_id, request)
}

async fn acknowledge_peer_request(
    relay_state: &Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    request_id: String,
    worker_private_key: &str,
    home_public_key: &str,
    response: RelayPeerResponse,
) {
    let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
        worker_private_key,
        home_public_key,
        &serde_json::to_vec(&response).expect("fake worker response should encode"),
    )
    .expect("fake worker should encrypt the peer response");
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        relay_state,
        request_id,
        WORKER_ID.to_string(),
        encrypted,
    )
    .await;
}

#[tokio::test]
async fn accepted_cancellation_waits_through_dispatch_and_durable_ack_then_forwards_once() {
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

    let submit_runtime = runtime.clone();
    let submit_dispatch = dispatch.clone();
    let submit = tokio::spawn(async move {
        submit_runtime
            .submit_remote_prompt_attempt(
                &submit_dispatch,
                submit_dispatch.prompt.clone(),
                Vec::new(),
                None,
                "unexpected fake worker response",
            )
            .await
    });
    let submit_request_id = loop {
        let (request_id, target_id, request) =
            next_peer_request(&mut peer_requests, &worker_private_key).await;
        assert_eq!(target_id, WORKER_ID);
        match request {
            RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
                leased_agent_id, ..
            } => {
                assert_eq!(leased_agent_id, LEASED_AGENT_ID);
                acknowledge_peer_request(
                    &relay_state,
                    request_id,
                    &worker_private_key,
                    &home_public_key,
                    RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated {
                        leased_agent_id: LEASED_AGENT_ID.to_string(),
                    },
                )
                .await;
            }
            RelayPeerRequest::SubmitLeasedPrompt {
                leased_agent_id, ..
            } => {
                assert_eq!(leased_agent_id, LEASED_AGENT_ID);
                break request_id;
            }
            other => panic!("expected prompt submission, got {other:?}"),
        }
    };

    let cancel_runtime = runtime.clone();
    let cancel_session_id = session_id.clone();
    let cancel_agent_id = agent_id.clone();
    let cancel_attachment_id = attachment_id.clone();
    let cancellation = tokio::spawn(async move {
        cancel_runtime
            .cancel_remote_agent_prompt_if_remote(
                &cancel_session_id,
                &cancel_agent_id,
                &cancel_attachment_id,
            )
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), peer_requests.recv())
            .await
            .is_err(),
        "CancelLeasedPrompt must wait for the in-flight submit ACK"
    );

    runtime
        .owned
        .mark_active_prompt_delivery(
            &session_id,
            &agent_id,
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            None,
            None,
        )
        .expect("home prompt should enter Dispatching before worker submission completes");
    let worker_prompt = prompt.clone();
    acknowledge_peer_request(
        &relay_state,
        submit_request_id,
        &worker_private_key,
        &home_public_key,
        RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id: WORKER_RUN_ID.to_string(),
            outcome: crate::session::PromptSubmissionOutcome::Started {
                prompt: worker_prompt.clone(),
            },
        },
    )
    .await;
    assert_eq!(
        submit
            .await
            .expect("submit task should join")
            .expect("fake worker should ACK"),
        WORKER_RUN_ID
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), peer_requests.recv())
            .await
            .is_err(),
        "worker ACK alone must not outrun the durable home ACK"
    );
    assert!(matches!(
        runtime
            .owned
            .settle_remote_dispatch_if_current(&dispatch, Some(WORKER_RUN_ID))
            .expect("home should persist the submission ACK"),
        RemotePromptDispatchSettlement::Settled(_)
    ));

    let (cancel_request_id, target_id, request) =
        next_peer_request(&mut peer_requests, &worker_private_key).await;
    assert_eq!(target_id, WORKER_ID);
    let RelayPeerRequest::CancelLeasedPrompt { leased_agent_id } = request else {
        panic!("expected one CancelLeasedPrompt after worker ACK, got {request:?}");
    };
    assert_eq!(leased_agent_id, LEASED_AGENT_ID);
    let binding = runtime
        .owned
        .agent_store
        .get_agent(&agent_id)
        .expect("home agent should remain available");
    assert_eq!(
        binding
            .remote_execution()
            .and_then(|remote| remote.active_worker_provider_run_id.as_deref()),
        Some(WORKER_RUN_ID),
        "the cancellation is routed while the ACKed worker run owns the lease"
    );
    acknowledge_peer_request(
        &relay_state,
        cancel_request_id,
        &worker_private_key,
        &home_public_key,
        RelayPeerResponse::LeasedPromptCancelled {
            cancellation: crate::session::PromptCancellation {
                prompt: worker_prompt,
                started_next: None,
            },
        },
    )
    .await;
    cancellation
        .await
        .expect("cancellation task should join")
        .expect("fake worker cancellation should succeed")
        .expect("remote cancellation should be handled");
    assert!(
        peer_requests.try_recv().is_err(),
        "the home kernel must forward exactly one cancellation request"
    );

    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .expect("home session should remain available");
    let active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .expect("home should keep one cancelling prompt until worker settlement");
    let mirrored = session
        .active_prompt_for_agent(&agent_id)
        .expect("session projection should mirror the same active prompt");
    assert_eq!(active.id(), prompt_id.as_str());
    assert_eq!(active.status(), crate::session::PromptStatus::Cancelling);
    assert_eq!(
        active.durable_delivery_provider_run_id(),
        Some(WORKER_RUN_ID)
    );
    assert_eq!(mirrored.id(), active.id());
    assert_eq!(mirrored.status(), active.status());
}
