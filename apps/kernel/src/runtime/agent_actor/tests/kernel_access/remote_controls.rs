use super::*;
use crate::agent::RemoteAgentBinding;
use crate::transport::{
    relay_crypto,
    relay_peer::{RelayPeerRequest, RelayPeerResponse},
};
use chariox_relay::protocol::RelayEnvelope;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[tokio::test]
async fn kernel_access_committed_remote_cancellation_survives_revocation_at_app_wait() {
    assert_revoked_remote_control(RemoteControl::Cancel).await;
}

#[tokio::test]
async fn kernel_access_revocation_refuses_remote_steering_waiting_for_app_lock() {
    assert_revoked_remote_control(RemoteControl::Steer).await;
}

#[tokio::test]
async fn kernel_access_revocation_refuses_remote_completion_waiting_for_app_lock() {
    assert_revoked_remote_control(RemoteControl::Complete).await;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RemoteControl {
    Cancel,
    Steer,
    Complete,
}

async fn assert_revoked_remote_control(operation: RemoteControl) {
    let worktree = TestWorktree::new("access-remote-control");
    let worker = WorkerSpy::new();
    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("remote-control-fixture".into());
    let mut daemon = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut daemon)
        .attach(AttachRequest::new(
            session.id(),
            "remote-holder",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    daemon
        .agents_mut()
        .bind_remote_execution(
            agent.id(),
            RemoteAgentBinding {
                worker_kernel_id: "worker".into(),
                worker_machine_id: "fixture-machine".into(),
                execution_lease_id: "fixture-lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: Some("worker-run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    // Seed owned home prompt state without dispatching a worker submission.
    for text in if operation == RemoteControl::Complete {
        vec!["active"]
    } else {
        vec!["active", "steering"]
    } {
        let prompt = PromptQueueItem::new(
            daemon.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            text,
            PromptStatus::Queued,
        );
        daemon
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .unwrap();
    }
    if operation == RemoteControl::Cancel {
        let prompt = daemon
            .prompt_state_owner()
            .active_prompt_for_agent(&session, agent.id())
            .unwrap();
        daemon
            .mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some("worker-run".into()),
                None,
            )
            .unwrap();
    }
    let projection = daemon.session_state_projection_store();
    let agent_projection = daemon.agent_runtime_projection_store();
    let prompts = daemon.prompt_state_owner();
    let app = Arc::new(Mutex::new(daemon));
    let mut state = owned_runtime_state(&app).await;
    let probe = Arc::new(tokio::sync::Notify::new());
    state.observe_app_lock_wait_for_test(probe.clone());
    let before = state.session_snapshot(session.id()).await.unwrap();
    let active_id = before
        .active_prompt_for_agent(agent.id())
        .unwrap()
        .id()
        .to_owned();
    let queued_id = before
        .queued_prompts_for_agent(agent.id())
        .and_then(|prompts| prompts.front())
        .map(|prompt| prompt.id().to_owned());
    let grant = state.insert_access_grant_for_test(session.id());
    let runtime = AgentRuntime::new(
        state.clone(),
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        projection,
        agent_projection,
        prompts,
        Default::default(),
    );
    let local_request = if operation == RemoteControl::Complete {
        LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().into(),
        })
    } else if operation == RemoteControl::Steer {
        LocalDaemonRequest::SteerQueuedPrompt(SteerQueuedPromptRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            target_agent_id: agent.id().into(),
            prompt_id: queued_id.clone().unwrap(),
        })
    } else {
        LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            target_agent_id: Some(agent.id().into()),
        })
    };
    let mut command =
        KernelCommand::from_local_request("external-remote-control", None, None, &local_request);
    command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant.clone();
    let locked_app = app.lock().await;
    let pending = tokio::spawn({
        let runtime = runtime.clone();
        let request = local_request.clone();
        async move { dispatch_control(&runtime, &command, request).await }
    });
    timeout(Duration::from_secs(3), probe.notified())
        .await
        .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(locked_app);
    let result = timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        worker.request_count(),
        usize::from(operation == RemoteControl::Cancel),
        "only an already committed cancellation may reach the worker after revocation"
    );
    let after = state.session_snapshot(session.id()).await.unwrap();
    assert_eq!(
        after.active_prompt_for_agent(agent.id()).unwrap().id(),
        active_id
    );
    assert_eq!(
        after.active_prompt_for_agent(agent.id()).unwrap().status(),
        if operation == RemoteControl::Cancel {
            PromptStatus::Cancelling
        } else {
            before.active_prompt_for_agent(agent.id()).unwrap().status()
        }
    );
    if let Some(queued_id) = queued_id {
        assert!(after
            .queued_prompts_for_agent(agent.id())
            .unwrap()
            .iter()
            .any(|prompt| prompt.id() == queued_id));
    }
    if operation == RemoteControl::Cancel {
        result.unwrap();
        return;
    }
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("grant revoked or expired"),
        "{error}"
    );
    let terminal =
        KernelCommand::from_local_request("terminal-remote-control", None, None, &local_request);
    dispatch_control(&runtime, &terminal, local_request)
        .await
        .unwrap();
    assert_eq!(
        worker.request_count(),
        1,
        "terminal control must reach the worker"
    );
}

async fn dispatch_control(
    runtime: &AgentRuntime,
    command: &KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::CompletePrompt(request) => {
            runtime.dispatch_prompt_complete(command, request).await
        }
        LocalDaemonRequest::CancelActivePrompt(request) => {
            runtime.dispatch_prompt_cancel(command, request).await
        }
        LocalDaemonRequest::SteerQueuedPrompt(request) => {
            runtime.dispatch_prompt_steer_queued(command, request).await
        }
        _ => unreachable!(),
    }
}

// Exercise the real discovery and encrypted peer-request wire surfaces. The
// worker records decoded control requests and acknowledges them without a provider.
struct WorkerSpy {
    url: String,
    requests: Arc<std::sync::Mutex<usize>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WorkerSpy {
    fn new() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let requests = Arc::new(std::sync::Mutex::new(0));
        let counter = requests.clone();
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async move {
                let worker = DaemonConfig::for_tests();
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                tokio::pin!(stopped);
                loop {
                    let stream = tokio::select! { _ = &mut stopped => break, accepted = listener.accept() => accepted.unwrap().0 };
                    let mut socket = accept_async(stream).await.unwrap();
                    let first = receive(&mut socket).await;
                    match first {
                        RelayEnvelope::ClientMetadataRequest { request_id, .. } => {
                            let presence = serde_json::from_value(serde_json::json!({"kernel_id":"worker", "machine_id":"fixture-machine", "public_key":worker.relay_public_key})).unwrap();
                            send(&mut socket, RelayEnvelope::ClientMetadataResponse { request_id, machines: None, kernels: None, kernel: Some(presence), error: None }).await;
                        }
                        RelayEnvelope::DaemonRegister { registration } => {
                            let RelayEnvelope::DaemonPeerRequest { request_id, encrypted_request, .. } = receive(&mut socket).await else { panic!("expected peer request") };
                            let decoded = relay_crypto::decrypt_payload_for_private_key(&worker.relay_private_key, &encrypted_request).unwrap();
                            let request: RelayPeerRequest = serde_json::from_slice(&decoded.plaintext).unwrap();
                            *counter.lock().unwrap() += 1;
                            let response = match request {
                                RelayPeerRequest::CompleteLeasedPrompt { .. } => RelayPeerResponse::LeasedPromptCompleted {
                                    provider_run_id: Some("worker-run".into()), provider_diagnostic: None, provider_termination: None,
                                    git_observations: vec![], workspace_live_sync_change: None,
                                    completion: crate::session::PromptCompletion { completed: PromptQueueItem::new("worker-active", "worker-attachment", "leased-agent", "active", PromptStatus::Completed), started_next: None },
                                },
                                RelayPeerRequest::CancelLeasedPrompt { .. } => RelayPeerResponse::LeasedPromptCancelled { cancellation: crate::session::PromptCancellation { prompt: PromptQueueItem::new("worker-active", "worker-attachment", "leased-agent", "active", PromptStatus::Cancelling), started_next: None } },
                                RelayPeerRequest::SteerLeasedPrompt { steer_id, .. } => RelayPeerResponse::LeasedPromptSteered { provider_run_id: "worker-run".into(), steer_id, replayed: false },
                                _ => panic!("unexpected worker request"),
                            };
                            send(&mut socket, RelayEnvelope::DaemonPeerResponse { request_id, from_daemon_id: "worker".into(), encrypted_response: Some(relay_crypto::encrypt_payload_for_peer(&worker.relay_private_key, &registration.public_key, &serde_json::to_vec(&response).unwrap()).unwrap()), error: None }).await;
                        }
                        _ => panic!("unexpected relay request"),
                    }
                    let _ = socket.close(None).await;
                }
            });
        });
        Self {
            url,
            requests,
            shutdown: Some(shutdown),
            thread: Some(thread),
        }
    }

    fn request_count(&self) -> usize {
        *self.requests.lock().unwrap()
    }
}

impl Drop for WorkerSpy {
    fn drop(&mut self) {
        let _ = self.shutdown.take().unwrap().send(());
        let _ = self.thread.take().unwrap().join();
    }
}

async fn receive(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) -> RelayEnvelope {
    let message = timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

async fn send(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    envelope: RelayEnvelope,
) {
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope).unwrap().into(),
        ))
        .await
        .unwrap();
}
