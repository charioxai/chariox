use super::*;
use chariox_relay::protocol::RelayEnvelope;
use tokio::sync::mpsc;

struct RoomManifestFixture {
    runtime: KernelRuntimeState,
    session_id: String,
    agent_id: String,
    relay_state: Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    priority_rx: mpsc::Receiver<RelayEnvelope>,
    worker_private_key: String,
    home_public_key: String,
}

async fn room_manifest_fixture() -> RoomManifestFixture {
    let relay_url = "ws://127.0.0.1:1".to_string();
    let mut home_config = crate::config::DaemonConfig::for_tests();
    home_config.relay_url = Some(relay_url.clone());
    home_config.relay_token = Some("room-manifest-test-token".to_string());
    let home_public_key = home_config.relay_public_key.clone();
    let worker_config = crate::config::DaemonConfig::for_tests();
    let worker_private_key = worker_config.relay_private_key.clone();
    let (app, runtime, session_id, agent_id) = agent_config_runtime_with_config(home_config).await;
    crate::app::KernelSessionService::new(&mut *app.lock().await)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "room-manifest-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("fixture session should have an attachment");
    app.lock()
        .await
        .agents_mut()
        .bind_remote_execution(
            &agent_id,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-1".to_string(),
                worker_machine_id: "machine-1".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("provider-run-current".to_string()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .expect("agent should bind to an active worker run");

    let relay_state = app.lock().await.relay_client_state();
    let (outgoing_tx, priority_rx, _event_rx) =
        crate::transport::relay_client::RelayOutgoingSender::channel(8);
    {
        let mut relay = relay_state.write().await;
        relay.test_set_connected_sender(outgoing_tx, relay_url);
        relay.remember_peer_public_key("worker-1", worker_config.relay_public_key.clone());
    }

    RoomManifestFixture {
        runtime,
        session_id,
        agent_id,
        relay_state,
        priority_rx,
        worker_private_key,
        home_public_key,
    }
}

#[tokio::test]
async fn projected_remote_completion_admits_queued_prompt_before_ordered_delivery() {
    let mut fixture = room_manifest_fixture().await;
    let (completed_prompt_id, pending_prompt_id, attachment_id) = {
        let mut app = fixture.runtime.app.lock().await;
        let attachment_id = app
            .attachments()
            .list_session_attachment_ids(&fixture.session_id)
            .into_iter()
            .next()
            .expect("fixture session has an attachment");
        let active = crate::session::PromptQueueItem::new(
            "remote-active",
            &attachment_id,
            &fixture.agent_id,
            "active prompt",
            crate::session::PromptStatus::Queued,
        );
        let active = app
            .prompt_owner_submit_prepared_prompt(&fixture.session_id, active, false)
            .expect("active prompt should be admitted");
        let crate::session::PromptSubmissionOutcome::Started { prompt: active } = active else {
            panic!("fixture prompt should start");
        };
        let queued = crate::session::PromptQueueItem::new(
            "remote-queued",
            &attachment_id,
            &fixture.agent_id,
            "queued prompt",
            crate::session::PromptStatus::Queued,
        );
        let queued = app
            .prompt_owner_submit_prepared_prompt(&fixture.session_id, queued, true)
            .expect("second prompt should be admitted to queue");
        let crate::session::PromptSubmissionOutcome::Queued { prompt: queued } = queued else {
            panic!("second fixture prompt should queue");
        };
        (
            active.id().to_string(),
            queued.id().to_string(),
            attachment_id,
        )
    };

    let held_lane = fixture
        .runtime
        .leased_agent_operations
        .lock("leased-agent-1")
        .await;
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        fixture.runtime.project_relay_remote_runtime_projection(
            &fixture.session_id,
            &fixture.agent_id,
            "provider-run-current",
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![crate::transport::relay_peer::RelayProjectedCompletion {
                message_id: "queued-completion-ordering".to_string(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some(completed_prompt_id),
                provider_termination: None,
            }],
        ),
    )
    .await
    .expect("projection must return without waiting on the leased-agent lane")
    .expect("worker completion projection should succeed");
    assert!(fixture.priority_rx.try_recv().is_err());
    let pre_ack_echo_count = fixture
        .runtime
        .owned
        .terminal_stream
        .drain_output_records(&fixture.session_id, &attachment_id)
        .into_iter()
        .filter(|record| record.kind == crate::terminal::TerminalOutputKind::PromptEcho)
        .count();
    assert_eq!(
        pre_ack_echo_count, 0,
        "queued prompt echo waits for worker ACK"
    );

    let (active, queued_after_promotion) = {
        let mut app = fixture.runtime.app.lock().await;
        (
            app.prompt_owner_active_prompt_for_agent(&fixture.session_id, &fixture.agent_id)
                .expect("promoted prompt state should be readable")
                .expect("queued prompt should become active before dispatch"),
            app.agent_runtime_projection_store()
                .next_queued_prompt(&fixture.session_id, &fixture.agent_id),
        )
    };
    assert_ne!(
        active.id(),
        pending_prompt_id,
        "promotion allocates a fresh prompt ID"
    );
    assert_eq!(active.status(), crate::session::PromptStatus::Dispatching);
    assert_eq!(active.prompt(), "queued prompt");
    assert_eq!(active.source_attachment_id(), attachment_id.as_str());
    assert_eq!(active.target_agent_id(), fixture.agent_id.as_str());
    assert!(queued_after_promotion.is_none());

    drop(held_lane);
    let (request_id, request) = next_peer_request(&mut fixture).await;
    let crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
        prompt,
        git_context,
        ..
    } = request
    else {
        panic!("expected ordered submission for the promoted prompt");
    };
    assert_eq!(prompt, "queued prompt");
    assert_eq!(
        git_context
            .expect("queued prompt delivery carries git turn context")
            .home_prompt_id,
        active.id(),
    );
    let promoted_prompt_id = active.id().to_string();
    acknowledge_peer_response(
        &fixture,
        request_id,
        crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id: "provider-run-next".to_string(),
            outcome: crate::session::PromptSubmissionOutcome::Started { prompt: active },
        },
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let mut app = fixture.runtime.app.lock().await;
            let delivered = app
                .prompt_owner_active_prompt_for_agent(&fixture.session_id, &fixture.agent_id)
                .expect("active prompt should be readable")
                .is_some_and(|prompt| {
                    prompt.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                });
            if delivered {
                break;
            }
            drop(app);
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("acknowledged dispatch should settle");
    let echoes = fixture
        .runtime
        .owned
        .terminal_stream
        .drain_output_records(&fixture.session_id, &attachment_id)
        .into_iter()
        .filter(|record| {
            record.kind == crate::terminal::TerminalOutputKind::PromptEcho
                && record.prompt_id.as_deref() == Some(promoted_prompt_id.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(echoes.len(), 1, "queued prompt should echo exactly once");
    assert_eq!(
        echoes[0].source_attachment_id.as_deref(),
        Some(attachment_id.as_str())
    );
    assert_eq!(
        echoes[0].provider_run_id,
        crate::provider::projected_leased_provider_run_id("leased-agent-1", "provider-run-next",)
    );
}

#[tokio::test]
async fn ordinary_completion_dispatches_workflow_head_after_projection() {
    let mut fixture = room_manifest_fixture().await;
    let (completed_prompt_id, attachment_id, workflow_run_id, node_run_id) = {
        let mut app = fixture.runtime.app.lock().await;
        let attachment_id = app
            .attachments()
            .list_session_attachment_ids(&fixture.session_id)
            .into_iter()
            .next()
            .expect("fixture session has an attachment");
        let active = crate::session::PromptQueueItem::new(
            "ordinary-active",
            &attachment_id,
            &fixture.agent_id,
            "ordinary prompt",
            crate::session::PromptStatus::Queued,
        );
        let active = app
            .prompt_owner_submit_prepared_prompt(&fixture.session_id, active, false)
            .expect("ordinary prompt should start");
        let crate::session::PromptSubmissionOutcome::Started { prompt: active } = active else {
            panic!("ordinary fixture prompt should start");
        };
        let workflow = app
            .sessions_mut()
            .create_workflow(&fixture.session_id, None)
            .expect("workflow should be created");
        let node = app
            .sessions_mut()
            .add_workflow_node(&fixture.session_id, workflow.id(), &fixture.agent_id)
            .expect("workflow node should be created");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(&fixture.session_id, workflow.id(), node.id(), None)
            .expect("workflow endpoint should be created");
        let run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                &fixture.session_id,
                workflow.id(),
                endpoint.id(),
                Some("queued workflow prompt".to_string()),
            )
            .expect("workflow should be invoked");
        let node_run_id = run.node_runs()[0].id().to_string();
        app.sessions_mut()
            .prepare_workflow_turn(
                &fixture.session_id,
                run.id(),
                &node_run_id,
                "workflow-turn".to_string(),
                "queued workflow prompt".to_string(),
                None,
                None,
            )
            .expect("workflow turn should be prepared");
        let workflow_prompt = crate::session::PromptQueueItem::new(
            "queued-workflow",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(run.id()),
            &fixture.agent_id,
            "queued workflow prompt",
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(run.id(), &node_run_id);
        let queued = app
            .prompt_owner_submit_prepared_prompt(&fixture.session_id, workflow_prompt, true)
            .expect("workflow prompt should queue behind ordinary work");
        assert!(matches!(
            queued,
            crate::session::PromptSubmissionOutcome::Queued { .. }
        ));
        (
            active.id().to_string(),
            attachment_id,
            run.id().to_string(),
            node_run_id,
        )
    };

    let mut projection = tokio::spawn({
        let runtime = fixture.runtime.clone();
        let session_id = fixture.session_id.clone();
        let agent_id = fixture.agent_id.clone();
        async move {
            runtime
                .project_relay_remote_runtime_projection(
                    &session_id,
                    &agent_id,
                    "provider-run-current",
                    None,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    vec![crate::transport::relay_peer::RelayProjectedCompletion {
                        message_id: "ordinary-completes-before-workflow".to_string(),
                        completed_at_ms: crate::session::unix_epoch_ms(),
                        home_prompt_id: Some(completed_prompt_id),
                        provider_termination: None,
                    }],
                )
                .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), &mut projection)
        .await
        .expect("completion projection should finish before workflow dispatch")
        .expect("projection task should join")
        .expect("ordinary completion with queued workflow should succeed");
    let (request_id, request) = next_peer_request(&mut fixture).await;
    let crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
        prompt,
        workflow_context,
        git_context,
        ..
    } = request
    else {
        panic!("workflow head should enter ordered submission");
    };
    let git_context = git_context.expect("workflow prompt should carry git context");
    assert_eq!(prompt, "queued workflow prompt");
    assert_eq!(
        git_context.source_attachment_id.as_deref(),
        Some(
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(&workflow_run_id)
                .as_str()
        )
    );
    assert_eq!(git_context.home_prompt_id, git_context.home_turn_id);
    assert_eq!(
        workflow_context
            .expect("workflow head carries workflow context")
            .workflow_run_id,
        workflow_run_id,
    );
    let promoted = crate::session::PromptQueueItem::new(
        git_context.home_prompt_id.clone(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(&workflow_run_id),
        &fixture.agent_id,
        "queued workflow prompt",
        crate::session::PromptStatus::Running,
    )
    .with_workflow_context(&workflow_run_id, &node_run_id);
    acknowledge_peer_response(
        &fixture,
        request_id,
        crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id: "provider-run-workflow-next".to_string(),
            outcome: crate::session::PromptSubmissionOutcome::Started { prompt: promoted },
        },
    )
    .await;
    let mut app = fixture.runtime.app.lock().await;
    let active = app
        .prompt_owner_active_prompt_for_agent(&fixture.session_id, &fixture.agent_id)
        .expect("active prompt state should be readable")
        .expect("workflow prompt should remain active");
    assert_eq!(active.prompt(), "queued workflow prompt");
    assert_eq!(active.target_agent_id(), fixture.agent_id.as_str());
    assert_eq!(
        active.source_attachment_id(),
        git_context.source_attachment_id.as_deref().unwrap(),
    );
    assert_eq!(active.workflow_node_run_id(), Some(node_run_id.as_str()));
    assert_ne!(active.id(), "queued-workflow");
    assert_ne!(attachment_id.as_str(), active.source_attachment_id());
}

#[tokio::test]
async fn rejected_ordered_queued_dispatch_uses_shared_sender_failure_semantics() {
    let mut fixture = room_manifest_fixture().await;
    let completed_prompt_id = {
        let mut app = fixture.runtime.app.lock().await;
        let attachment_id = app
            .attachments()
            .list_session_attachment_ids(&fixture.session_id)
            .into_iter()
            .next()
            .expect("fixture session has an attachment");
        let active = crate::session::PromptQueueItem::new(
            "reject-active",
            &attachment_id,
            &fixture.agent_id,
            "active prompt",
            crate::session::PromptStatus::Queued,
        );
        let active = app
            .prompt_owner_submit_prepared_prompt(&fixture.session_id, active, false)
            .expect("active prompt should start");
        let crate::session::PromptSubmissionOutcome::Started { prompt: active } = active else {
            panic!("active fixture prompt should start");
        };
        let queued = crate::session::PromptQueueItem::new(
            "reject-queued",
            &attachment_id,
            &fixture.agent_id,
            "queued prompt",
            crate::session::PromptStatus::Queued,
        );
        assert!(matches!(
            app.prompt_owner_submit_prepared_prompt(&fixture.session_id, queued, true)
                .expect("second prompt should queue"),
            crate::session::PromptSubmissionOutcome::Queued { .. }
        ));
        active.id().to_string()
    };

    let held_lane = fixture
        .runtime
        .leased_agent_operations
        .lock("leased-agent-1")
        .await;
    fixture
        .runtime
        .project_relay_remote_runtime_projection(
            &fixture.session_id,
            &fixture.agent_id,
            "provider-run-current",
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![crate::transport::relay_peer::RelayProjectedCompletion {
                message_id: "reject-queued-dispatch".to_string(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some(completed_prompt_id),
                provider_termination: None,
            }],
        )
        .await
        .expect("completion projection should return before delivery");
    drop(held_lane);
    let (request_id, request) = next_peer_request(&mut fixture).await;
    assert!(matches!(
        request,
        crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt { .. }
    ));
    // A known pre-admission rejection is safe to cancel. An unexpected response
    // would instead be indeterminate and must retain this exact prompt.
    crate::transport::relay_client::resolve_pending_peer_error_for_test(
        &fixture.relay_state,
        request_id,
        "worker-1".to_string(),
        chariox_relay::protocol::RelayError {
            code: "action_not_allowed".to_string(),
            message: "queued submission rejected before worker admission".to_string(),
            retryable: false,
        },
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let mut app = fixture.runtime.app.lock().await;
            if app
                .prompt_owner_active_prompt_for_agent(&fixture.session_id, &fixture.agent_id)
                .expect("active prompt state should be readable")
                .is_none()
            {
                break;
            }
            drop(app);
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("shared sender should settle and cancel a rejected promoted prompt");
}

fn create_room_slice(runtime: &KernelRuntimeState, name: &str) -> crate::slice::SliceRecord {
    runtime
        .owned
        .slice_store
        .create(
            "home-kernel",
            "home-machine",
            crate::slice::CreateSliceInput {
                name: name.to_string(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: crate::slice::SliceDisplayMode::Headed,
                display_backend: crate::slice::SliceDisplayBackend::Selkies,
                workspace_id: None,
                worktree_id: None,
                workspace_mount: None,
                development: None,
                worker_kernel_ref: Some("worker-1".to_string()),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .expect("Room slice should be created")
}

async fn next_peer_request(
    fixture: &mut RoomManifestFixture,
) -> (String, crate::transport::relay_peer::RelayPeerRequest) {
    let envelope = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        fixture.priority_rx.recv(),
    )
    .await
    .expect("peer request should be queued")
    .expect("connected relay queue should remain open");
    let RelayEnvelope::DaemonPeerRequest {
        request_id,
        encrypted_request,
        ..
    } = envelope
    else {
        panic!("expected a daemon peer request");
    };
    let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
        &fixture.worker_private_key,
        &encrypted_request,
    )
    .expect("worker should decrypt the peer request");
    let request = serde_json::from_slice(&decrypted.plaintext).expect("peer request should decode");
    (request_id, request)
}

async fn next_manifest_update(
    fixture: &mut RoomManifestFixture,
) -> (String, crate::extension::RemoteExtensionManifest) {
    let (request_id, request) = next_peer_request(fixture).await;
    let crate::transport::relay_peer::RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
        leased_agent_id,
        remote_extension_manifest,
    } = request
    else {
        panic!("expected a manifest update, not another peer request");
    };
    assert_eq!(leased_agent_id, "leased-agent-1");
    (request_id, remote_extension_manifest)
}

async fn acknowledge_manifest_update(fixture: &RoomManifestFixture, request_id: String) {
    acknowledge_peer_response(
        fixture,
        request_id,
        crate::transport::relay_peer::RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated {
            leased_agent_id: "leased-agent-1".to_string(),
        },
    )
    .await;
}

async fn acknowledge_peer_response(
    fixture: &RoomManifestFixture,
    request_id: String,
    response: crate::transport::relay_peer::RelayPeerResponse,
) {
    let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &fixture.worker_private_key,
        &fixture.home_public_key,
        &serde_json::to_vec(&response).expect("peer response should encode"),
    )
    .expect("worker should encrypt the manifest response");
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &fixture.relay_state,
        request_id,
        "worker-1".to_string(),
        encrypted,
    )
    .await;
}

struct WorkflowSuccessorSetup {
    ordinary_prompt_id: String,
    workflow_id: String,
}

type WorkflowSuccessorSetupResult = Result<WorkflowSuccessorSetup, crate::error::DaemonError>;

async fn queue_workflow_successor_after_ordinary_prompt(
    fixture: &RoomManifestFixture,
) -> WorkflowSuccessorSetup {
    let session_id = fixture.session_id.clone();
    let agent_id = fixture.agent_id.clone();
    fixture
        .runtime
        .with_app_side_effect(move |app| -> WorkflowSuccessorSetupResult {
            let attachment = crate::app::KernelSessionService::new(app).attach(
                crate::attachment::AttachRequest::new(
                    &session_id,
                    "workflow-successor-test",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ),
            )?;
            let active = crate::session::PromptQueueItem::new(
                "ordinary-active-prompt",
                attachment.id(),
                &agent_id,
                "ordinary prompt",
                crate::session::PromptStatus::Running,
            );
            app.prompt_owner_sync_external_active_prompt(
                &session_id,
                &agent_id,
                Some(active.clone()),
            )?;

            let workflow = app
                .sessions_mut()
                .create_workflow(&session_id, Some("queued-successor".to_string()))?;
            let node =
                app.sessions_mut()
                    .add_workflow_node(&session_id, workflow.id(), &agent_id)?;
            let endpoint = app.sessions_mut().create_workflow_endpoint(
                &session_id,
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )?;
            let run = app.sessions_mut().invoke_workflow_endpoint(
                &session_id,
                workflow.id(),
                endpoint.id(),
                Some("workflow successor".to_string()),
            )?;
            let node_run_id = run
                .node_runs()
                .first()
                .expect("workflow run should have an entry node")
                .id()
                .to_string();
            app.sessions_mut().prepare_workflow_turn(
                &session_id,
                run.id(),
                &node_run_id,
                "workflow-successor-delivery-token".to_string(),
                "workflow successor".to_string(),
                None,
                None,
            )?;
            let workflow_prompt = crate::session::PromptQueueItem::new(
                "queued-workflow-successor",
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(run.id()),
                &agent_id,
                "workflow successor",
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(run.id(), &node_run_id);
            let queued =
                app.prompt_owner_submit_prepared_prompt(&session_id, workflow_prompt, false)?;
            assert!(matches!(
                queued,
                crate::session::PromptSubmissionOutcome::Queued { .. }
            ));
            Ok(WorkflowSuccessorSetup {
                ordinary_prompt_id: active.id().to_string(),
                workflow_id: workflow.id().to_string(),
            })
        })
        .await
        .expect("ordinary prompt and workflow successor should queue")
}

async fn project_ordinary_completion(fixture: &RoomManifestFixture, prompt_id: String) {
    fixture
        .runtime
        .project_relay_remote_runtime_projection(
            &fixture.session_id,
            &fixture.agent_id,
            "provider-run-current",
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![crate::transport::relay_peer::RelayProjectedCompletion {
                message_id: "ordinary-completion-for-workflow-successor".to_string(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some(prompt_id),
                provider_termination: None,
            }],
        )
        .await
        .expect("ordinary completion should promote the queued workflow prompt");
}

async fn active_prompt_for(
    fixture: &RoomManifestFixture,
) -> Option<crate::session::PromptQueueItem> {
    let session_id = fixture.session_id.clone();
    let agent_id = fixture.agent_id.clone();
    fixture
        .runtime
        .with_app_side_effect(move |app| {
            app.prompt_owner_active_prompt_for_agent(&session_id, &agent_id)
        })
        .await
        .expect("active prompt should be readable")
}

#[tokio::test]
async fn ordinary_completion_restores_persisted_workflow_ids_before_ordered_dispatch() {
    let mut fixture = room_manifest_fixture().await;
    let successor = queue_workflow_successor_after_ordinary_prompt(&fixture).await;
    let ordinary_prompt_id = successor.ordinary_prompt_id;
    let persisted_session = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .expect("session with queued workflow successor should be persisted");
    let restored_session: crate::session::RuntimeSession = serde_json::from_slice(
        &serde_json::to_vec(&persisted_session).expect("persisted session should encode"),
    )
    .expect("persisted session should restore");
    let persisted_successor = restored_session
        .queued_prompts_for_agent(&fixture.agent_id)
        .and_then(|prompts| prompts.front())
        .expect("persisted session should retain its queued workflow successor");
    let expected_workflow_run_id = persisted_successor
        .workflow_run_id()
        .expect("persisted workflow successor should retain its run id")
        .to_string();
    let expected_workflow_node_run_id = persisted_successor
        .workflow_node_run_id()
        .expect("persisted workflow successor should retain its node run id")
        .to_string();

    let mut empty_prompt_state = restored_session.clone();
    empty_prompt_state.mirror_agent_prompt_state(
        &fixture.agent_id,
        None,
        std::collections::VecDeque::new(),
    );
    fixture
        .runtime
        .owned
        .prompt_state_owner
        .restore_session_state(&empty_prompt_state);
    fixture
        .runtime
        .owned
        .prompt_state_owner
        .restore_session_state(&restored_session);
    let restored_successor = fixture
        .runtime
        .owned
        .prompt_state_owner
        .peek_next_queued_prompt(&restored_session, &fixture.agent_id)
        .expect("workflow successor should be restored before promotion");
    assert_eq!(
        restored_successor.workflow_run_id(),
        Some(expected_workflow_run_id.as_str())
    );
    assert_eq!(
        restored_successor.workflow_node_run_id(),
        Some(expected_workflow_node_run_id.as_str())
    );
    let lane = fixture
        .runtime
        .leased_agent_operations
        .lock("leased-agent-1")
        .await;

    project_ordinary_completion(&fixture, ordinary_prompt_id).await;
    assert!(
        fixture.priority_rx.try_recv().is_err(),
        "workflow prompt transport must wait for the existing leased-agent lane"
    );
    let promoted = active_prompt_for(&fixture)
        .await
        .expect("workflow successor should be admitted before remote transport");
    assert!(crate::app::workflow_runtime::is_workflow_prompt_source(
        promoted.source_attachment_id()
    ));
    drop(lane);

    let (request_id, request) = next_peer_request(&mut fixture).await;
    let crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
        prompt,
        workflow_context: Some(workflow_context),
        git_context,
        ..
    } = request
    else {
        panic!("expected promoted workflow prompt on the ordered sender");
    };
    assert_eq!(prompt, "workflow successor");
    assert_eq!(
        workflow_context.workflow_run_id, expected_workflow_run_id,
        "successor dispatch should restore its persisted workflow run id"
    );
    assert_eq!(
        workflow_context.workflow_node_run_id, expected_workflow_node_run_id,
        "successor dispatch should restore its persisted workflow node run id"
    );
    assert_eq!(
        git_context
            .as_ref()
            .map(|context| context.home_prompt_id.as_str()),
        Some(promoted.id())
    );

    let app_lock_read = fixture
        .runtime
        .with_app_side_effect(|app| app.config().daemon_id.clone());
    let daemon_id = tokio::time::timeout(std::time::Duration::from_secs(2), app_lock_read)
        .await
        .expect("relay round-trip must not retain the app mutex");
    assert!(!daemon_id.is_empty());

    let slice = create_room_slice(&fixture.runtime, "room-manifest-during-workflow-dispatch");
    let manifest_waiting = fixture
        .runtime
        .leased_agent_operations
        .notify_on_next_acquire_for_tests("leased-agent-1");
    fixture
        .runtime
        .bind_room_environment_slice(
            crate::local::BindRoomEnvironmentSliceRequest {
                session_id: fixture.session_id.clone(),
                slice_ref: slice.id,
            },
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("Room bind should enqueue manifest sync during the pending relay request");
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        manifest_waiting.notified(),
    )
    .await
    .expect("manifest sync should queue behind the ordered prompt relay round-trip");
    assert!(
        fixture.priority_rx.try_recv().is_err(),
        "manifest sync must not overtake the pending workflow prompt request"
    );

    acknowledge_peer_response(
        &fixture,
        request_id,
        crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id: "worker-run-workflow-successor".to_string(),
            outcome: crate::session::PromptSubmissionOutcome::Started {
                prompt: promoted.clone(),
            },
        },
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if active_prompt_for(&fixture).await.is_some_and(|active| {
                active.id() == promoted.id()
                    && active.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("ordered sender should settle the promoted workflow prompt");

    let (manifest_request_id, manifest) = next_manifest_update(&mut fixture).await;
    assert!(manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, manifest_request_id).await;
}

#[tokio::test]
async fn rejected_promoted_workflow_head_is_not_reported_as_delivered() {
    let mut fixture = room_manifest_fixture().await;
    let successor = queue_workflow_successor_after_ordinary_prompt(&fixture).await;
    let workflow_id = successor.workflow_id;
    let ordinary_prompt_id = successor.ordinary_prompt_id;
    project_ordinary_completion(&fixture, ordinary_prompt_id).await;
    let home_prompt_id = active_prompt_for(&fixture)
        .await
        .expect("workflow successor should be active before remote submission")
        .id()
        .to_string();

    let (request_id, request) = next_peer_request(&mut fixture).await;
    let (workflow_run_id, workflow_node_run_id) = match request {
        crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
            workflow_context: Some(context),
            ..
        } => (context.workflow_run_id, context.workflow_node_run_id),
        other => panic!("expected workflow prompt submission, got {other:?}"),
    };
    crate::transport::relay_client::resolve_pending_peer_error_for_test(
        &fixture.relay_state,
        request_id,
        "worker-1".to_string(),
        chariox_relay::protocol::RelayError {
            code: "action_not_allowed".to_string(),
            message: "rejected workflow submission before worker admission".to_string(),
            retryable: false,
        },
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if active_prompt_for(&fixture).await.is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a rejected relay response should cancel the undispatched prompt");

    let host_daemon_id = fixture
        .runtime
        .owned
        .session_store
        .get_session(&fixture.session_id)
        .expect("session should remain available")
        .host_daemon_id()
        .to_string();
    let failed_workflow = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let terminal = fixture
                .runtime
                .owned
                .durable_state_store
                .resolve_workflow_run(&host_daemon_id, &fixture.session_id, &workflow_run_id)
                .ok()
                .flatten()
                .filter(|run| run.status() == crate::session::WorkflowRunStatus::Failed);
            if let Some(run) = terminal {
                break run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("rejected SubmitLeasedPrompt must fail the workflow run durably");
    assert_eq!(
        failed_workflow.status(),
        crate::session::WorkflowRunStatus::Failed
    );
    assert_eq!(
        failed_workflow.workflow_id(),
        workflow_id.as_str(),
        "the durable failure belongs to the exact queued workflow"
    );
    assert_eq!(
        failed_workflow.id(),
        workflow_run_id.as_str(),
        "the durable failure belongs to the run sent to the worker"
    );
    assert!(failed_workflow.node_runs().iter().any(|node_run| {
        node_run.id() == workflow_node_run_id.as_str()
            && node_run.status() == crate::session::WorkflowNodeRunStatus::Failed
    }));
    assert_eq!(
        failed_workflow
            .failure_events()
            .iter()
            .filter(|event| {
                event.kind() == crate::session::WorkflowFailureKind::TransportFailure
                    && event.source_node_run_id() == workflow_node_run_id.as_str()
            })
            .count(),
        1,
        "exact workflow and node run should retain one TransportFailure"
    );
    assert_eq!(failed_workflow.failure_events().len(), 1);
    assert!(
        failed_workflow
            .node_runs()
            .iter()
            .all(|node_run| node_run.status() != crate::session::WorkflowNodeRunStatus::Completed),
        "the rejected workflow must not record a completed node"
    );
    assert_eq!(failed_workflow.completed_by_node_run_id(), None);
    assert!(failed_workflow.final_output().is_none());
    let durable_history = fixture
        .runtime
        .owned
        .operational_history_store
        .load_session_events(&fixture.session_id, Some(&fixture.agent_id))
        .expect("workflow history should remain readable");
    let dispatch_provider_run_id = format!("remote-dispatch:{home_prompt_id}");
    assert!(
        durable_history.iter().any(|event| {
            event.kind == crate::history::HistoryEventKind::ProviderOutput
                && event.prompt_id.as_deref() == Some(home_prompt_id.as_str())
                && event.provider_run_id.as_deref() == Some(dispatch_provider_run_id.as_str())
                && event.workflow_id.as_deref() == Some(workflow_id.as_str())
                && event.workflow_run_id.as_deref() == Some(workflow_run_id.as_str())
                && event.workflow_node_id.as_deref() == Some(workflow_node_run_id.as_str())
        }),
        "dispatch-failure history must retain the exact workflow, node, and prompt identity"
    );
    assert!(!durable_history.iter().any(|event| {
        event.kind == crate::history::HistoryEventKind::WorkflowNodeCompleted
            && event.workflow_id.as_deref() == Some(workflow_id.as_str())
            && event.workflow_run_id.as_deref() == Some(workflow_run_id.as_str())
            && event.workflow_node_id.as_deref() == Some(workflow_node_run_id.as_str())
    }));

    let binding = fixture
        .runtime
        .owned
        .agent_store
        .get_agent(&fixture.agent_id)
        .expect("leased agent should remain available");
    assert_eq!(
        binding
            .remote_execution()
            .and_then(|remote| remote.active_worker_provider_run_id.as_deref()),
        None,
        "rejection must not retain a worker run for an unaccepted prompt"
    );
}

#[tokio::test]
async fn bind_and_delete_refresh_an_already_leased_agent_without_a_prompt() {
    let mut fixture = room_manifest_fixture().await;
    let slice = create_room_slice(&fixture.runtime, "room-manifest-bind-delete");
    fixture
        .runtime
        .bind_room_environment_slice(
            crate::local::BindRoomEnvironmentSliceRequest {
                session_id: fixture.session_id.clone(),
                slice_ref: slice.id.clone(),
            },
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("Room slice should bind");

    let (request_id, manifest) = next_manifest_update(&mut fixture).await;
    assert!(manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, request_id).await;

    fixture
        .runtime
        .delete_slice(&slice.id)
        .expect("bound Room slice should delete");
    let (request_id, manifest) = next_manifest_update(&mut fixture).await;
    assert!(!manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, request_id).await;
    assert!(fixture
        .runtime
        .room_environment_slice(&fixture.session_id)
        .expect("Room should remain available")
        .is_none());
}

#[tokio::test]
async fn bind_and_delete_enqueue_refresh_without_a_current_tokio_runtime() {
    let mut fixture = room_manifest_fixture().await;
    let slice = create_room_slice(&fixture.runtime, "room-manifest-no-runtime");
    let runtime = fixture.runtime.clone();
    let session_id = fixture.session_id.clone();
    let slice_ref = slice.id.clone();
    std::thread::spawn(move || {
        runtime.bind_room_environment_slice(
            crate::local::BindRoomEnvironmentSliceRequest {
                session_id,
                slice_ref,
            },
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
    })
    .join()
    .expect("bind thread should complete")
    .expect("Room slice should bind without a Tokio runtime");

    let (request_id, manifest) = next_manifest_update(&mut fixture).await;
    assert!(manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, request_id).await;

    let runtime = fixture.runtime.clone();
    let slice_ref = slice.id.clone();
    std::thread::spawn(move || runtime.delete_slice(&slice_ref))
        .join()
        .expect("delete thread should complete")
        .expect("bound Room slice should delete without a Tokio runtime");

    let (request_id, manifest) = next_manifest_update(&mut fixture).await;
    assert!(!manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, request_id).await;
}

#[tokio::test]
async fn delayed_bind_refresh_recomputes_after_delete_before_sending() {
    let mut fixture = room_manifest_fixture().await;
    let slice = create_room_slice(&fixture.runtime, "room-manifest-delayed-refresh");
    let lane = fixture
        .runtime
        .leased_agent_operations
        .lock("leased-agent-1")
        .await;
    let bind_waiting = fixture
        .runtime
        .leased_agent_operations
        .notify_on_next_acquire_for_tests("leased-agent-1");

    fixture
        .runtime
        .bind_room_environment_slice(
            crate::local::BindRoomEnvironmentSliceRequest {
                session_id: fixture.session_id.clone(),
                slice_ref: slice.id.clone(),
            },
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("Room slice should bind");
    tokio::time::timeout(std::time::Duration::from_secs(2), bind_waiting.notified())
        .await
        .expect("bind refresh should reach the per-agent lane");

    let delete_waiting = fixture
        .runtime
        .leased_agent_operations
        .notify_on_next_acquire_for_tests("leased-agent-1");
    fixture
        .runtime
        .delete_slice(&slice.id)
        .expect("bound Room slice should delete");
    tokio::time::timeout(std::time::Duration::from_secs(2), delete_waiting.notified())
        .await
        .expect("delete refresh should wait in the same per-agent lane");
    drop(lane);

    let (first_request, first_manifest) = next_manifest_update(&mut fixture).await;
    assert!(!first_manifest.room_browser_available);
    assert!(fixture.priority_rx.try_recv().is_err());
    acknowledge_manifest_update(&fixture, first_request).await;

    let (second_request, second_manifest) = next_manifest_update(&mut fixture).await;
    assert!(!second_manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, second_request).await;
}

#[tokio::test]
async fn paused_prompt_dispatch_and_provider_launch_serialize_with_room_bind_and_delete() {
    let mut fixture = room_manifest_fixture().await;
    let slice = create_room_slice(&fixture.runtime, "room-manifest-producer-ordering");
    let initial_agent = fixture
        .runtime
        .owned
        .agent_store
        .get_agent(&fixture.agent_id)
        .expect("leased agent should exist");
    assert!(
        !fixture
            .runtime
            .remote_extension_manifest_for_agent(&initial_agent)
            .expect("initial manifest should build")
            .room_browser_available
    );

    let prompt_dispatch = crate::app::KernelRemotePromptDispatch {
        session_id: fixture.session_id.clone(),
        agent_id: fixture.agent_id.clone(),
        prompt_id: "room-manifest-dispatch-prompt".to_string(),
        worker_kernel_id: "worker-1".to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        relay_url: None,
        relay_token: None,
        source_attachment_id: "room-manifest-dispatch-attachment".to_string(),
        prompt: "dispatch prompt".to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        workspace_live_sync_mode: None,
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        workflow_context: None,
    };
    let dispatch_state = fixture.runtime.clone();
    let dispatch = tokio::spawn(async move {
        dispatch_state
            .submit_remote_prompt_attempt(
                &prompt_dispatch,
                "dispatch prompt".to_string(),
                Vec::new(),
                None,
                "unexpected prompt test response",
            )
            .await
    });
    let (prompt_request_id, prompt_request) = next_peer_request(&mut fixture).await;
    let crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
        leased_agent_id,
        remote_extension_manifest,
        ..
    } = prompt_request
    else {
        panic!("expected the actual remote prompt sender request");
    };
    assert_eq!(leased_agent_id, "leased-agent-1");
    assert!(!remote_extension_manifest.room_browser_available);

    let bind_waiting = fixture
        .runtime
        .leased_agent_operations
        .notify_on_next_acquire_for_tests("leased-agent-1");
    fixture
        .runtime
        .bind_room_environment_slice(
            crate::local::BindRoomEnvironmentSliceRequest {
                session_id: fixture.session_id.clone(),
                slice_ref: slice.id.clone(),
            },
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("Room slice should bind while dispatch is paused");
    tokio::time::timeout(std::time::Duration::from_secs(2), bind_waiting.notified())
        .await
        .expect("bind refresh should wait behind the paused dispatch");
    assert!(fixture.priority_rx.try_recv().is_err());

    let prompt = crate::session::PromptQueueItem::new(
        "room-manifest-dispatch-prompt",
        "room-manifest-dispatch-attachment",
        &fixture.agent_id,
        "dispatch prompt",
        crate::session::PromptStatus::Running,
    );
    acknowledge_peer_response(
        &fixture,
        prompt_request_id,
        crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id: "worker-run-after-dispatch".to_string(),
            outcome: crate::session::PromptSubmissionOutcome::Started { prompt },
        },
    )
    .await;
    dispatch
        .await
        .expect("dispatch task should complete")
        .expect("actual remote prompt sender should succeed");
    let (bind_request, bind_manifest) = next_manifest_update(&mut fixture).await;
    assert!(bind_manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, bind_request).await;

    let stale_agent = fixture
        .runtime
        .owned
        .agent_store
        .get_agent(&fixture.agent_id)
        .expect("leased agent should remain available");
    let stale_launch_manifest = fixture
        .runtime
        .remote_extension_manifest_for_agent(&stale_agent)
        .expect("bound manifest should build");
    assert!(stale_launch_manifest.room_browser_available);

    let request = crate::local::LaunchProviderRunRequest {
        session_id: fixture.session_id.clone(),
        agent_id: Some(fixture.agent_id.clone()),
        adapter_key: "codex".to_string(),
        provider: "codex".to_string(),
        account_profile: "default".to_string(),
        model: "test-model".to_string(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: true,
    };
    let remote_execution = stale_agent
        .remote_execution()
        .cloned()
        .expect("agent should retain its lease");
    let held_lane = fixture
        .runtime
        .leased_agent_operations
        .lock("leased-agent-1")
        .await;
    let launch_waiting = fixture
        .runtime
        .leased_agent_operations
        .notify_on_next_acquire_for_tests("leased-agent-1");
    let launch_state = fixture.runtime.clone();
    let launch_agent_id = fixture.agent_id.clone();
    let launch = tokio::spawn(async move {
        launch_state
            .send_remote_native_provider_launch_attempt(
                &request,
                &launch_agent_id,
                &remote_execution,
                None,
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), launch_waiting.notified())
        .await
        .expect("actual provider launch sender should wait in the lease lane");

    let delete_waiting = fixture
        .runtime
        .leased_agent_operations
        .notify_on_next_acquire_for_tests("leased-agent-1");
    fixture
        .runtime
        .delete_slice(&slice.id)
        .expect("bound Room slice should delete while launch is paused");
    tokio::time::timeout(std::time::Duration::from_secs(2), delete_waiting.notified())
        .await
        .expect("delete refresh should enter the shared lease lane");
    drop(held_lane);

    let (launch_request_id, launch_request) = next_peer_request(&mut fixture).await;
    let crate::transport::relay_peer::RelayPeerRequest::LaunchLeasedNativeProviderRun {
        remote_extension_manifest,
        ..
    } = launch_request
    else {
        panic!("expected the actual provider launch sender request");
    };
    assert!(!remote_extension_manifest.room_browser_available);
    assert!(fixture.priority_rx.try_recv().is_err());
    acknowledge_peer_response(
        &fixture,
        launch_request_id,
        crate::transport::relay_peer::RelayPeerResponse::Pong {
            value: "launch request observed".to_string(),
            daemon_id: "worker-1".to_string(),
        },
    )
    .await;
    let launch_response = launch
        .await
        .expect("launch task should complete")
        .expect("actual provider launch sender should return its peer response");
    assert!(matches!(
        launch_response,
        crate::transport::relay_peer::RelayPeerResponse::Pong { ref value, ref daemon_id }
            if value == "launch request observed" && daemon_id == "worker-1"
    ));

    let (delete_request, delete_manifest) = next_manifest_update(&mut fixture).await;
    assert!(!delete_manifest.room_browser_available);
    acknowledge_manifest_update(&fixture, delete_request).await;
    assert!(fixture.priority_rx.try_recv().is_err());
}

#[tokio::test]
async fn failed_bind_and_delete_do_not_enqueue_manifest_refreshes() {
    let mut fixture = room_manifest_fixture().await;
    let missing_bind = fixture.runtime.bind_room_environment_slice(
        crate::local::BindRoomEnvironmentSliceRequest {
            session_id: fixture.session_id.clone(),
            slice_ref: "missing-room-slice".to_string(),
        },
        crate::session::DEFAULT_LOCAL_USER_ID,
    );
    assert!(missing_bind.is_err());

    let slice = create_room_slice(&fixture.runtime, "room-manifest-failed-delete");
    fixture
        .runtime
        .owned
        .slice_store
        .bind_environment(&fixture.session_id, &slice.id, 2, |_| Ok(()))
        .expect("test should bind the slice directly");
    fixture
        .runtime
        .owned
        .slice_store
        .attach_agent(&slice.id, &fixture.session_id, &fixture.agent_id, 3)
        .expect("test should mark the slice as having an active agent");
    assert!(fixture.runtime.delete_slice(&slice.id).is_err());
    assert!(fixture
        .runtime
        .room_environment_slice(&fixture.session_id)
        .expect("Room should remain available")
        .is_some());
    assert!(fixture.priority_rx.try_recv().is_err());
}
