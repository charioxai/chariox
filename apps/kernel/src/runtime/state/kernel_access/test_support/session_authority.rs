use super::*;
use crate::runtime::command::KernelCommand;
use crate::runtime::session_actor::{FocusedAgentProjection, SessionRuntime};
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
use tokio::sync::{Mutex, Notify};
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

fn runtime(state: &KernelRuntimeState, app: &crate::DaemonApp) -> SessionRuntime {
    SessionRuntime::with_queue_limit_and_focus_projection(
        state.clone(),
        4,
        FocusedAgentProjection::default(),
        state.owned.session_projection.clone(),
        state.owned.agent_runtime_projection.clone(),
        app.terminal_stream_store(),
    )
}

fn external_command(request: &LocalDaemonRequest, grant: &str) -> KernelCommand {
    let mut command =
        KernelCommand::from_local_request("external-session-operation", None, None, request);
    command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant.into();
    command
}

#[tokio::test]
async fn kernel_access_destroy_refuses_another_same_owner_session_agent() {
    let worktree = crate::test_support::TestWorktree::new("access-session-target");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (allowed, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let (other, victim) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let state = router.runtime_state();
    let runtime = runtime(&state, &*app.lock().await);
    let grant = state.insert_access_grant_for_test(allowed.id());
    let before = state.session_snapshot(other.id()).await.unwrap();
    let request = LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
        session_id: allowed.id().into(),
        agent_id: victim.id().into(),
    });
    let result = runtime
        .dispatch_session_command(external_command(&request, &grant), request)
        .await;
    assert!(result.is_err(), "a grant for A destroyed B's agent");
    assert_eq!(state.session_snapshot(other.id()).await.unwrap(), before);
    assert_eq!(
        state.owned.agent_store.get_agent(victim.id()).unwrap(),
        victim
    );
}

#[tokio::test]
async fn kernel_access_remote_destroy_rechecks_after_app_lock_wait() {
    revoked_remote_operation(false, false).await;
}

#[tokio::test]
async fn kernel_access_remote_spawn_rechecks_after_app_lock_wait() {
    revoked_remote_operation(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_remote_destroy_rechecks_after_discovery_wait() {
    revoked_remote_operation(false, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_remote_spawn_rechecks_after_discovery_wait() {
    revoked_remote_operation(true, true).await;
}

async fn revoked_remote_operation(spawn: bool, discovery_wait: bool) {
    let worktree = crate::test_support::TestWorktree::new("access-session-operation");
    let worker = WorkerSpy::new(discovery_wait);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("session-operation-fixture".into());
    let mut daemon = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    if !spawn {
        daemon
            .agents_mut()
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: worker.id.clone(),
                    worker_machine_id: "fixture-machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "leased-agent".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
    }
    let app = Arc::new(Mutex::new(daemon));
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let mut state = router.runtime_state();
    let probe = Arc::new(Notify::new());
    state.observe_app_lock_wait_for_test(probe.clone());
    let runtime = runtime(&state, &*app.lock().await);
    let before = state.session_snapshot(session.id()).await.unwrap();
    let grant = state.insert_access_grant_for_test(session.id());
    let request = if spawn {
        LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
            session_id: session.id().into(),
            alias: Some("remote-created".into()),
            provider: Some("dev-stub".into()),
            account_profile: None,
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: Some(worktree.path().display().to_string()),
            kernel_ref: Some(worker.id.clone()),
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        })
    } else {
        LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
        })
    };
    let guard = if discovery_wait {
        None
    } else {
        Some(app.lock().await)
    };
    let pending = tokio::spawn({
        let runtime = runtime.clone();
        let request = request.clone();
        let command = external_command(&request, &grant);
        async move { runtime.dispatch_session_command(command, request).await }
    });
    let reached = if discovery_wait {
        &worker.discovery_started
    } else {
        &probe
    };
    timeout(Duration::from_secs(3), reached.notified())
        .await
        .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(guard);
    worker.release_discovery.notify_one();
    let result = timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        0,
        "revoked session operation reached worker"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    assert_eq!(state.session_snapshot(session.id()).await.unwrap(), before);
    let terminal =
        KernelCommand::from_local_request("terminal-session-control", None, None, &request);
    let response = runtime
        .dispatch_session_command(terminal, request)
        .await
        .unwrap();
    assert!(if spawn {
        matches!(response, LocalDaemonResponse::AgentSpawned { .. })
    } else {
        matches!(response, LocalDaemonResponse::AgentDestroyed { .. })
    });
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        2,
        "terminal operation should reach worker"
    );
}

struct WorkerSpy {
    id: String,
    url: String,
    requests: Arc<AtomicUsize>,
    discovery_started: Arc<Notify>,
    release_discovery: Arc<Notify>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WorkerSpy {
    fn new(pause_discovery: bool) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let id = format!("session-authority-worker-{:016x}", rand::random::<u64>());
        let worker_id = id.clone();
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
                            let presence = serde_json::from_value(serde_json::json!({
                                "kernel_id":worker_id, "machine_id":"fixture-machine", "public_key":worker.relay_public_key,
                                "accepting_remote_leases":true, "available_providers":["dev-stub"]
                            })).unwrap();
                            send(&mut socket, RelayEnvelope::ClientMetadataResponse { request_id, machines: None, kernels: None, kernel: Some(presence), error: None }).await;
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
            shutdown: Some(shutdown),
            thread: Some(thread),
        }
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
