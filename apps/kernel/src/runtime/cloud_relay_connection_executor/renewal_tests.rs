//! MP-08 / MP-09 / MP-10 / MP-11: issuer expiry through the real renewal entry point.
use super::*;
use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn renewal_persists_the_issued_expiry_instead_of_the_requested_lifetime() {
    let root = crate::test_support::TestWorktree::new("relay-issued-expiry");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    let expiry = (crate::session::unix_epoch_ms() / 1_000 + 120) * 1_000;
    let claims = serde_json::json!({"exp": expiry / 1_000,
        "public_key_thumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)});
    let token = format!("header.{}.signature", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap()));
    let response = serde_json::json!({"token": token, "expiresAt": chrono::DateTime::from_timestamp_millis(expiry.try_into().unwrap()).unwrap().to_rfc3339()}).to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        api_url: format!("http://{}", listener.local_addr().unwrap()),
        relay_url: "wss://relay.example.test".into(),
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-enrollment".into()),
        ..Default::default()
    });
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 16_384];
        let read = stream.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..read]).starts_with("POST /relay/token "));
        let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len());
        stream.write_all(header.as_bytes()).await.unwrap();
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let projection = app.config_projection_store();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 1);
    ensure_cloud_relay_connection(&router.runtime_state(), &projection).await.unwrap();
    server.await.unwrap();
    let saved = projection.snapshot();
    assert_eq!(saved.cloud_relay.as_ref().unwrap().token_expires_at_ms, Some(expiry));
    assert!(!crate::runtime::cloud_relay_control::cloud_relay_token_refresh_due(&saved, expiry - 61_000));
    assert!(crate::runtime::cloud_relay_control::cloud_relay_token_refresh_due(&saved, expiry - 60_000));
}
