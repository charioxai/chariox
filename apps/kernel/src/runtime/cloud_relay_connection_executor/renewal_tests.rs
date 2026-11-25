//! MP-08 / MP-09 / MP-10 / MP-11: issuer expiry through the real renewal entry point.
use super::*;
use crate::local::LocalDaemonRequest;
use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn undoing_owner_removal_recovers_the_existing_kernel_enrollment() {
    let root = crate::test_support::TestWorktree::new("relay-undo-removal");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let expiry = crate::session::unix_epoch_ms() / 1_000 + 180;
    let claims = serde_json::json!({
        "exp": expiry,
        "public_key_thumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
    });
    let token = format!(
        "header.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap())
    );
    let expires_at = chrono::DateTime::from_timestamp(expiry.try_into().unwrap(), 0)
        .unwrap()
        .to_rfc3339();
    let profile = crate::config::PersistedCloudRelayProfile {
        api_url: format!("http://{}", listener.local_addr().unwrap()),
        relay_url: "ws://127.0.0.1:1".into(),
        user_id: "cloud-owner".into(),
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-enrollment".into()),
        ..Default::default()
    };
    config.cloud_relay = Some(profile.clone());
    let server = tokio::spawn(async move {
        let refused = r#"{"error":{"code":"kernel_removal_pending","message":"Undo Remove"}}"#;
        let restored = serde_json::json!({"token": token, "expiresAt": expires_at}).to_string();
        for (status, body) in [
            ("409 Conflict", refused),
            ("201 Created", restored.as_str()),
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 16_384];
            let read = stream.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("POST /relay/token "));
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(body.as_bytes()).await.unwrap();
        }
    });
    let app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let projection = app.config_projection_store();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    let refusal = ensure_cloud_relay_connection(&router.runtime_state(), &projection)
        .await
        .expect_err("owner removal pauses Cloud connection");
    assert!(refusal.to_string().contains("Undo Remove in Settings"));
    let pending = projection.snapshot();
    let pending_profile = pending.cloud_relay.as_ref().unwrap();
    assert!(pending_profile.kernel_removal_pending);
    assert_eq!(pending_profile.kernel_credential, profile.kernel_credential);
    assert!(pending.relay_token.is_none());
    ensure_cloud_relay_connection(&router.runtime_state(), &projection)
        .await
        .expect("Undo restores the existing enrollment");
    server.await.unwrap();
    let restored = projection.snapshot();
    let restored_profile = restored.cloud_relay.as_ref().unwrap();
    assert_eq!(restored_profile.kernel_removal(), None);
    assert_eq!(restored_profile.kernel_id, profile.kernel_id);
    assert_eq!(
        restored_profile.kernel_credential,
        profile.kernel_credential
    );
    assert!(restored.relay_token.is_some());
}

#[tokio::test]
async fn renewal_persists_the_issued_expiry_instead_of_the_requested_lifetime() {
    let root = crate::test_support::TestWorktree::new("relay-issued-expiry");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    let expiry = (crate::session::unix_epoch_ms() / 1_000 + 120) * 1_000;
    let claims = serde_json::json!({"exp": expiry / 1_000,
        "public_key_thumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)});
    let token = format!(
        "header.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap())
    );
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
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    ensure_cloud_relay_connection(&router.runtime_state(), &projection)
        .await
        .unwrap();
    server.await.unwrap();
    let saved = projection.snapshot();
    assert_eq!(
        saved.cloud_relay.as_ref().unwrap().token_expires_at_ms,
        Some(expiry)
    );
    assert!(
        !crate::runtime::cloud_relay_control::cloud_relay_token_refresh_due(
            &saved,
            expiry - 61_000
        )
    );
    assert!(
        crate::runtime::cloud_relay_control::cloud_relay_token_refresh_due(&saved, expiry - 60_000)
    );
}

/// Row 563 (presence under disk stalls): a stalled app lock (a slow disk under a durable write)
/// must not hold back the renewed relay token. The connector reads its token from the projection,
/// so renewal that waits for the app lock lets the first token expire and the kernel goes offline.
#[tokio::test]
async fn renewal_reaches_the_projection_while_the_app_lock_is_held() {
    let root = crate::test_support::TestWorktree::new("relay-renew-stalled-app");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    let expiry = (crate::session::unix_epoch_ms() / 1_000 + 120) * 1_000;
    let claims = serde_json::json!({"exp": expiry / 1_000,
        "public_key_thumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)});
    let token = format!(
        "header.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap())
    );
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
        let _ = stream.read(&mut request).await.unwrap();
        let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len());
        stream.write_all(header.as_bytes()).await.unwrap();
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let projection = app.config_projection_store();
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 1);
    // The held guard stands for a durable write stalled on a slow disk under the app lock.
    let stalled_disk = app.lock().await;
    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        ensure_cloud_relay_connection(&router.runtime_state(), &projection),
    )
    .await;
    let installed = projection.snapshot().relay_token;
    drop(stalled_disk);
    // Let a late renewal settle so the server task and the app are released either way.
    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
    assert!(
        outcome.is_ok_and(|result| result.is_ok()),
        "renewal must not wait for the app lock behind a stalled disk write"
    );
    assert_eq!(
        installed.as_deref(),
        Some(token.as_str()),
        "the connector reads the projection: the renewed token must be there before the stall ends"
    );
    // The durable write follows once the disk answers.
    let mut durable = None;
    for _ in 0..100 {
        durable = app.lock().await.config().relay_token.clone();
        if durable.as_deref() == Some(token.as_str()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(durable.as_deref(), Some(token.as_str()));
}

/// The body is a long async test: it overflows the default 2 MiB test stack in a debug build, so
/// it runs on a large-stack thread like the other long relay tests.
#[test]
fn removed_cloud_kernel_keeps_its_owner_identity_on_renewal() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(removed_cloud_kernel_keeps_its_owner_identity())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn removed_cloud_kernel_keeps_its_owner_identity() {
    let root = crate::test_support::TestWorktree::new("relay-unlinked-owner");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        api_url: format!("http://{}", listener.local_addr().unwrap()),
        relay_url: "wss://relay.example.test".into(),
        user_id: "cloud-owner".into(),
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-enrollment".into()),
        ..Default::default()
    });
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 16_384];
        let read = stream.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..read]).starts_with("POST /relay/token "));
        let body = r#"{"error":{"code":"kernel_unlinked","message":"Kernel was removed"}}"#;
        let header = format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(header.as_bytes()).await.unwrap();
        stream.write_all(body.as_bytes()).await.unwrap();
    });
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(root.session_request().with_owner_user_id("cloud-owner"))
        .unwrap();
    let (workflow, endpoint) = {
        let mut sessions = app.sessions_mut();
        let workflow = sessions
            .create_workflow_controlled_by_metaagent(session.id(), None, Some(agent.id().into()))
            .unwrap();
        let node = sessions
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .unwrap();
        let endpoint = sessions
            .create_workflow_endpoint(session.id(), workflow.id(), node.id(), None)
            .unwrap();
        let endpoint = sessions
            .set_workflow_endpoint_owner(
                session.id(),
                workflow.id(),
                endpoint.id(),
                "cloud-owner".into(),
            )
            .unwrap();
        (workflow, endpoint)
    };
    let projection = app.config_projection_store();
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 1);
    let result = ensure_cloud_relay_connection(&router.runtime_state(), &projection).await;
    server.await.unwrap();
    assert!(
        result.is_err(),
        "a removed enrollment cannot renew relay access"
    );
    assert!(
        projection.snapshot().cloud_relay.is_some(),
        "the owner identity must not disappear when Cloud removes the kernel"
    );
    assert!(
        result.unwrap_err().to_string().contains("/cloud link"),
        "the refusal should tell the owner how to re-link"
    );
    let saved = projection.snapshot();
    let profile = saved.cloud_relay.as_ref().unwrap();
    assert!(profile.kernel_unlinked);
    assert!(profile.kernel_credential.is_none());
    assert!(saved.relay_token.is_none());
    assert_eq!(
        crate::runtime::self_host_terminal_grants::owner_user_id(&saved),
        "cloud-owner"
    );
    let second = ensure_cloud_relay_connection(&router.runtime_state(), &projection).await;
    assert!(second.unwrap_err().to_string().contains("/cloud link"));
    let (invoked, _) = router
        .runtime_state()
        .execute_workflow_request(
            LocalDaemonRequest::InvokeWorkflowEndpoint(
                crate::local::InvokeWorkflowEndpointRequest {
                    session_id: session.id().into(),
                    workflow_ref: workflow.id().into(),
                    endpoint_ref: endpoint.id().into(),
                    queue_ref: None,
                    prompt: Some("owner workflow".into()),
                    publication_invocation: None,
                },
            ),
            "cloud-owner".into(),
            None,
        )
        .await;
    assert!(invoked.unwrap_err().to_string().contains("/cloud link"));
    let log = app
        .lock()
        .await
        .load_session_history_entries(&session, None)
        .unwrap();
    assert_eq!(
        log.iter()
            .filter(
                |entry| entry.kind == crate::history::SessionHistoryEntryKind::Notice
                    && entry.text.contains("/cloud link")
            )
            .count(),
        1,
    );
}
