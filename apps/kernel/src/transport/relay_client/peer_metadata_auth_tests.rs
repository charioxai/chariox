// MP-08 / MP-10 / MP-11: metadata uses a client scope; peer registration retains kernel authority.
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_mp10_temporary_peer_uses_metadata_only_cloud_authority() {
    metadata_authority(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_mp10_connected_peer_cache_miss_uses_metadata_only_cloud_authority() {
    metadata_authority(true).await;
}

async fn metadata_authority(connected: bool) {
    let api = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", api.local_addr().unwrap());
    let api_task = tokio::spawn(async move {
        let (mut stream, _) = api.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0; 4096];
        let end = loop {
            let n = stream.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            assert!(bytes.len() < 16384);
            if let Some(i) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        while bytes.len() < end + length {
            let n = stream.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            assert!(bytes.len() < 16384);
        }
        let body: serde_json::Value = serde_json::from_slice(&bytes[end..end + length]).unwrap();
        assert_eq!(body["subjectKind"], "client");
        assert_eq!(
            body["allowedActions"],
            serde_json::json!(["client.metadata.read"])
        );
        assert_eq!(body["machineCredential"], "synthetic-machine-credential");
        assert!(body.get("sessionToken").is_none());
        let response =
            r#"{"token":"synthetic-metadata-client","expiresAt":"2030-01-01T00:00:00Z"}"#;
        stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", response.len(), response).as_bytes()).await.unwrap();
    });
    let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("ws://{}", relay.local_addr().unwrap());
    let relay_task = tokio::spawn(async move {
        let (stream, _) = relay.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let Some(Ok(Message::Text(text))) = socket.next().await else {
            panic!("metadata request missing");
        };
        let request: chariox_relay::protocol::RelayEnvelope = serde_json::from_str(&text).unwrap();
        let chariox_relay::protocol::RelayEnvelope::ClientMetadataRequest {
            request_id,
            auth_token,
            ..
        } = request
        else {
            panic!("metadata envelope missing");
        };
        assert_eq!(
            auth_token, "synthetic-metadata-client",
            "kernel registration credentials must not authenticate client metadata"
        );
        let kernel = serde_json::from_value(serde_json::json!({"kernel_id":"worker", "machine_id":"worker-machine", "public_key":"worker-public-key"})).unwrap();
        socket
            .send(Message::Text(
                serde_json::to_string(
                    &chariox_relay::protocol::RelayEnvelope::ClientMetadataResponse {
                        request_id,
                        machines: None,
                        kernels: None,
                        kernel: Some(kernel),
                        error: None,
                    },
                )
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let _ = timeout(Duration::from_secs(2), socket.next()).await;
    });
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(relay_url.clone());
    config.relay_token = Some("synthetic-kernel-registration".into());
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        api_url,
        relay_url,
        machine_id: Some("owner-machine".into()),
        machine_credential: Some("synthetic-machine-credential".into()),
        ..Default::default()
    });
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let target = chariox_relay::protocol::ClientTarget {
        daemon_id: Some("worker".into()),
        daemon_alias: None,
    };
    let request = RelayPeerRequest::Ping {
        value: "scope-check".into(),
    };
    let authorize = || {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(())
        } else {
            Err(DaemonError::LocalTransport {
                operation: "fixture admission recheck",
                message: "revoked after metadata".into(),
            })
        }
    };
    let result = if connected {
        enqueue_peer_request_via_connected_relay_authorized(
            &config,
            &state,
            target,
            request,
            Duration::from_secs(3),
            authorize,
        )
        .await
        .map(|_| ())
    } else {
        send_peer_request_via_temporary_connection_authorized(
            &config,
            target,
            request,
            Duration::from_secs(3),
            authorize,
        )
        .await
        .map(|_| ())
    };
    assert!(
        matches!(
            result,
            Err(DaemonError::LocalTransport {
                operation: "fixture admission recheck",
                ..
            })
        ),
        "metadata must succeed before admission is rechecked"
    );
    assert_eq!(
        config.relay_token.as_deref(),
        Some("synthetic-kernel-registration")
    );
    api_task.await.unwrap();
    relay_task.await.unwrap();
}
