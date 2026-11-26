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

/// Row 563 review: a renewal whose own save failed leaves the projection's profile ahead of the app's.
/// The next renewal passes that projected profile as `expected`; it must still be saved, and an
/// older delayed save must not replace a newer one.
#[tokio::test]
async fn a_later_renewal_is_saved_after_the_first_save_failed() {
    let root = crate::test_support::TestWorktree::new("relay-renew-after-failed-save");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        api_url: "http://127.0.0.1:9".into(),
        relay_url: "wss://relay.example.test".into(),
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-enrollment".into()),
        token_expires_at_ms: Some(1_000),
        ..Default::default()
    });
    let durable = config.cloud_relay.clone().unwrap();
    let app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 1);
    let state = router.runtime_state();
    // The first renewal's save never ran: the projection (and so `expected`) is ahead of the app.
    let mut ahead = durable.clone();
    ahead.token_expires_at_ms = Some(5_000);
    let mut second = durable.clone();
    second.token_expires_at_ms = Some(9_000);
    state
        .finish_cloud_relay_renewal(ahead.clone(), Some((second, "second".into())))
        .await
        .unwrap();
    {
        let app = app.lock().await;
        assert_eq!(app.config().relay_token.as_deref(), Some("second"));
        assert_eq!(
            app.config()
                .cloud_relay
                .as_ref()
                .unwrap()
                .token_expires_at_ms,
            Some(9_000)
        );
    }
    // The delayed first save arrives last and is older: it changes nothing.
    state
        .finish_cloud_relay_renewal(durable, Some((ahead, "first-late".into())))
        .await
        .unwrap();
    let app = app.lock().await;
    assert_eq!(app.config().relay_token.as_deref(), Some("second"));
    assert_eq!(
        app.config()
            .cloud_relay
            .as_ref()
            .unwrap()
            .token_expires_at_ms,
        Some(9_000)
    );
}

/// Row 563 review: an owner removal recorded while a renewal's save is still queued must survive it.
#[tokio::test]
async fn a_delayed_renewal_save_does_not_undo_a_recorded_removal() {
    let root = crate::test_support::TestWorktree::new("relay-renew-after-removal");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        api_url: "http://127.0.0.1:9".into(),
        relay_url: "wss://relay.example.test".into(),
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-enrollment".into()),
        token_expires_at_ms: Some(1_000),
        ..Default::default()
    });
    let requested = config.cloud_relay.clone().unwrap();
    let app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 1);
    let state = router.runtime_state();
    state
        .mark_cloud_kernel_removal(
            requested.clone(),
            crate::config::CloudKernelRemoval::Pending,
        )
        .await
        .unwrap();
    let mut renewed = requested.clone();
    renewed.token_expires_at_ms = Some(9_000);
    state
        .finish_cloud_relay_renewal(requested, Some((renewed, "late".into())))
        .await
        .unwrap();
    let app = app.lock().await;
    assert!(
        app.config()
            .cloud_relay
            .as_ref()
            .unwrap()
            .kernel_removal_pending
    );
    assert_ne!(app.config().relay_token.as_deref(), Some("late"));
}

/// Row 563 review: after Undo Remove the app still carries the removal mark until a renewal save
/// lands. A save whose first attempt failed must stay recoverable by the next renewal, which is
/// accepted against the baseline the app and disk still carry (`expected` keeps the removal mark).
#[tokio::test]
async fn a_renewal_save_after_undo_remove_recovers_once_storage_answers() {
    let root = crate::test_support::TestWorktree::new("relay-renew-after-undo-failed-save");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        api_url: "http://127.0.0.1:9".into(),
        relay_url: "wss://relay.example.test".into(),
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-enrollment".into()),
        token_expires_at_ms: Some(1_000),
        ..Default::default()
    });
    let enrolled = config.cloud_relay.clone().unwrap();
    let app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 1);
    let state = router.runtime_state();
    state
        .mark_cloud_kernel_removal(enrolled.clone(), crate::config::CloudKernelRemoval::Pending)
        .await
        .unwrap();
    let removed = app.lock().await.config().cloud_relay.clone().unwrap();
    assert!(removed.kernel_removal_pending);
    // The first renewal's save failed; the next one carries the baseline's removal mark.
    let mut restored = removed.clone();
    restored.kernel_removal_pending = false;
    restored.token_expires_at_ms = Some(9_000);
    state
        .finish_cloud_relay_renewal(removed, Some((restored, "after-undo".into())))
        .await
        .unwrap();
    let app = app.lock().await;
    let saved = app.config().cloud_relay.clone().unwrap();
    assert!(!saved.kernel_removal_pending);
    assert_eq!(app.config().relay_token.as_deref(), Some("after-undo"));
}

/// Row 563 review (f1ccb33): T1 then T2 installed over durable T0; only T1's save lands first.
/// T2 stays projected through T1's publish and an unrelated one, and T2's save is accepted
/// afterwards, also when T0 carried the Undo Remove mark that T1's save cleared.
#[tokio::test]
async fn an_intermediate_renewal_save_keeps_the_newest_renewal_projected_and_saveable() {
    for removal_pending in [false, true] {
        let root = crate::test_support::TestWorktree::new("relay-renew-intermediate-save");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
        config.relay_url = Some("wss://relay.example.test".into());
        config.relay_token = Some("t0".into());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url: "http://127.0.0.1:9".into(),
            relay_url: "wss://relay.example.test".into(),
            kernel_id: Some(config.daemon_id.clone()),
            kernel_credential: Some("synthetic-enrollment".into()),
            token_expires_at_ms: Some(1_000),
            kernel_removal_pending: removal_pending,
            ..Default::default()
        });
        let durable = config.cloud_relay.clone().unwrap();
        let app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let projection = app.config_projection_store();
        let app = Arc::new(tokio::sync::Mutex::new(app));
        let router =
            crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 1);
        let state = router.runtime_state();
        let mut t1 = durable.clone();
        t1.kernel_removal_pending = false;
        t1.token_expires_at_ms = Some(9_000);
        let mut t2 = t1.clone();
        t2.token_expires_at_ms = Some(19_000);
        // What the executor passes as `expected` for each save: the baseline at install time.
        let expected_t1 = durable.clone();
        projection
            .install_cloud_relay_renewal(&projection.snapshot(), t1.clone(), "t1".into())
            .unwrap();
        let mut expected_t2 = projection.snapshot().cloud_relay.unwrap();
        expected_t2.kernel_removal_pending = projection.unsaved_renewal_removal_baseline().unwrap();
        projection
            .install_cloud_relay_renewal(&projection.snapshot(), t2.clone(), "t2".into())
            .unwrap();

        state
            .finish_cloud_relay_renewal(expected_t1, Some((t1, "t1".into())))
            .await
            .unwrap();
        assert_eq!(
            app.lock().await.config().relay_token.as_deref(),
            Some("t1"),
            "removal_pending={removal_pending}: T1's save lands"
        );
        assert_eq!(
            projection.snapshot().relay_token.as_deref(),
            Some("t2"),
            "removal_pending={removal_pending}: T1's publish keeps T2 projected"
        );
        let mut unrelated = app.lock().await.config().clone();
        unrelated.runtime_mcp_port = 4321;
        projection.update(unrelated);
        assert_eq!(
            projection.snapshot().relay_token.as_deref(),
            Some("t2"),
            "removal_pending={removal_pending}: an unrelated publish keeps T2 projected"
        );

        state
            .finish_cloud_relay_renewal(expected_t2, Some((t2, "t2".into())))
            .await
            .unwrap();
        let app = app.lock().await;
        assert_eq!(
            app.config().relay_token.as_deref(),
            Some("t2"),
            "removal_pending={removal_pending}: T2's save lands after T1's"
        );
        let saved = app.config().cloud_relay.clone().unwrap();
        assert!(!saved.kernel_removal_pending);
        assert_eq!(saved.token_expires_at_ms, Some(19_000));
        assert_eq!(projection.snapshot().relay_token.as_deref(), Some("t2"));
    }
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
