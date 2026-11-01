use super::*;

#[test]
fn native_refresh_pending_retries_only_the_exact_pre_admission_relay_error() {
    let worker = DaemonError::LocalTransport {
        operation: "remote runtime tool catalog reload",
        message: "native_runtime_catalog_refresh_pending: refresh needed".into(),
    };
    assert!(remote_prompt_error_should_retry_transport(&worker));
    for operation in [
        "read relay peer response",
        "read temporary relay peer response",
    ] {
        let transmitted = DaemonError::LocalTransport {
            operation,
            message: worker.to_string(),
        };
        assert!(remote_prompt_error_should_retry_transport(&transmitted));
    }
    assert!(!remote_prompt_error_should_retry_transport(
        &DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: worker.to_string(),
        }
    ));
}

#[test]
fn launch_credential_retry_requires_the_typed_worker_diagnostic() {
    use crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE;

    for operation in [
        "read relay peer response",
        "read temporary relay peer response",
    ] {
        let required = DaemonError::RelayTransport {
            operation,
            code: REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE.to_string(),
            message: "worker requested a credential for a cold provider launch".to_string(),
            retryable: false,
        };
        assert!(remote_prompt_error_requires_provider_launch_credential(
            &required
        ));
    }
    let unrelated = DaemonError::LocalTransport {
        operation: "read relay peer response",
        message: "worker unavailable".to_string(),
    };
    assert!(!remote_prompt_error_requires_provider_launch_credential(
        &unrelated
    ));
    let untyped_required = DaemonError::LocalTransport {
        operation: "read relay peer response",
        message: format!("{REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE}: relaunch"),
    };
    assert!(!remote_prompt_error_requires_provider_launch_credential(
        &untyped_required
    ));
}
use std::sync::Arc;

use crate::app::{DaemonApp, KernelPreparedPromptSubmission};
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};
use chariox_relay::protocol::RelayEnvelope;
use tokio::sync::Mutex;

const CANCELLATION_GUARD_RELAY_URL: &str = "ws://127.0.0.1:43179";
const CANCELLATION_GUARD_WORKER_ID: &str = "worker-cancel-before-submit";
const CANCELLATION_GUARD_LEASED_AGENT_ID: &str = "leased-agent-cancel-before-submit";

async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
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
        let app_locked = app.lock().await;
        (
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.prompt_state_owner(),
            app_locked.active_turn_store(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workflow_design_event_store(),
            app_locked.metaagent_event_store(),
            app_locked.workspace_coordinator(),
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

async fn receive_fake_worker_request(
    receiver: &mut tokio::sync::mpsc::Receiver<RelayEnvelope>,
    worker_private_key: &str,
) -> (String, crate::transport::relay_peer::RelayPeerRequest) {
    let envelope = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv())
        .await
        .expect("fake relay should receive the peer request")
        .expect("fake relay request channel should remain open");
    let RelayEnvelope::DaemonPeerRequest {
        request_id,
        encrypted_request,
        ..
    } = envelope
    else {
        panic!("expected a daemon peer request");
    };
    let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
        worker_private_key,
        &encrypted_request,
    )
    .expect("fake worker should decrypt the peer request");
    let request =
        serde_json::from_slice(&decrypted.plaintext).expect("fake worker request should decode");
    (request_id, request)
}

#[tokio::test]
async fn cancelling_accepted_remote_prompt_never_sends_submit_request() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(CANCELLATION_GUARD_RELAY_URL.to_string());
    config.relay_token = Some("cancel-before-submit-test-token".to_string());
    let worker_config = crate::config::DaemonConfig::for_tests();

    let mut app = DaemonApp::bootstrap(config).expect("home app should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "cancel-before-submit-workspace",
            "cancel-before-submit-worktree",
        ))
        .expect("home session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "cancel-before-submit-client",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("home attachment should be created");
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.to_string(),
                worker_machine_id: "worker-machine-cancel-before-submit".to_string(),
                execution_lease_id: "lease-cancel-before-submit".to_string(),
                leased_agent_id: CANCELLATION_GUARD_LEASED_AGENT_ID.to_string(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .expect("home agent should bind to the fake worker");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let mut submission = runtime
        .owned
        .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: PromptQueueItem::new(
                "pending:cancel-before-submit",
                &attachment_id,
                &agent_id,
                "this accepted prompt is cancelled before dispatch",
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
    assert_eq!(
        runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .expect("session should remain available"),
                &agent_id,
            )
            .expect("accepted prompt should remain active")
            .durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Accepted)
    );
    runtime
        .owned
        .begin_remote_prompt_cancellation(&session_id, &agent_id, &attachment_id)
        .expect("cancellation intent should persist before dispatch begins");

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
    let worker_private_key = worker_config.relay_private_key.clone();
    let prompt = dispatch.prompt.clone();
    let submission = submit_remote_prompt_to_worker_with_binding_refresh(
        &runtime,
        &mut dispatch,
        prompt,
        Vec::new(),
    );
    tokio::pin!(submission);
    tokio::select! {
        result = &mut submission => {
            let error = result.expect_err("cancelled Accepted prompt must not be submitted");
            assert!(error.to_string().contains("cannot start remote dispatch"));
            assert!(
                peer_requests.try_recv().is_err(),
                "cancelled Accepted prompt must send no relay request"
            );
        }
        envelope = peer_requests.recv() => {
            let envelope = envelope.expect("fake relay should receive any unexpected request");
            let RelayEnvelope::DaemonPeerRequest {
                encrypted_request,
                ..
            } = envelope else {
                panic!("unexpected fake relay envelope: {envelope:?}");
            };
            let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
                &worker_private_key,
                &encrypted_request,
            )
            .expect("fake worker should decrypt an unexpected request");
            let request: crate::transport::relay_peer::RelayPeerRequest =
                serde_json::from_slice(&decrypted.plaintext)
                    .expect("fake worker request should decode");
            assert!(
                matches!(
                    request,
                    crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt { .. }
                ),
                "unexpected relay request before SubmitLeasedPrompt: {request:?}"
            );
            panic!(
                "cancelled Accepted prompt sent SubmitLeasedPrompt before cancellation settled"
            );
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
            panic!("dispatch neither rejected cancellation nor produced a test relay request");
        }
    }
}

#[tokio::test]
async fn connected_held_prompt_response_does_not_block_remote_manifest_sync() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(CANCELLATION_GUARD_RELAY_URL.to_string());
    config.relay_token = Some("manifest-lane-test-token".to_string());
    let home_public_key = config.relay_public_key.clone();
    let worker_config = crate::config::DaemonConfig::for_tests();

    let mut app = DaemonApp::bootstrap(config).expect("home app should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "manifest-lane-workspace",
            "manifest-lane-worktree",
        ))
        .expect("home session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "manifest-lane-client",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("home attachment should be created");
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.to_string(),
                worker_machine_id: "worker-machine-manifest-lane".to_string(),
                execution_lease_id: "lease-manifest-lane".to_string(),
                leased_agent_id: CANCELLATION_GUARD_LEASED_AGENT_ID.to_string(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .expect("home agent should bind to the fake worker");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let mut submission = runtime
        .owned
        .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: PromptQueueItem::new(
                "pending:manifest-lane",
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
    assert!(
        matches!(
            &request,
            crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt { .. }
        ),
        "first fake relay request should submit the held prompt: {request:?}"
    );

    let sync_runtime = runtime.clone();
    let sync_agent = runtime
        .owned
        .agent_store
        .get_agent(&agent_id)
        .expect("home agent should remain available");
    let sync_task = tokio::spawn(async move {
        sync_runtime
            .sync_remote_extension_manifest_for_agent(&sync_agent, None, None)
            .await
    });

    let (manifest_request_id, request) =
        receive_fake_worker_request(&mut peer_requests, &worker_config.relay_private_key).await;
    assert_ne!(manifest_request_id, submit_request_id);
    let crate::transport::relay_peer::RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
        leased_agent_id,
        ..
    } = request
    else {
        panic!("manifest sync should proceed while prompt response is held: {request:?}");
    };
    assert_eq!(leased_agent_id, CANCELLATION_GUARD_LEASED_AGENT_ID);

    let manifest_response = crate::transport::relay_peer::RelayPeerResponse::LeasedAgentRemoteExtensionManifestUpdated {
        leased_agent_id: leased_agent_id.clone(),
    };
    let encrypted_manifest_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &worker_config.relay_private_key,
        &home_public_key,
        &serde_json::to_vec(&manifest_response).expect("manifest response should encode"),
    )
    .expect("fake worker should encrypt the manifest response");
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &relay_state,
        manifest_request_id,
        CANCELLATION_GUARD_WORKER_ID.to_string(),
        encrypted_manifest_response,
    )
    .await;
    sync_task
        .await
        .expect("manifest sync task should not panic")
        .expect("manifest sync should succeed before prompt response is released");
    assert!(
        !submit_task.is_finished(),
        "the fake worker must still be holding the prompt response"
    );

    let submit_response = crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
        provider_run_id: "provider-run-manifest-lane".to_string(),
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
        "provider-run-manifest-lane"
    );
}

// MP-08/MP-10/MP-11: shared PromptOrigin is also part of the peer contract.
#[tokio::test]
async fn trigger_origins_cross_only_the_versioned_peer_boundary() {
    for origin in [
        crate::session::PromptOrigin::Schedule,
        crate::session::PromptOrigin::Notification,
    ] {
        trigger_origin_peer_submission(
            origin,
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            true,
        )
        .await;
        trigger_origin_peer_submission(origin, 239, false).await;
    }
}

async fn trigger_origin_peer_submission(
    origin: crate::session::PromptOrigin,
    version: u32,
    supported: bool,
) {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(CANCELLATION_GUARD_RELAY_URL.to_string());
    config.relay_token = Some("manifest-lane-test-token".to_string());
    let home_public_key = config.relay_public_key.clone();
    let worker_config = crate::config::DaemonConfig::for_tests();

    let mut app = DaemonApp::bootstrap(config).expect("home app should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "manifest-lane-workspace",
            "manifest-lane-worktree",
        ))
        .expect("home session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "manifest-lane-client",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("home attachment should be created");
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.to_string(),
                worker_machine_id: "worker-machine-manifest-lane".to_string(),
                execution_lease_id: "lease-manifest-lane".to_string(),
                leased_agent_id: CANCELLATION_GUARD_LEASED_AGENT_ID.to_string(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(version),
            },
        )
        .expect("home agent should bind to the fake worker");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let mut submission = runtime
        .owned
        .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
            session_id: session_id.clone(),
            prompt: PromptQueueItem::new(
                "pending:manifest-lane",
                &attachment_id,
                &agent_id,
                "hold the worker response while manifest sync runs",
                PromptStatus::Queued,
            )
            .with_prompt_origin(origin),
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

    if !supported {
        tokio::select! {
            result = submit_task => {
                let error = result.unwrap().expect_err("legacy worker must refuse new trigger origins");
                assert!(error.to_string().contains("incompatible or legacy relay peer protocol"));
                assert!(peer_requests.try_recv().is_err(), "no request may cross the unsupported peer boundary");
            }
            _ = peer_requests.recv() => panic!("legacy worker was sent a trigger origin it cannot deserialize"),
            _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => panic!("legacy worker refusal timed out"),
        }
        return;
    }
    let (submit_request_id, request) =
        receive_fake_worker_request(&mut peer_requests, &worker_config.relay_private_key).await;
    let crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
        git_context: Some(context),
        ..
    } = request
    else {
        panic!("trigger submission must carry its origin to the worker");
    };
    assert_eq!(context.prompt_origin, Some(origin));
    let value = serde_json::to_value(&context).unwrap();
    assert_eq!(
        value["prompt_origin"],
        serde_json::to_value(origin).unwrap()
    );
    let round_trip: crate::transport::relay_peer::RemoteGitTurnContext =
        serde_json::from_value(value).unwrap();
    assert_eq!(round_trip.prompt_origin, Some(origin));
    let submit_response = crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
        provider_run_id: "provider-run-manifest-lane".to_string(),
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
        "provider-run-manifest-lane"
    );
}

const WORKFLOW_CREDENTIAL_CANARY: &str = "workflow-submit-token-canary";

#[derive(Clone, Copy)]
enum WorkflowCredentialState {
    MissingVault,
    MissingCredential,
    Locked,
    Unlocked,
    ActiveWorkerRun,
}

struct WorkflowSubmissionFixture {
    runtime: KernelRuntimeState,
    dispatch: crate::app::KernelRemotePromptDispatch,
    root: std::path::PathBuf,
    previous_home: Option<std::ffi::OsString>,
}

impl WorkflowSubmissionFixture {
    fn new(credential_state: WorkflowCredentialState) -> Self {
        use crate::app::KernelSessionService;
        use crate::attachment::{AttachRequest, ClientCapabilityLevel};
        use crate::config::{CredentialVaultBackend, DaemonConfig};
        use crate::session::{CreateSessionRequest, DEFAULT_LOCAL_USER_ID};
        use std::sync::Arc;

        static NEXT_FIXTURE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let fixture_id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "chariox-remote-workflow-submit-credential-{}-{}-{fixture_id}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let previous_home = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", &root);
        crate::secret::clear_vault_secret_process_cache().unwrap();

        let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(root.join("operations.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        config.user_config.credential_vault.backend = CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path =
            root.join("credentials.vault").display().to_string();
        // A missing relay makes reaching transport distinguishable from failing credential
        // admission, without sending a SubmitLeasedPrompt to any worker.
        config.relay_url = None;
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, _) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                root.to_string_lossy(),
                root.to_string_lossy(),
            ))
            .unwrap();
        let profile = app
            .provider_account_profile_registry()
            .create_managed(DEFAULT_LOCAL_USER_ID, "claude", "Workflow submit fixture")
            .unwrap();
        crate::test_support::authenticate_provider_account(
            &app.provider_account_profile_registry(),
            DEFAULT_LOCAL_USER_ID,
            "claude",
            &profile.profile_id,
        )
        .expect("workflow fixture account should be authenticated");
        let agent = KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "claude")
                    .with_account_profile(profile.profile_id.clone()),
            )
            .unwrap();
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "workflow-submit-credential-test",
                ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let vault_path = root.join("credentials.vault");
        if matches!(credential_state, WorkflowCredentialState::MissingCredential) {
            crate::secret::unlock_chariox_encrypted_vault(
                &vault_path,
                "fixture passphrase",
                crate::secret::VaultUnlockLease::KernelShutdown,
            )
            .unwrap();
        } else if matches!(
            credential_state,
            WorkflowCredentialState::Locked | WorkflowCredentialState::Unlocked
        ) {
            crate::secret::unlock_chariox_encrypted_vault(
                &vault_path,
                "fixture passphrase",
                crate::secret::VaultUnlockLease::KernelShutdown,
            )
            .unwrap();
            crate::provider::store_provider_account_credential(
                app.config(),
                DEFAULT_LOCAL_USER_ID,
                "claude",
                &profile.profile_id,
                WORKFLOW_CREDENTIAL_CANARY,
                false,
            )
            .unwrap();
            if matches!(credential_state, WorkflowCredentialState::Locked) {
                crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
                crate::secret::clear_vault_secret_process_cache().unwrap();
            }
        }
        app.agents()
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "workflow-worker".into(),
                    worker_machine_id: "workflow-worker-machine".into(),
                    execution_lease_id: "workflow-lease".into(),
                    leased_agent_id: "workflow-leased-agent".into(),
                    active_worker_provider_run_id: matches!(
                        credential_state,
                        WorkflowCredentialState::ActiveWorkerRun
                    )
                    .then(|| "already-active-worker-run".into()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();

        let app = Arc::new(tokio::sync::Mutex::new(app));
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            app,
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
        );
        let runtime = router.runtime_state();
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let submission = runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: crate::session::PromptQueueItem::new(
                    "pending:workflow-submit-credential",
                    &attachment_id,
                    &agent_id,
                    "workflow prompt credential admission",
                    crate::session::PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .unwrap()
            .unwrap();
        let mut dispatch = submission.remote_dispatch.unwrap();
        dispatch.workflow_context = Some(crate::execution_lease::RemoteWorkflowTurnContext {
            home_kernel_id: "home-kernel".into(),
            home_session_id: session_id,
            home_agent_id: agent_id,
            workflow_run_id: "workflow-run".into(),
            workflow_node_run_id: "workflow-node-run".into(),
            delivery_token: "workflow-delivery-token".into(),
        });
        Self {
            runtime,
            dispatch,
            root,
            previous_home,
        }
    }

    async fn submit(&mut self) -> Result<String, DaemonError> {
        let prompt = self.dispatch.prompt.clone();
        submit_remote_prompt_to_worker_with_binding_refresh(
            &self.runtime,
            &mut self.dispatch,
            prompt,
            Vec::new(),
        )
        .await
    }

    async fn submit_after_rejecting_vault_unlock(&self) -> Result<String, DaemonError> {
        let runtime = self.runtime.clone();
        let mut dispatch = self.dispatch.clone();
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let prompt = dispatch.prompt.clone();
        let submission = submit_remote_prompt_to_worker_with_binding_refresh(
            &runtime,
            &mut dispatch,
            prompt,
            Vec::new(),
        );
        tokio::pin!(submission);
        let mut completed_submission = None;
        let interaction = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let session = runtime
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .expect("session should remain available");
                if let Some(interaction) = session.active_interaction_for_agent(&agent_id) {
                    break Some(interaction.clone());
                }
                tokio::select! {
                    result = &mut submission => {
                        completed_submission = Some(result);
                        return None;
                    }
                    _ = tokio::task::yield_now() => {}
                }
            }
        })
        .await
        .expect("locked vault should request a passphrase before transport");
        if let Some(result) = completed_submission {
            return result;
        }
        let interaction = interaction.expect("vault interaction should be observed");
        assert_eq!(interaction.title(), Some("Unlock Chariox Vault"));
        runtime
            .answer_terminal_runtime_interaction(
                &session_id,
                interaction.id(),
                "cancel",
                None,
                Some(
                    runtime
                        .owned
                        .session_store
                        .get_session(&session_id)
                        .unwrap()
                        .owner_user_id(),
                ),
                None,
                None,
                Some(crate::local::KernelConnectionClass::Terminal),
            )
            .await
            .expect("vault cancellation should resolve");
        submission.await
    }
}

impl Drop for WorkflowSubmissionFixture {
    fn drop(&mut self) {
        let _ = crate::secret::lock_chariox_encrypted_vault(self.root.join("credentials.vault"));
        let _ = crate::secret::clear_vault_secret_process_cache();
        match self.previous_home.take() {
            Some(value) => std::env::set_var("CHARIOX_HOME", value),
            None => std::env::remove_var("CHARIOX_HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn classify_error(
    error: DaemonError,
    request_transport_started: bool,
) -> RemotePromptSubmissionOutcome {
    classify_remote_prompt_submission_outcome(Err(error), request_transport_started)
}

#[test]
fn remote_prompt_submission_classifies_pre_send_rejection_and_indeterminate_transport() {
    let definitely_pre_send = classify_error(
        DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "relay is not connected".to_string(),
        },
        true,
    );
    assert!(matches!(
        &definitely_pre_send,
        RemotePromptSubmissionOutcome::DefinitelyPreSend(error)
            if remote_prompt_error_should_retry_transport(error)
    ));

    let known_rejection = classify_error(
        DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "leased_agent_not_found".to_string(),
            message: "worker rejected the stale lease before admission".to_string(),
            retryable: false,
        },
        true,
    );
    assert!(matches!(
        known_rejection,
        RemotePromptSubmissionOutcome::RejectedBeforeAdmission(_)
    ));

    let relay_admission_rejection = classify_error(
        DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "target_not_connected".to_string(),
            message: "target daemon is not connected to relay".to_string(),
            retryable: true,
        },
        true,
    );
    assert!(matches!(
        &relay_admission_rejection,
        RemotePromptSubmissionOutcome::RejectedBeforeAdmission(error)
            if remote_prompt_error_should_retry_transport(error)
    ));

    let response_timeout = classify_error(
        DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "target_disconnected".to_string(),
            message: "worker disconnected while awaiting response".to_string(),
            retryable: true,
        },
        true,
    );
    assert!(matches!(
        response_timeout,
        RemotePromptSubmissionOutcome::Indeterminate(_)
    ));

    let partial_write = classify_error(
        DaemonError::LocalTransport {
            operation: "write temporary relay peer request",
            message: "connection closed during websocket write".to_string(),
        },
        true,
    );
    assert!(matches!(
        partial_write,
        RemotePromptSubmissionOutcome::Indeterminate(_)
    ));

    let setup_failure = classify_error(
        DaemonError::LocalTransport {
            operation: "prepare remote prompt manifest",
            message: "local validation failed before network submission".to_string(),
        },
        false,
    );
    assert!(matches!(
        setup_failure,
        RemotePromptSubmissionOutcome::DefinitelyPreSend(_)
    ));
}

#[test]
fn remote_prompt_dispatch_holds_after_worker_timeout() {
    let outcome = classify_error(
        DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: "remote prompt dispatch timed out waiting for worker response".to_string(),
        },
        true,
    );

    assert!(matches!(
        outcome,
        RemotePromptSubmissionOutcome::Indeterminate(_)
    ));
}

#[test]
fn remote_prompt_dispatch_does_not_trust_missing_lease_text_after_send() {
    let outcome = classify_error(
        DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: "leased_agent_not_found".to_string(),
        },
        true,
    );

    assert!(matches!(
        outcome,
        RemotePromptSubmissionOutcome::Indeterminate(_)
    ));
}

#[test]
fn remote_prompt_dispatch_refreshes_binding_for_structured_missing_lease_errors() {
    let outcome = classify_error(
        DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "leased_agent_not_found".to_string(),
            message: "the leased agent was not found".to_string(),
            retryable: false,
        },
        true,
    );

    assert!(matches!(
        outcome,
        RemotePromptSubmissionOutcome::RejectedBeforeAdmission(error)
            if remote_prompt_error_should_refresh_binding(&error)
    ));
}

#[test]
fn remote_prompt_dispatch_retries_with_a_credential_only_when_worker_requests_it() {
    let unstructured = classify_error(
        DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: format!(
                "transport error: {}: worker run was lost",
                crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE,
            ),
        },
        true,
    );
    assert!(matches!(
        unstructured,
        RemotePromptSubmissionOutcome::Indeterminate(_)
    ));

    let unrelated = classify_error(
        DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: "worker run was lost".to_string(),
        },
        true,
    );
    assert!(matches!(
        unrelated,
        RemotePromptSubmissionOutcome::Indeterminate(_)
    ));

    let structured = classify_error(
        DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE
                .to_string(),
            message: "worker requires a launch credential".to_string(),
            retryable: false,
        },
        true,
    );
    assert!(matches!(
        structured,
        RemotePromptSubmissionOutcome::RejectedBeforeAdmission(error)
            if remote_prompt_error_requires_provider_launch_credential(&error)
    ));
}

#[test]
fn remote_prompt_dispatch_retries_disconnected_relay_targets() {
    let error = DaemonError::LocalTransport {
        operation: "read relay peer response",
        message: "target daemon is not connected to relay".to_string(),
    };

    assert!(remote_prompt_error_should_retry_transport(&error));
}

#[test]
fn remote_prompt_dispatch_retries_relay_reported_disconnected_targets() {
    let error = DaemonError::LocalTransport {
        operation: "read relay peer response",
        message: "target daemon disconnected from relay".to_string(),
    };

    assert!(remote_prompt_error_should_retry_transport(&error));
}

#[test]
fn remote_prompt_dispatch_retries_temporary_relay_responses() {
    let error = DaemonError::LocalTransport {
        operation: "read temporary relay peer response",
        message: "target daemon disconnected from relay".to_string(),
    };

    assert!(remote_prompt_error_should_retry_transport(&error));
}

#[test]
fn remote_prompt_dispatch_retries_structured_temporary_relay_responses() {
    let error = DaemonError::RelayTransport {
        operation: "read temporary relay peer response",
        code: "target_disconnected".to_string(),
        message: "target daemon disconnected from relay".to_string(),
        retryable: true,
    };

    assert!(remote_prompt_error_should_retry_transport(&error));
}

#[test]
fn remote_prompt_dispatch_does_not_retry_relay_authorization_failures() {
    let error = DaemonError::LocalTransport {
        operation: "read relay peer response",
        message: "invalid relay token".to_string(),
    };

    assert!(!remote_prompt_error_should_retry_transport(&error));
}

#[test]
fn remote_prompt_dispatch_does_not_retry_structured_relay_authorization_failures() {
    let error = DaemonError::RelayTransport {
        operation: "read relay peer response",
        code: "invalid_relay_token".to_string(),
        message: "invalid relay token".to_string(),
        retryable: false,
    };

    assert!(!remote_prompt_error_should_retry_transport(&error));
}

#[test]
fn remote_prompt_transport_retry_window_is_bounded() {
    assert!(!remote_prompt_transport_retry_window_expired(
        REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW - std::time::Duration::from_millis(1),
    ));
    assert!(remote_prompt_transport_retry_window_expired(
        REMOTE_PROMPT_TRANSPORT_RETRY_WINDOW,
    ));
}

#[test]
fn remote_prompt_dispatch_does_not_retry_stopped_slice_forever() {
    assert!(!remote_prompt_slice_status_allows_transport_retry(
        &crate::slice::SliceStatus::Stopped,
    ));
    assert!(!remote_prompt_slice_status_allows_transport_retry(
        &crate::slice::SliceStatus::Stopping,
    ));
    assert!(!remote_prompt_slice_status_allows_transport_retry(
        &crate::slice::SliceStatus::Unhealthy,
    ));
    assert!(remote_prompt_slice_status_allows_transport_retry(
        &crate::slice::SliceStatus::Starting,
    ));
    assert!(remote_prompt_slice_status_allows_transport_retry(
        &crate::slice::SliceStatus::Running,
    ));
}

#[test]
fn leased_prompt_submit_timeout_covers_codex_mcp_retry_window() {
    assert!(
        crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT
            > std::time::Duration::from_secs(180)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn remote_workflow_submit_rejects_missing_vault_credential_before_transport() {
    crate::test_support::isolated_env_test!();
    let mut fixture = WorkflowSubmissionFixture::new(WorkflowCredentialState::MissingCredential);
    assert!(fixture.dispatch.workflow_context.is_some());
    let error = fixture
        .submit()
        .await
        .expect_err("workflow submit must reject a missing credential before transport");
    assert!(error.to_string().contains("remote Claude launch requires"));
    assert!(!error.to_string().contains("relay_url is not configured"));
    assert!(!format!("{error:?}").contains(WORKFLOW_CREDENTIAL_CANARY));
}

#[tokio::test(flavor = "current_thread")]
async fn remote_workflow_submit_rejects_missing_or_locked_vault_before_transport() {
    crate::test_support::isolated_env_test!();
    for credential_state in [
        WorkflowCredentialState::MissingVault,
        WorkflowCredentialState::Locked,
    ] {
        let fixture = WorkflowSubmissionFixture::new(credential_state);
        assert!(fixture.dispatch.workflow_context.is_some());
        let error = fixture
            .submit_after_rejecting_vault_unlock()
            .await
            .expect_err("rejected vault unlock must stop before relay transport");
        assert!(
            crate::secret::is_chariox_vault_locked_error(&error)
                || error.to_string().contains("vault unlock was cancelled")
                || error.to_string().contains("remote Claude launch requires"),
            "missing or locked vault should reject before sending: {error}"
        );
        assert!(!error.to_string().contains("relay_url is not configured"));
        assert!(!format!("{error:?}").contains(WORKFLOW_CREDENTIAL_CANARY));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn remote_workflow_submit_admits_vaulted_token_or_active_worker_run_without_exposing_it() {
    crate::test_support::isolated_env_test!();
    let _claude = crate::test_support::TestClaudeExecutable::new();
    for credential_state in [
        WorkflowCredentialState::Unlocked,
        WorkflowCredentialState::ActiveWorkerRun,
    ] {
        let mut fixture = WorkflowSubmissionFixture::new(credential_state);
        assert!(fixture.dispatch.workflow_context.is_some());
        let error = fixture
            .submit()
            .await
            .expect_err("fixture relay is absent after credential admission");
        assert!(
            error.to_string().contains("relay_url is not configured"),
            "credential admission should reach the transport boundary: {error}"
        );
        assert!(!error.to_string().contains(WORKFLOW_CREDENTIAL_CANARY));
        assert!(!format!("{error:?}").contains(WORKFLOW_CREDENTIAL_CANARY));
    }
}

#[tokio::test]
async fn leased_prompt_uses_shared_handoff_dispatch_before_provider_submission() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("worker boot");
    let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "handoff-home",
            "handoff-session",
            "handoff-agent",
            false,
            "handoff-user",
        )
        .expect("lease");
    let leased = crate::app::RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".into()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent");
    let entry = crate::history::SessionHistoryEntry::user_prompt(
        &leased.backing_session_id,
        &leased.backing_attachment_id,
        &leased.backing_agent_id,
        "Our earlier accepted constraint is PRESERVE_LEASED_CONVERSATION.",
    );
    let event = crate::history::HistoryEvent::transcript(
        app.operational_history_store().reserve_sequence(),
        &entry,
        crate::history::HistoryEventTurnContext {
            prompt_id: Some("earlier-worker-prompt".into()),
            ..Default::default()
        },
    );
    app.operational_history_store()
        .append(&event)
        .expect("earlier history");
    // Exercise submission to a ready worker run, rather than the separate
    // asynchronous launch path that admits this first prompt into its queue.
    let prepared = crate::app::RemoteLeaseRuntime::new(&mut app)
        .prepare_leased_prompt_submission(
            &leased.id,
            "Continue the investigation",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("prepare ready worker");
    if let crate::app::PreparedLeasedProviderRun::LaunchRequired(request) = prepared.provider_run {
        app.launch_leased_provider(request)
            .expect("ready worker run");
    }
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let (run, outcome) = runtime
        .submit_relay_leased_prompt(
            &leased.id,
            crate::transport::relay_peer::RelayAgentExecutionProfile {
                provider: leased.provider.clone(),
                account_profile: leased.account_profile.clone(),
                model: leased.model.clone(),
                effort: leased.effort.clone(),
            },
            None,
            "Continue the investigation",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
        .await
        .expect("worker submission");
    assert!(
        matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Started { .. }
        ),
        "ready worker must dispatch immediately"
    );
    let records = runtime.owned.terminal_stream.input_records();
    assert!(
        records.iter().any(|r| r.provider_run_id == run
            && String::from_utf8_lossy(&r.bytes).contains("PRESERVE_LEASED_CONVERSATION")),
        "worker submission must carry its earlier conversation through the shared dispatcher"
    );
    let session = runtime
        .owned
        .session_store
        .get_session(&leased.backing_session_id)
        .expect("session");
    assert_eq!(
        runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &leased.backing_agent_id)
            .expect("active")
            .prompt(),
        "Continue the investigation"
    );
}

#[path = "remote_prompt_worker_submission_runtime_handoff_tests.rs"]
mod handoff;

#[test]
fn a_worker_account_refusal_fails_the_prompt_instead_of_reconciling() {
    let refused = classify_error(
        DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "provider_account_unusable".to_string(),
            message: "selected claude account `work` is not authenticated".to_string(),
            retryable: false,
        },
        true,
    );
    assert!(matches!(
        &refused,
        RemotePromptSubmissionOutcome::RejectedBeforeAdmission(error)
            if !remote_prompt_error_should_retry_transport(error)
                && !remote_prompt_error_should_refresh_binding(error)
    ));
}

// MP-08/MP-10/MP-11: failed idle admission must retry through the real peer
// dispatch path, rather than consuming a resend with no dispatch left to run.
#[tokio::test]
async fn trigger_resend_idle_remote_admission_failure_retries_one_peer_submission() {
    let mut config = super::super::agent_trigger_runtime_state::tests::durable_state_config(
        "resend-remote-admission-failure",
    );
    config.relay_url = Some(CANCELLATION_GUARD_RELAY_URL.into());
    config.relay_token = Some("resend-worker-test-token".into());
    let (_worktree, runtime, config, session_id, agent_id, _) =
        super::super::agent_trigger_runtime_state::tests::durable_unconfirmed_trigger_delivery_with_config(config).await;
    let session = runtime.owned.session_snapshot(&session_id).unwrap();
    runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&session, &agent_id)
        .unwrap();
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, &agent_id);
    runtime
        .owned
        .mirror_prompt_owner_agent_state(&session_id, &agent_id, active, queued)
        .unwrap();
    runtime
        .owned
        .agent_store
        .write()
        .bind_remote_execution(
            &agent_id,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: CANCELLATION_GUARD_WORKER_ID.into(),
                worker_machine_id: "resend-worker-machine".into(),
                execution_lease_id: "resend-worker-lease".into(),
                leased_agent_id: CANCELLATION_GUARD_LEASED_AGENT_ID.into(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let worker_config = crate::config::DaemonConfig::for_tests();
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
    let owner = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
    let request = crate::local::ResendTriggerDeliveryRequest {
        session_id: session_id.clone(),
        prompt_id: "original-firing".into(),
    };
    let connection = rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_remote_resend BEFORE INSERT ON durable_trigger_resends
             BEGIN SELECT RAISE(FAIL, 'injected remote resend admission failure'); END;",
        )
        .unwrap();
    let error = runtime
        .resend_trigger_delivery(request.clone(), owner.owner_user_id())
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("injected remote resend admission failure"));
    assert!(peer_requests.try_recv().is_err());
    let session = runtime.owned.session_snapshot(&session_id).unwrap();
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&session, &agent_id);
    assert!(active.is_none() && queued.is_empty());
    connection
        .execute_batch("DROP TRIGGER reject_remote_resend;")
        .unwrap();
    let retry_runtime = runtime.clone();
    let retry_request = request.clone();
    let owner_user_id = owner.owner_user_id().to_string();
    let retry = tokio::spawn(async move {
        retry_runtime
            .resend_trigger_delivery(retry_request, &owner_user_id)
            .await
    });
    retry.await.unwrap().unwrap();
    let current = runtime.owned.session_snapshot(&session_id).unwrap();
    let pending = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&current, &agent_id)
        .expect("idle resend must admit a dispatchable prompt");
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while peer_requests.is_empty() { tokio::task::yield_now().await; }
    }).await.unwrap_or_else(|_| panic!("resend dispatch stalled at {}; interaction {:?}",
        super::super::remote_prompt_dispatch_execution_runtime::remote_prompt_dispatch_test_stages(pending.id()),
        runtime.owned.session_snapshot(&session_id).unwrap().active_interaction_for_agent(&agent_id).map(|item| item.title())));
    let (request_id, received) =
        receive_fake_worker_request(&mut peer_requests, &worker_config.relay_private_key).await;
    let crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
        git_context: Some(context),
        prompt,
        ..
    } = received
    else {
        panic!("retry must submit one replacement to the worker");
    };
    assert_eq!(
        context.prompt_origin,
        Some(crate::session::PromptOrigin::Schedule)
    );
    let response = crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
        provider_run_id: "resend-worker-run".into(),
        outcome: crate::session::PromptSubmissionOutcome::Started {
            prompt: PromptQueueItem::new(
                "resend-worker-prompt",
                "resend-worker-attachment",
                CANCELLATION_GUARD_LEASED_AGENT_ID,
                prompt,
                PromptStatus::Running,
            ),
        },
    };
    let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &worker_config.relay_private_key,
        &config.relay_public_key,
        &serde_json::to_vec(&response).unwrap(),
    )
    .unwrap();
    crate::transport::relay_client::resolve_pending_peer_response_for_test(
        &relay_state,
        request_id,
        CANCELLATION_GUARD_WORKER_ID.into(),
        encrypted,
    )
    .await;
    assert!(runtime
        .owned
        .durable_state_store
        .trigger_resend_admitted(&session_id, "original-firing")
        .unwrap());
    assert!(runtime
        .resend_trigger_delivery(request, owner.owner_user_id())
        .await
        .is_err());
}
