//! MP-08/MP-10/MP-11: metadata discovery must never borrow a machine runtime grant.

use super::*;
use crate::config::PersistedCloudRelayProfile;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

async fn metadata_issuer(status: u16) -> (String, tokio::task::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let n = stream.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&chunk[..n]);
            assert!(request.len() <= 16 * 1024);
            if let Some(end) = request.windows(4).position(|p| p == b"\r\n\r\n") {
                break end + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        assert!(headers.starts_with("POST /relay/token HTTP/1.1\r\n"));
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        while request.len() < header_end + length {
            let n = stream.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&chunk[..n]);
            assert!(request.len() <= 16 * 1024);
        }
        let body = serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
        let response = if status == 200 {
            r#"{"token":"synthetic-metadata-grant","expiresAt":"2030-01-01T00:00:00Z"}"#
        } else {
            r#"{"error":"synthetic metadata issuance denied"}"#
        };
        let reason = if status == 200 { "OK" } else { "Forbidden" };
        let reply = format!("HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}", response.len());
        stream.write_all(reply.as_bytes()).await.unwrap();
        body
    });
    (url, server)
}

async fn metadata_relay(
    query: &RelayMetadataQuery,
    expected_token: &str,
) -> (String, tokio::task::JoinHandle<bool>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let expected = serde_json::to_value(query).unwrap();
    let expected_token = expected_token.to_string();
    let accepted = Arc::new(AtomicBool::new(false));
    let server_accepted = accepted.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        server_accepted.store(true, Ordering::Release);
        let mut socket = accept_async(stream).await.unwrap();
        let Some(Ok(Message::Text(text))) = socket.next().await else {
            panic!("metadata request missing");
        };
        let RelayEnvelope::ClientMetadataRequest {
            request_id,
            auth_token,
            query,
        } = serde_json::from_str(&text).unwrap()
        else {
            panic!("metadata request envelope missing");
        };
        assert_eq!(serde_json::to_value(query).unwrap(), expected);
        let scoped = auth_token == expected_token;
        socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientMetadataResponse {
                    request_id,
                    machines: Some(Vec::new()),
                    kernels: Some(Vec::new()),
                    kernel: None,
                    error: None,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let _ = timeout(Duration::from_secs(1), socket.next()).await;
        scoped
    });
    (url, server, accepted)
}

fn hosted_config(api_url: String, relay_url: String) -> DaemonConfig {
    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(relay_url.clone());
    config.relay_token = Some("synthetic-machine-runtime-grant".into());
    config.relay_request_timeout_ms = 1_000;
    config.cloud_relay = Some(PersistedCloudRelayProfile {
        api_url,
        relay_url,
        account_id: "account-fixture".into(),
        user_id: "user-fixture".into(),
        realm_id: "realm-fixture".into(),
        machine_id: Some("machine-fixture".into()),
        machine_credential: Some("synthetic-machine-credential".into()),
        cloud_session_token: Some("synthetic-session".into()),
        ..Default::default()
    });
    config
}

async fn require_scoped_metadata_grant(query: RelayMetadataQuery) {
    let (api_url, issuer) = metadata_issuer(200).await;
    let (relay_url, relay, _) = metadata_relay(&query, "synthetic-metadata-grant").await;
    let config = hosted_config(api_url, relay_url);
    let original_profile = config.cloud_relay.clone();
    query_relay_once(&config, query).await.unwrap();
    assert!(
        relay.await.unwrap(),
        "hosted metadata used the machine runtime grant"
    );
    let body = issuer.await.unwrap();
    assert_eq!(
        body["subject"],
        format!("relay-inventory:{}", config.daemon_id)
    );
    assert_eq!(body["subjectKind"], "client");
    assert_eq!(body["allowedActions"], json!(["client.metadata.read"]));
    assert_eq!(body["machineId"], "machine-fixture");
    assert_eq!(body["realmId"], "realm-fixture");
    assert_eq!(body["machineCredential"], "synthetic-machine-credential");
    assert!(body.get("sessionToken").is_none());
    assert_eq!(
        config.relay_token.as_deref(),
        Some("synthetic-machine-runtime-grant")
    );
    assert_eq!(config.cloud_relay, original_profile);
}

#[tokio::test]
async fn fresh_machine_query_uses_client_metadata_grant() {
    require_scoped_metadata_grant(RelayMetadataQuery::ListLiveMachines).await;
}

#[tokio::test]
async fn fresh_kernel_directory_query_uses_client_metadata_grant() {
    require_scoped_metadata_grant(RelayMetadataQuery::ListLiveKernelsForMachine {
        machine_ref: "worker-machine".into(),
    })
    .await;
}

#[tokio::test]
async fn fresh_kernel_alias_lookup_uses_client_metadata_grant() {
    require_scoped_metadata_grant(RelayMetadataQuery::GetLiveKernel {
        kernel_ref: "worker-alias".into(),
    })
    .await;
}

#[tokio::test]
async fn denied_metadata_issuance_never_falls_back_to_runtime_grant() {
    let (api_url, issuer) = metadata_issuer(403).await;
    let query = RelayMetadataQuery::ListLiveMachines;
    let (relay_url, relay, accepted) = metadata_relay(&query, "synthetic-metadata-grant").await;
    let config = hosted_config(api_url, relay_url);
    let result = query_relay_once(&config, query).await;
    assert!(result.is_err(), "denied metadata issuance must fail closed");
    issuer.await.unwrap();
    assert!(
        !accepted.load(Ordering::Acquire),
        "issuance failure must not open a relay socket"
    );
    relay.abort();
}

#[tokio::test]
async fn self_hosted_metadata_keeps_its_grant_with_an_unrelated_cloud_profile() {
    let (api_url, issuer) = metadata_issuer(200).await;
    let query = RelayMetadataQuery::ListLiveMachines;
    let (relay_url, relay, _) = metadata_relay(&query, "synthetic-machine-runtime-grant").await;
    let mut config = hosted_config(api_url, relay_url);
    config.cloud_relay.as_mut().unwrap().relay_url = "wss://unrelated-cloud.example/relay".into();
    query_relay_once(&config, query).await.unwrap();
    assert!(relay.await.unwrap());
    assert!(!issuer.is_finished());
    issuer.abort();
}

// MP-08 / MP-10 / MP-11: combined ownership/metadata selector contract.
#[tokio::test]
async fn enrolled_metadata_queries_bind_owner_targets_and_kernel_key() {
    for query in [
        RelayMetadataQuery::ListLiveMachines,
        RelayMetadataQuery::ListLiveKernelsForMachine {
            machine_ref: "worker".into(),
        },
        RelayMetadataQuery::GetLiveKernel {
            kernel_ref: "worker".into(),
        },
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let issuer = tokio::spawn(async move {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0_u8; 4096];
                let header_end = loop {
                    let n = stream.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                    assert!(request.len() <= 16 * 1024);
                    if let Some(end) = request.windows(4).position(|p| p == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let response = if index == 0 {
                    assert!(
                        headers.starts_with("GET /relay/targets?"),
                        "every metadata query must select owner targets first"
                    );
                    assert!(headers
                        .to_ascii_lowercase()
                        .contains("x-chariox-kernel-credential: synthetic-kernel-credential"));
                    json!({"targets": [
                        {"daemonId":"owner", "status":"ONLINE"},
                        {"daemonId":"worker", "status":"ONLINE"},
                        {"daemonId":"dormant", "status":"OFFLINE"},
                        {"daemonId":"unlinked", "status":"REVOKED"}
                    ]})
                    .to_string()
                } else {
                    assert!(headers.starts_with("POST /relay/token HTTP/1.1"));
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    while request.len() < header_end + length {
                        let n = stream.read(&mut chunk).await.unwrap();
                        assert!(n > 0);
                        request.extend_from_slice(&chunk[..n]);
                    }
                    let body: Value =
                        serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
                    assert_eq!(body["allowedTargets"], json!(["owner", "worker"]));
                    assert_eq!(body["allowedActions"], json!(["client.metadata.read"]));
                    assert_eq!(body["kernelCredential"], "synthetic-kernel-credential");
                    assert_eq!(body["publicKeyThumbprint"], "synthetic-kernel-key");
                    assert!(body.get("machineCredential").is_none());
                    assert!(body.get("sessionToken").is_none());
                    json!({"token":"synthetic-metadata-grant", "expiresAt":"2030-01-01T00:00:00Z"})
                        .to_string()
                };
                stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
            }
        });
        let (relay_url, relay, _) = metadata_relay(&query, "synthetic-metadata-grant").await;
        let mut config = hosted_config(api_url, relay_url);
        let profile = config.cloud_relay.as_mut().unwrap();
        profile.kernel_credential = Some("synthetic-kernel-credential".into());
        profile.kernel_public_key_thumbprint = Some("synthetic-kernel-key".into());
        query_relay_once(&config, query).await.unwrap();
        assert!(relay.await.unwrap());
        issuer.await.unwrap();
    }
}

// MP-08 / MP-10 / MP-11: historical account inventory must not block StartSlice.
#[tokio::test]
async fn eighty_six_targets_use_bounded_metadata_grants() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let issuer = tokio::spawn(async move {
        let mut granted = std::collections::BTreeSet::new();
        while granted.len() < 86 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            let header_end = loop {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                if let Some(end) = request.windows(4).position(|p| p == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let (status, response) = if headers.starts_with("GET /relay/targets?") {
                (
                    200,
                    json!({"targets": (0..86).map(|n| json!({"daemonId":format!("worker-{n:02}"),"status":"ONLINE"})).collect::<Vec<_>>() }),
                )
            } else {
                assert!(headers.starts_with("POST /relay/token HTTP/1.1"));
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                while request.len() < header_end + length {
                    let n = stream.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                }
                let body: Value =
                    serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
                let targets = body["allowedTargets"].as_array().unwrap();
                if targets.len() > 64 {
                    let response = "{\"error\":\"target limit exceeded\"}";
                    stream.write_all(format!("HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
                    return false;
                }
                assert_eq!(body["allowedActions"], json!(["client.metadata.read"]));
                for target in targets {
                    assert!(granted.insert(target.as_str().unwrap().to_string()));
                }
                (
                    200,
                    json!({"token":"synthetic-metadata-grant", "expiresAt":"2030-01-01T00:00:00Z"}),
                )
            };
            let response = response.to_string();
            stream.write_all(format!("HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
        }
        true
    });
    let relay_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("ws://{}", relay_listener.local_addr().unwrap());
    let relay = tokio::spawn(async move {
        for count in [64, 22] {
            let (stream, _) = relay_listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let Some(Ok(Message::Text(text))) = socket.next().await else {
                panic!("missing query")
            };
            let RelayEnvelope::ClientMetadataRequest {
                request_id,
                query: RelayMetadataQuery::ListLiveMachines,
                ..
            } = serde_json::from_str(&text).unwrap()
            else {
                panic!("unexpected query")
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::ClientMetadataResponse {
                        request_id,
                        machines: Some(vec![RelayMachinePresence {
                            machine_id: "shared-machine".into(),
                            machine_alias: None,
                            kernel_count: count,
                            available_providers: vec![],
                            provider_accounts: vec![],
                        }]),
                        kernels: None,
                        kernel: None,
                        error: None,
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            let _ = timeout(Duration::from_secs(1), socket.next()).await;
        }
    });
    let mut config = hosted_config(api_url, relay_url);
    let profile = config.cloud_relay.as_mut().unwrap();
    profile.kernel_credential = Some("synthetic-kernel-credential".into());
    profile.kernel_public_key_thumbprint = Some("synthetic-kernel-key".into());
    let result = timeout(Duration::from_secs(10), list_live_machines(&config))
        .await
        .unwrap();
    assert!(
        issuer.await.unwrap(),
        "an account with 86 targets exceeded the 64-target API bound"
    );
    let machines = result.unwrap();
    assert_eq!(machines.len(), 1);
    assert_eq!(machines[0].kernel_count, 86);
    relay.await.unwrap();
}
