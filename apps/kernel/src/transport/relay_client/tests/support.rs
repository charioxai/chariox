#![allow(unused_imports)]
pub(super) use super::super::*;

pub(super) use arroba_relay::{protocol::ClientTarget, RelayConfig, RelayServer};
pub(super) use tokio::sync::oneshot;
pub(super) use tokio::time::{sleep, Duration};

pub(super) use crate::agent::CreateAgentRequest;
pub(super) use crate::app::RemoteLeaseRuntime;
pub(super) use crate::attachment::{AttachRequest, ClientCapabilityLevel};
pub(super) use crate::config::DaemonConfig;
pub(super) use crate::local::{
    AttachToSessionRequest, DetachFromSessionRequest, FocusAgentRequest, GetSessionStateRequest,
    ListSessionsRequest, LocalDaemonRequest, LocalDaemonResponse, ResizeTerminalRequest,
    ResolveSessionRequest, RespondToInteractionRequest, UpdateSessionConfigRequest,
    ValidateWorkflowOutputRequest,
};
pub(super) use crate::runtime::command::KernelCommand;
pub(super) use crate::session::CreateSessionRequest;
pub(super) use crate::transport::relay_crypto;
pub(super) use crate::transport::relay_discovery;
pub(super) use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
pub(super) use std::collections::BTreeMap;
pub(super) use std::sync::OnceLock;

pub(super) async fn relay_client_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

pub(super) fn create_test_session(app: &mut DaemonApp, workspace: &str, worktree: &str) -> String {
    crate::app::KernelSessionService::new(app)
        .create_session(CreateSessionRequest::new(workspace, worktree))
        .expect("session should be created")
        .0
        .id()
        .to_string()
}

pub(super) fn create_test_session_with_alias(
    app: &mut DaemonApp,
    workspace: &str,
    worktree: &str,
    alias: &str,
) -> (String, String) {
    let (session, agent) = crate::app::KernelSessionService::new(app)
        .create_session(CreateSessionRequest::new(workspace, worktree).with_alias(alias))
        .expect("session should be created");
    (session.id().to_string(), agent.id().to_string())
}

pub(super) fn attach_test_client(
    app: &mut DaemonApp,
    session_id: &str,
    client_id: &str,
    capability_level: ClientCapabilityLevel,
) -> String {
    crate::app::KernelSessionService::new(app)
        .attach(AttachRequest::new(session_id, client_id, capability_level))
        .expect("session should attach")
        .id()
        .to_string()
}

pub(super) async fn refresh_remote_inventory_projection_for_app_with_relay_state(
    app: &Arc<Mutex<DaemonApp>>,
) -> Result<(), DaemonError> {
    let (config_projection, remote_inventory_projection) = {
        let app = app.lock().await;
        (
            app.config_projection_store(),
            app.remote_relay_inventory_projection_store(),
        )
    };
    refresh_remote_inventory_projection(config_projection, remote_inventory_projection).await
}

pub(super) async fn wait_for_daemon_registration(
    registry: Arc<RwLock<arroba_relay::server::RelayRegistry>>,
    daemon_id: &str,
) {
    for _ in 0..200 {
        if registry.read().await.daemon(daemon_id).is_some() {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("daemon `{daemon_id}` did not register with relay");
}

pub(super) async fn wait_for_active_interaction(
    app: Arc<Mutex<DaemonApp>>,
    session_id: &str,
    agent_id: &str,
) -> String {
    for _ in 0..80 {
        {
            let app = app.lock().await;
            if let Ok(session) = app.sessions().get_session(session_id) {
                if let Some(interaction) = session.active_interaction_for_agent(agent_id) {
                    return interaction.id().to_string();
                }
            }
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("interaction for agent `{agent_id}` did not become active");
}

pub(super) async fn send_client_envelope<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    envelope: &RelayEnvelope,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket
        .send(Message::Text(
            serde_json::to_string(envelope)
                .expect("relay envelope should serialize")
                .into(),
        ))
        .await
        .expect("client envelope should send");
}

pub(super) async fn send_client_request<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    request_id: &str,
    daemon_id: &str,
    daemon_public_key: &str,
    request: LocalDaemonRequest,
) -> String
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let client_private_key = relay_crypto::generate_private_key_base64();
    let plaintext = serde_json::to_vec(&request).expect("request should serialize");
    let encrypted_request =
        relay_crypto::encrypt_payload_for_peer(&client_private_key, daemon_public_key, &plaintext)
            .expect("request should encrypt");
    send_client_envelope(
        socket,
        &RelayEnvelope::ClientRequest {
            request_id: request_id.to_string(),
            target: ClientTarget {
                daemon_id: Some(daemon_id.to_string()),
                daemon_alias: None,
            },
            encrypted_request,
        },
    )
    .await;
    client_private_key
}

pub(super) async fn expect_client_connected<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
) -> String
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match socket.next().await {
        Some(Ok(Message::Text(payload))) => {
            match serde_json::from_str::<RelayEnvelope>(&payload)
                .expect("relay envelope should parse")
            {
                RelayEnvelope::ClientConnected {
                    daemon_public_key, ..
                } => daemon_public_key,
                other => panic!("unexpected envelope: {other:?}"),
            }
        }
        other => panic!("unexpected relay message: {other:?}"),
    }
}

pub(super) async fn expect_client_response<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    request_id: &str,
    client_private_key: &str,
) -> LocalDaemonResponse
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(payload))) => {
                match serde_json::from_str::<RelayEnvelope>(&payload)
                    .expect("relay envelope should parse")
                {
                    RelayEnvelope::ClientResponse {
                        request_id: response_request_id,
                        encrypted_response,
                        error,
                    } => {
                        assert_eq!(response_request_id, request_id);
                        assert!(error.is_none(), "unexpected relay error: {error:?}");
                        let encrypted_response =
                            encrypted_response.expect("response payload should exist");
                        let decrypted = relay_crypto::decrypt_payload_for_private_key(
                            client_private_key,
                            &encrypted_response,
                        )
                        .expect("response should decrypt");
                        return serde_json::from_slice(&decrypted.plaintext)
                            .expect("local response should deserialize");
                    }
                    RelayEnvelope::ClientEvent { .. } => {}
                    other => panic!("unexpected envelope: {other:?}"),
                }
            }
            other => panic!("unexpected relay message: {other:?}"),
        }
    }
}

pub(super) async fn expect_json_client_response<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    request_id: &str,
    client_private_key: &str,
) -> serde_json::Value
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(payload))) => {
                match serde_json::from_str::<RelayEnvelope>(&payload)
                    .expect("relay envelope should parse")
                {
                    RelayEnvelope::ClientResponse {
                        request_id: response_request_id,
                        encrypted_response,
                        error,
                    } => {
                        assert_eq!(response_request_id, request_id);
                        assert!(error.is_none(), "unexpected relay error: {error:?}");
                        let encrypted_response =
                            encrypted_response.expect("response payload should exist");
                        let decrypted = relay_crypto::decrypt_payload_for_private_key(
                            client_private_key,
                            &encrypted_response,
                        )
                        .expect("response should decrypt");
                        return serde_json::from_slice(&decrypted.plaintext)
                            .expect("json response should deserialize");
                    }
                    RelayEnvelope::ClientEvent { .. } => {}
                    other => panic!("unexpected envelope: {other:?}"),
                }
            }
            other => panic!("unexpected relay message: {other:?}"),
        }
    }
}

pub(super) async fn expect_client_event<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    client_private_key: &str,
) -> serde_json::Value
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(payload))) => {
                match serde_json::from_str::<RelayEnvelope>(&payload)
                    .expect("relay envelope should parse")
                {
                    RelayEnvelope::ClientEvent {
                        encrypted_event, ..
                    } => {
                        let decrypted = relay_crypto::decrypt_payload_for_private_key(
                            client_private_key,
                            &encrypted_event,
                        )
                        .expect("event should decrypt");
                        return serde_json::from_slice(&decrypted.plaintext)
                            .expect("event should deserialize");
                    }
                    RelayEnvelope::ClientResponse { .. } => continue,
                    other => panic!("unexpected envelope: {other:?}"),
                }
            }
            other => panic!("unexpected relay message: {other:?}"),
        }
    }
}

pub(super) async fn expect_client_event_envelope<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    client_private_key: &str,
) -> (u64, serde_json::Value)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(payload))) => {
                match serde_json::from_str::<RelayEnvelope>(&payload)
                    .expect("relay envelope should parse")
                {
                    RelayEnvelope::ClientEvent {
                        event_id,
                        encrypted_event,
                        ..
                    } => {
                        let decrypted = relay_crypto::decrypt_payload_for_private_key(
                            client_private_key,
                            &encrypted_event,
                        )
                        .expect("event should decrypt");
                        return (
                            event_id,
                            serde_json::from_slice(&decrypted.plaintext)
                                .expect("event should deserialize"),
                        );
                    }
                    RelayEnvelope::ClientResponse { .. } => continue,
                    other => panic!("unexpected envelope: {other:?}"),
                }
            }
            other => panic!("unexpected relay message: {other:?}"),
        }
    }
}

pub(super) async fn expect_named_client_event<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    client_private_key: &str,
    expected_event: &str,
) -> (u64, serde_json::Value)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let envelope = expect_client_event_envelope(socket, client_private_key).await;
        if envelope.1["event"] == serde_json::json!(expected_event) {
            return envelope;
        }
    }
}

pub(super) async fn decrypt_relay_event_from_channel(
    outgoing_rx: &mut mpsc::UnboundedReceiver<RelayEnvelope>,
    client_private_key: &str,
) -> (u64, serde_json::Value) {
    match outgoing_rx
        .recv()
        .await
        .expect("relay event should be emitted")
    {
        RelayEnvelope::DaemonEvent {
            event_id,
            encrypted_event,
            ..
        } => {
            let decrypted =
                relay_crypto::decrypt_payload_for_private_key(client_private_key, &encrypted_event)
                    .expect("event should decrypt");
            (
                event_id,
                serde_json::from_slice(&decrypted.plaintext).expect("event should deserialize"),
            )
        }
        other => panic!("unexpected relay envelope: {other:?}"),
    }
}

pub(super) async fn expect_client_error<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    request_id: &str,
    _client_private_key: &str,
) -> RelayError
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match socket.next().await {
        Some(Ok(Message::Text(payload))) => {
            match serde_json::from_str::<RelayEnvelope>(&payload)
                .expect("relay envelope should parse")
            {
                RelayEnvelope::ClientResponse {
                    request_id: response_request_id,
                    encrypted_response,
                    error,
                } => {
                    assert_eq!(response_request_id, request_id);
                    assert!(encrypted_response.is_none());
                    error.expect("relay error should exist")
                }
                other => panic!("unexpected envelope: {other:?}"),
            }
        }
        other => panic!("unexpected relay message: {other:?}"),
    }
}
