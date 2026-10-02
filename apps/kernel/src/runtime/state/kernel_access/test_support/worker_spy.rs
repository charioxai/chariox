use super::*;
use crate::transport::{
    relay_crypto,
    relay_peer::{RelayPeerRequest, RelayPeerResponse},
};
use chariox_relay::protocol::RelayEnvelope;
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::Notify;
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

pub(in crate::runtime::state) struct WorkerSpy {
    pub(in crate::runtime::state) id: String,
    pub(in crate::runtime::state) url: String,
    pub(in crate::runtime::state) requests: Arc<AtomicUsize>,
    pub(in crate::runtime::state) discovery_started: Arc<Notify>,
    pub(in crate::runtime::state) release_discovery: Arc<Notify>,
    pub(in crate::runtime::state) setup_status:
        Arc<std::sync::Mutex<Option<crate::local::ProjectEnvironmentSetupStatus>>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WorkerSpy {
    pub(in crate::runtime::state) fn new(pause_discovery: bool) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let id = format!("session-authority-worker-{:016x}", rand::random::<u64>());
        let worker_id = id.clone();
        let setup_status = Arc::new(std::sync::Mutex::new(
            None::<crate::local::ProjectEnvironmentSetupStatus>,
        ));
        let worker_setup_status = setup_status.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let discovery_started = Arc::new(Notify::new());
        let release_discovery = Arc::new(Notify::new());
        let started = discovery_started.clone();
        let released = release_discovery.clone();
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async move {
                let worker = crate::config::DaemonConfig::for_tests();
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let mut home_agent = String::new();
                let mut first_discovery = true;
                tokio::pin!(stopped);
                loop {
                    let stream = tokio::select! { _ = &mut stopped => break, accepted = listener.accept() => accepted.unwrap().0 };
                    let mut socket = accept_async(stream).await.unwrap();
                    match receive(&mut socket).await {
                        RelayEnvelope::ClientMetadataRequest { request_id, .. } => {
                            if pause_discovery && first_discovery {
                                first_discovery = false;
                                started.notify_one();
                                released.notified().await;
                            }
                            let presence: chariox_relay::protocol::RelayKernelPresence = serde_json::from_value(serde_json::json!({
                                "kernel_id":worker_id, "machine_id":"fixture-machine", "public_key":worker.relay_public_key,
                                "accepting_remote_leases":true, "available_providers":["dev-stub"]
                            })).unwrap();
                            send(&mut socket, RelayEnvelope::ClientMetadataResponse { request_id, machines: None, kernels: Some(vec![presence.clone()]), kernel: Some(presence), error: None }).await;
                        }
                        RelayEnvelope::DaemonRegister { registration } => {
                            let RelayEnvelope::DaemonPeerRequest { request_id, encrypted_request, .. } = receive(&mut socket).await else { panic!("expected peer request") };
                            let decoded = relay_crypto::decrypt_payload_for_private_key(&worker.relay_private_key, &encrypted_request).unwrap();
                            let request: RelayPeerRequest = serde_json::from_slice(&decoded.plaintext).unwrap();
                            counter.fetch_add(1, Ordering::SeqCst);
                            let response = match request {
                                RelayPeerRequest::DestroyLeasedAgent { leased_agent_id } => RelayPeerResponse::LeasedAgentDestroyed { leased_agent_id },
                                RelayPeerRequest::DestroyExecutionLease { lease_id } => RelayPeerResponse::ExecutionLeaseDestroyed { lease_id },
                                RelayPeerRequest::CreateExecutionLease { home_kernel_id, home_session_id, home_agent_id, home_agent_metaagent, owner_user_id } => {
                                    home_agent = home_agent_id.clone();
                                    RelayPeerResponse::ExecutionLeaseCreated { lease: crate::execution_lease::ExecutionLease::new(
                                        "lease".into(), home_kernel_id, home_session_id, home_agent_id, home_agent_metaagent, owner_user_id,
                                        worker_id.clone(), "fixture-machine".into(),
                                    ), relay_peer_protocol_version: crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION }
                                }
                                RelayPeerRequest::SpawnLeasedAgent { lease_id, provider, account_profile, model, effort, execution_mode, permission_level, .. } => {
                                    RelayPeerResponse::LeasedAgentSpawned { leased_agent: serde_json::from_value(serde_json::json!({
                                        "id":"leased-agent", "lease_id":lease_id, "home_agent_id":home_agent, "provider":provider,
                                        "account_profile":account_profile, "model":model, "effort":effort, "execution_mode":execution_mode,
                                        "permission_level":permission_level, "backing_session_id":"worker-session", "backing_agent_id":"worker-agent",
                                        "backing_attachment_id":"worker-attachment", "created_at_ms":1
                                    })).unwrap() }
                                }
                                RelayPeerRequest::UpdateLeasedAgentConfig { leased_agent_id, execution_mode, permission_level } => RelayPeerResponse::LeasedAgentConfigUpdated { leased_agent: serde_json::from_value(serde_json::json!({
                                    "id":leased_agent_id, "lease_id":"lease", "home_agent_id":home_agent, "provider":"dev-stub", "account_profile":"default",
                                    "model":null, "effort":null, "execution_mode":execution_mode, "permission_level":permission_level,
                                    "backing_session_id":"worker-session", "backing_agent_id":"worker-agent", "backing_attachment_id":"worker-attachment", "created_at_ms":1
                                })).unwrap() },
                                RelayPeerRequest::CancelLeasedPrompt { .. } => RelayPeerResponse::LeasedPromptCancelled { cancellation: crate::session::PromptCancellation {
                                    prompt: crate::session::PromptQueueItem::new("worker-prompt", "worker-attachment", "worker-agent", "fixture", crate::session::PromptStatus::Cancelled),
                                    started_next: None,
                                } },
                                RelayPeerRequest::SendLeasedNativeProviderInput { data_base64, .. } => {
                                    use base64::Engine;
                                    RelayPeerResponse::LeasedNativeProviderInputSent { byte_count: base64::engine::general_purpose::STANDARD.decode(data_base64).unwrap().len() }
                                }
                                RelayPeerRequest::CancelLeasedProjectEnvironmentSetup { .. } => {
                                    let mut status = worker_setup_status.lock().unwrap().clone().unwrap();
                                    status.phase = crate::local::ProjectEnvironmentSetupPhase::Cancelled;
                                    status.retryable = true;
                                    RelayPeerResponse::LeasedProjectEnvironmentSetupCancelled { setup: crate::transport::relay_peer::RelayProjectEnvironmentSetupStatus { status, definition: None } }
                                }
                                RelayPeerRequest::RoomBrowserController { session_id, slice_id, command: crate::transport::room_browser_controller::RoomBrowserControllerCommand::Release } => RelayPeerResponse::RoomBrowserController { session_id, slice_id, result: crate::transport::room_browser_controller::RoomBrowserControllerResult::Process { snapshot: None } },
                                _ => panic!("unexpected worker request"),
                            };
                            send(&mut socket, RelayEnvelope::DaemonPeerResponse { request_id, from_daemon_id: worker_id.clone(), encrypted_response: Some(relay_crypto::encrypt_payload_for_peer(&worker.relay_private_key, &registration.public_key, &serde_json::to_vec(&response).unwrap()).unwrap()), error: None }).await;
                        }
                        _ => panic!("unexpected relay request"),
                    }
                    let _ = socket.close(None).await;
                }
            });
        });
        Self {
            id,
            url,
            requests,
            discovery_started,
            release_discovery,
            setup_status,
            shutdown: Some(shutdown),
            thread: Some(thread),
        }
    }
}

impl Drop for WorkerSpy {
    fn drop(&mut self) {
        self.release_discovery.notify_one();
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
