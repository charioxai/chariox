use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{watch, Mutex, RwLock};
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use arroba_relay::protocol::{EncryptedRelayPayload, RelayEnvelope, RelayError};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::transport::relay_crypto;

#[derive(Debug, Clone, Default)]
pub struct RelayClientState {
    connected: bool,
}

pub async fn run_daemon_relay_connector(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let (relay_url, heartbeat) = {
        let app = app.lock().await;
        let config = app.config();
        let Some(relay_url) = config.relay_url.clone() else {
            return;
        };
        if config.relay_token.is_none() {
            return;
        }
        (relay_url, Duration::from_millis(config.relay_heartbeat_ms))
    };

    loop {
        if *shutdown.borrow() {
            set_connected(&state, false).await;
            return;
        }

        match connect_async(&relay_url).await {
            Ok((mut socket, _)) => {
                let register = {
                    let app = app.lock().await;
                    RelayEnvelope::DaemonRegister {
                        registration: app.relay_registration(),
                    }
                };
                if send_envelope(&mut socket, &register).await.is_err() {
                    set_connected(&state, false).await;
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                set_connected(&state, true).await;

                loop {
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_ok() && *shutdown.borrow() {
                                let _ = send_envelope(&mut socket, &RelayEnvelope::Close {
                                    reason: "daemon shutting down".to_string(),
                                }).await;
                                let _ = socket.close(None).await;
                                set_connected(&state, false).await;
                                return;
                            }
                        }
                        incoming = socket.next() => {
                            match incoming {
                                Some(Ok(Message::Text(payload))) => {
                                    if handle_incoming_envelope(&app, &mut socket, &payload).await.is_err() {
                                        set_connected(&state, false).await;
                                        break;
                                    }
                                }
                                Some(Ok(Message::Close(_))) => {
                                    set_connected(&state, false).await;
                                    break;
                                }
                                Some(Ok(_)) => {}
                                Some(Err(_)) | None => {
                                    set_connected(&state, false).await;
                                    break;
                                }
                            }
                        }
                        _ = sleep(heartbeat) => {
                            let heartbeat_frame = RelayEnvelope::DaemonHeartbeat {
                                daemon_id: register_daemon_id(&register).to_string(),
                            };
                            if send_envelope(&mut socket, &heartbeat_frame).await.is_err() {
                                set_connected(&state, false).await;
                                break;
                            }
                        }
                    }
                }
            }
            Err(_) => {
                set_connected(&state, false).await;
                let reconnect_delay = sleep(Duration::from_secs(1));
                tokio::pin!(reconnect_delay);
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_ok() && *shutdown.borrow() {
                            return;
                        }
                    }
                    _ = &mut reconnect_delay => {}
                }
            }
        }
    }
}

async fn handle_incoming_envelope<S>(
    app: &Arc<Mutex<DaemonApp>>,
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    payload: &str,
) -> Result<(), DaemonError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let envelope = serde_json::from_str::<RelayEnvelope>(payload).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "parse relay envelope",
            message: error.to_string(),
        }
    })?;
    match envelope {
        RelayEnvelope::DaemonRequest {
            relay_request_id,
            encrypted_request,
        } => {
            let relay_response = handle_daemon_request(app, encrypted_request).await;
            send_envelope(
                socket,
                &RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: relay_response.encrypted_response,
                    error: relay_response.error,
                },
            )
            .await?;
        }
        RelayEnvelope::Close { reason } => {
            return Err(DaemonError::LocalTransport {
                operation: "relay closed connection",
                message: reason,
            });
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct RelayRequestOutcome {
    encrypted_response: Option<EncryptedRelayPayload>,
    error: Option<RelayError>,
}

async fn handle_daemon_request(
    app: &Arc<Mutex<DaemonApp>>,
    encrypted_request: EncryptedRelayPayload,
) -> RelayRequestOutcome {
    let (request, client_public_key, daemon_private_key) = {
        let app = app.lock().await;
        let daemon_private_key = app.config().relay_private_key.clone();
        let decrypted = match relay_crypto::decrypt_payload_for_private_key(
            &daemon_private_key,
            &encrypted_request,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        let request = match serde_json::from_slice::<LocalDaemonRequest>(&decrypted.plaintext) {
            Ok(request) => request,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        (request, decrypted.sender_public_key, daemon_private_key)
    };

    if !is_supported_relay_request(&request) {
        return RelayRequestOutcome {
            encrypted_response: None,
            error: Some(relay_error(
                "unsupported_request",
                "relay transport does not yet support this request type",
                false,
            )),
        };
    }

    let result = {
        let mut app = app.lock().await;
        app.handle_local_request(request)
    };
    match result {
        Ok(response) => {
            let plaintext = match serde_json::to_vec(&response) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(relay_error(
                            "relay_request_failed",
                            &format!("failed to serialize relay response: {error}"),
                            false,
                        )),
                    };
                }
            };
            match relay_crypto::encrypt_payload_for_peer(
                &daemon_private_key,
                &client_public_key,
                &plaintext,
            ) {
                Ok(encrypted_response) => RelayRequestOutcome {
                    encrypted_response: Some(encrypted_response),
                    error: None,
                },
                Err(error) => RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "relay_request_failed",
                        &format!("failed to encrypt relay response: {error}"),
                        false,
                    )),
                },
            }
        }
        Err(error) => RelayRequestOutcome {
            encrypted_response: None,
            error: Some(map_relay_error(&error)),
        },
    }
}

fn register_daemon_id(register: &RelayEnvelope) -> &str {
    match register {
        RelayEnvelope::DaemonRegister { registration } => registration.daemon_id.as_str(),
        _ => unreachable!("relay register envelope expected"),
    }
}

fn is_supported_relay_request(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::ListSessions(_)
            | LocalDaemonRequest::GetSessionState(_)
            | LocalDaemonRequest::AttachToSession(_)
    )
}

fn map_relay_error(error: &DaemonError) -> RelayError {
    match error {
        DaemonError::SessionNotFound { .. } => {
            relay_error("session_not_found", &error.to_string(), false)
        }
        DaemonError::AttachmentNotFound { .. } => {
            relay_error("attachment_not_found", &error.to_string(), false)
        }
        DaemonError::AttachmentNotInSession { .. } => {
            relay_error("attachment_not_in_session", &error.to_string(), false)
        }
        DaemonError::LocalTransport { .. } => {
            relay_error("transport_error", &error.to_string(), true)
        }
        _ => relay_error("relay_request_failed", &error.to_string(), false),
    }
}

fn relay_error(code: &str, message: &str, retryable: bool) -> RelayError {
    RelayError {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    }
}

async fn send_envelope<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    envelope: &RelayEnvelope,
) -> Result<(), DaemonError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let payload = serde_json::to_string(envelope).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay envelope",
        message: error.to_string(),
    })?;
    socket
        .send(Message::Text(payload.into()))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "send relay envelope",
            message: error.to_string(),
        })
}

async fn set_connected(state: &Arc<RwLock<RelayClientState>>, connected: bool) {
    state.write().await.connected = connected;
}

#[cfg(test)]
mod tests {
    use super::*;

    use arroba_relay::{protocol::ClientTarget, RelayConfig, RelayServer};
    use tokio::sync::oneshot;
    use tokio::time::{sleep, Duration};

    use crate::attachment::ClientCapabilityLevel;
    use crate::config::DaemonConfig;
    use crate::local::{
        AttachToSessionRequest, GetSessionStateRequest, ListSessionsRequest, LocalDaemonResponse,
    };
    use crate::session::CreateSessionRequest;
    use crate::transport::relay_crypto;

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_connector_registers_with_relay() {
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector_task = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app),
            Arc::clone(&state),
            shutdown_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

        {
            let guard = registry.read().await;
            assert!(guard.daemon(&config.daemon_id).is_some());
        }
        assert!(state.read().await.connected);

        let _ = shutdown_tx.send(true);
        connector_task.await.expect("connector task should join");
        sleep(Duration::from_millis(50)).await;
        {
            let guard = registry.read().await;
            assert!(guard.daemon(&config.daemon_id).is_none());
        }
        assert!(!state.read().await.connected);

        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn proxied_session_requests_are_handled_through_relay() {
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let created_session_id = {
            let mut app = app.lock().await;
            let response = app
                .handle_local_request(LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new("workspace-relay-test", "worktree-relay-test"),
                ))
                .expect("session should be created");
            match response {
                LocalDaemonResponse::SessionCreated { session, .. } => session.id().to_string(),
                other => panic!("unexpected response: {other:?}"),
            }
        };
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector_task = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app),
            Arc::clone(&state),
            shutdown_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut client_socket, _) = connect_async(&url)
            .await
            .expect("client should connect to relay");
        send_client_envelope(
            &mut client_socket,
            &RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some(config.daemon_id.clone()),
                    daemon_alias: None,
                },
            },
        )
        .await;
        let daemon_public_key = expect_client_connected(&mut client_socket).await;

        let list_request_private_key = send_client_request(
            &mut client_socket,
            "list-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::ListSessions(ListSessionsRequest),
        )
        .await;
        let list_response =
            expect_client_response(&mut client_socket, "list-1", &list_request_private_key).await;
        assert!(matches!(
            list_response,
            LocalDaemonResponse::SessionsListed { sessions } if sessions.iter().any(|session| session.id() == created_session_id)
        ));

        let state_request_private_key = send_client_request(
            &mut client_socket,
            "state-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                session_id: created_session_id.clone(),
            }),
        )
        .await;
        let state_response =
            expect_client_response(&mut client_socket, "state-1", &state_request_private_key).await;
        assert!(matches!(
            state_response,
            LocalDaemonResponse::SessionState { session } if session.id() == created_session_id
        ));

        let attach_request_private_key = send_client_request(
            &mut client_socket,
            "attach-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
                session_id: created_session_id.clone(),
                client_id: "relay-client".to_string(),
                capability_level: ClientCapabilityLevel::MessageTransport,
            }),
        )
        .await;
        let attach_response =
            expect_client_response(&mut client_socket, "attach-1", &attach_request_private_key)
                .await;
        assert!(matches!(
            attach_response,
            LocalDaemonResponse::SessionAttached { attachment } if attachment.session_id() == created_session_id
        ));

        let _ = shutdown_tx.send(true);
        connector_task.await.expect("connector task should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    async fn wait_for_daemon_registration(
        registry: Arc<RwLock<arroba_relay::server::RelayRegistry>>,
        daemon_id: &str,
    ) {
        for _ in 0..40 {
            if registry.read().await.daemon(daemon_id).is_some() {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
        panic!("daemon `{daemon_id}` did not register with relay");
    }

    async fn send_client_envelope<S>(
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

    async fn send_client_request<S>(
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
        let encrypted_request = relay_crypto::encrypt_payload_for_peer(
            &client_private_key,
            daemon_public_key,
            &plaintext,
        )
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

    async fn expect_client_connected<S>(
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

    async fn expect_client_response<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        request_id: &str,
        client_private_key: &str,
    ) -> LocalDaemonResponse
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
                        assert!(error.is_none(), "unexpected relay error: {error:?}");
                        let encrypted_response =
                            encrypted_response.expect("response payload should exist");
                        let decrypted = relay_crypto::decrypt_payload_for_private_key(
                            client_private_key,
                            &encrypted_response,
                        )
                        .expect("response should decrypt");
                        serde_json::from_slice(&decrypted.plaintext)
                            .expect("local response should deserialize")
                    }
                    other => panic!("unexpected envelope: {other:?}"),
                }
            }
            other => panic!("unexpected relay message: {other:?}"),
        }
    }
}
