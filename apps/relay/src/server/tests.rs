mod daemon;
mod display;
mod metadata;
mod peer;
mod registry;
mod routing;
mod runtime_client;
mod subscription;

use super::*;

use crate::auth::{
    DEFAULT_RELAY_REALM_ID, RelayAction, RelayAuthError, RelayAuthRequest, RelayAuthVerifier,
    RelaySubjectKind, RelayTokenClaims, ScopedTokenVerifier,
};
use crate::protocol::{
    ClientTarget, DaemonRegistration, EncryptedRelayPayload, RelayDisplayTunnelHeader,
    RelayDisplayTunnelRegistration, RelayDisplayTunnelStreamChunk, RelayEnvelope,
    RelayMetadataQuery,
};
use crate::registry::DaemonKey;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Duration, sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};

fn test_registration(
    daemon_id: &str,
    machine_id: &str,
    os_name: &str,
    kernel_started_at_ms: u64,
) -> DaemonRegistration {
    test_registration_with_token(
        daemon_id,
        machine_id,
        os_name,
        kernel_started_at_ms,
        "secret",
    )
}

fn test_registration_with_token(
    daemon_id: &str,
    machine_id: &str,
    os_name: &str,
    kernel_started_at_ms: u64,
    auth_token: &str,
) -> DaemonRegistration {
    DaemonRegistration {
        auth_token: auth_token.to_string(),
        daemon_id: daemon_id.to_string(),
        machine_id: machine_id.to_string(),
        machine_alias: None,
        os_name: Some(os_name.to_string()),
        kernel_started_at_ms,
        daemon_alias: None,
        kernel_alias: None,
        public_key: format!("public-key-{daemon_id}"),
        capabilities: vec!["kernel_ws".to_string()],
        available_providers: vec!["codex".to_string()],
        provider_accounts: Vec::new(),
        accepting_remote_leases: true,
        leased_agent_count: 0,
        local_session_count: 0,
    }
}

fn scoped_claim(
    token_id: &str,
    subject: &str,
    subject_kind: RelaySubjectKind,
    realm_id: &str,
    actions: Vec<RelayAction>,
    targets: Option<Vec<&str>>,
) -> RelayTokenClaims {
    RelayTokenClaims {
        issuer: "test-issuer".to_string(),
        subject: subject.to_string(),
        subject_kind,
        realm_id: realm_id.to_string(),
        allowed_actions: actions,
        allowed_targets: targets.map(|values| {
            values
                .into_iter()
                .map(std::string::ToString::to_string)
                .collect()
        }),
        issued_at_ms: 1,
        expires_at_ms: 100,
        token_id: token_id.to_string(),
        account_id: None,
        organization_id: None,
        user_id: None,
        device_id: None,
        machine_id: None,
        client_id: None,
        session_id: None,
        public_key_thumbprint: None,
        entitlements_version: None,
    }
}

async fn connect_async_with_retry(
    url: &str,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    tokio_tungstenite::tungstenite::Error,
> {
    let mut last_error = None;
    for _ in 0..20 {
        match connect_async(url).await {
            Ok(socket) => return Ok(socket),
            Err(error) => {
                last_error = Some(error);
                sleep(Duration::from_millis(10)).await;
            }
        }
    }
    Err(last_error.expect("connect retry should record an error"))
}

async fn relay_http_get(addr: std::net::SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("relay HTTP connection should open");
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nhost: relay.test\r\n\r\n").as_bytes())
        .await
        .expect("relay HTTP request should write");
    let mut response = String::new();
    timeout(Duration::from_secs(2), stream.read_to_string(&mut response))
        .await
        .expect("relay HTTP response should complete")
        .expect("relay HTTP response should read");
    response
}

async fn relay_http_get_until_close_or_reset(addr: std::net::SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("relay HTTP connection should open");
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nhost: relay.test\r\n\r\n").as_bytes())
        .await
        .expect("relay HTTP request should write");
    let mut response = Vec::new();
    let mut buffer = [0_u8; 512];
    loop {
        match timeout(Duration::from_secs(2), stream.read(&mut buffer)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(size)) => response.extend_from_slice(&buffer[..size]),
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Ok(Err(error)) => panic!("relay HTTP response should read: {error}"),
            Err(_) => panic!("relay HTTP response should complete"),
        }
    }
    String::from_utf8(response).expect("relay HTTP response should be utf8")
}

async fn assert_no_relay_close(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    match timeout(Duration::from_millis(100), socket.next()).await {
        Err(_) => {}
        Ok(Some(Ok(Message::Text(text)))) => {
            let envelope = serde_json::from_str::<RelayEnvelope>(&text)
                .expect("relay text frame should decode");
            assert!(
                !matches!(envelope, RelayEnvelope::Close { .. }),
                "active socket received relay close after accepted token expiry"
            );
        }
        Ok(other) => panic!("active socket closed unexpectedly: {other:?}"),
    }
}
