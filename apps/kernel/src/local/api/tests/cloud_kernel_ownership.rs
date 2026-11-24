//! Local protocol-478 drill through the real router and kernel-private state.
use super::*;
use crate::local::{
    CloudRelayStatusRequest, ConnectCloudRelayRequest, LogoutCloudRelayRequest,
    PollCloudRelayLoginRequest, StartCloudRelayLoginRequest,
};
use std::io::{Read, Write};
use std::net::TcpListener;

#[test]
fn cloud_kernel_ownership_local_router_drill() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!("chariox-kauth-router-{}", rand::random::<u64>()));
    fs::create_dir(&root).unwrap();
    std::env::set_var("CHARIOX_HOME", &root);
    let mut config = DaemonConfig::for_tests();
    config.daemon_id = "kernel-a".into();
    config.host_machine_id = "machine-fixture".into();
    config.user_config.state.path = Some(root.join("a/state.db").to_string_lossy().into());
    let key = crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let profile = serde_json::json!({"email":"owner@example.test", "accountId":"account-a", "userId":"owner-a", "accountSlug":"fixture", "realmId":"realm-a", "relayUrl":"wss://relay.example.test", "issuerId":"fixture", "kernelId":"kernel-a", "machineId":"machine-fixture", "publicKeyThumbprint":key});
    // MP-08 / MP-10 / MP-11: begin the HTTP fixture deadline after router startup.
    let mut sibling_config = config.clone();
    sibling_config.daemon_id = "kernel-b".into();
    sibling_config.user_config.state.path = Some(root.join("b/state.db").to_string_lossy().into());
    let sibling = LocalRouterTestHarness::with_config(sibling_config);
    let harness = LocalRouterTestHarness::with_config(config);
    let claims = {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::json!({"public_key_thumbprint":key,"exp":4102444800_u64}).to_string(),
        )
    };
    let fixture = serve_cloud(
        listener,
        vec![
            ("/auth/device/start", "200 OK", device_start()),
            (
                "/auth/device/poll",
                "200 OK",
                serde_json::json!({"status":"approved", "profile":profile, "kernelCredential":"synthetic-kernel-grant"}),
            ),
            (
                "/relay/token",
                "200 OK",
                serde_json::json!({"token":format!("header.{claims}.signature"), "expiresAt":"2099-01-01T00:00:00Z"}),
            ),
            (
                "/kernels/unlink",
                "503 Service Unavailable",
                serde_json::json!({"code":"dependency_unavailable"}),
            ),
            ("/kernels/unlink", "200 OK", serde_json::json!({"ok":true})),
        ],
    );
    harness
        .dispatch(LocalDaemonRequest::StartCloudRelayLogin(
            StartCloudRelayLoginRequest {
                api_url: url.clone(),
                client_id: Some("old-terminal-id".into()),
                client_alias: None,
                machine_id: None,
            },
        ))
        .unwrap();
    let approved = harness
        .dispatch(LocalDaemonRequest::PollCloudRelayLogin(
            PollCloudRelayLoginRequest {
                api_url: url,
                device_code: "synthetic-device".into(),
                supports_access_denied: None,
            },
        ))
        .unwrap();
    assert!(!serde_json::to_string(&approved)
        .unwrap()
        .contains("synthetic-kernel-grant"));
    let connected = harness
        .dispatch(LocalDaemonRequest::ConnectCloudRelay(
            ConnectCloudRelayRequest,
        ))
        .unwrap();
    let connected = serde_json::to_value(connected).unwrap();
    assert!(connected["CloudRelayConnected"].get("token").is_none());
    assert_eq!(
        connected["CloudRelayConnected"]["profile"]["kernel_id"],
        "kernel-a"
    );
    assert!(matches!(
        sibling
            .dispatch(LocalDaemonRequest::CloudRelayStatus(
                CloudRelayStatusRequest
            ))
            .unwrap(),
        LocalDaemonResponse::CloudRelayStatus { profile: None }
    ));
    let logout = || {
        LocalDaemonRequest::LogoutCloudRelay(LogoutCloudRelayRequest {
            revoke_client: false,
            revoke_machine: false,
        })
    };
    assert!(harness.dispatch(logout()).is_err());
    assert!(matches!(
        harness
            .dispatch(LocalDaemonRequest::CloudRelayStatus(
                CloudRelayStatusRequest
            ))
            .unwrap(),
        LocalDaemonResponse::CloudRelayStatus { profile: Some(_) }
    ));
    harness.dispatch(logout()).unwrap();
    assert!(matches!(
        harness
            .dispatch(LocalDaemonRequest::CloudRelayStatus(
                CloudRelayStatusRequest
            ))
            .unwrap(),
        LocalDaemonResponse::CloudRelayStatus { profile: None }
    ));
    let requests = fixture.join().unwrap();
    assert_eq!(requests[0]["enrollmentKind"], "KERNEL");
    assert_eq!(requests[0]["kernelId"], "kernel-a");
    assert!(requests[0].get("clientId").is_none());
    assert!(requests[2].get("machineCredential").is_none());
    assert!(requests[2].get("sessionToken").is_none());
    assert_eq!(requests[3]["kernelId"], "kernel-a");
    let durable: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("a/cloud-relay.json")).unwrap()).unwrap();
    assert!(durable["profile"].is_null());
    assert_eq!(durable["former_cloud_kernel_id"], "kernel-a");
    assert!(durable["relay_token"].is_null());
    drop(harness);
    drop(sibling);
    fs::remove_dir_all(root).unwrap();
}

// A kernel linked to one Cloud account refuses an approval for another and withdraws it in Cloud at once: Cloud lists the
// kernel from the approval on, and a refused enrollment left there opens as a kernel that never connects.
#[test]
fn cloud_enrollment_for_another_account_is_withdrawn_in_cloud() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let temp = crate::test_support::TestTempDir::new("kauth-conflict");
    let root = temp.path();
    std::env::set_var("CHARIOX_HOME", root);
    let mut config = DaemonConfig::for_tests();
    config.daemon_id = "kernel-a".into();
    config.host_machine_id = "machine-fixture".into();
    config.user_config.state.path = Some(root.join("a/state.db").to_string_lossy().into());
    let key = crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let approval = |account: &str, credential: &str| serde_json::json!({"status":"approved", "kernelCredential":credential, "profile":{"email":"owner@example.test", "accountId":account, "userId":format!("owner-{account}"), "accountSlug":"fixture", "realmId":format!("realm-{account}"), "relayUrl":"wss://relay.example.test", "issuerId":"fixture", "kernelId":"kernel-a", "machineId":"machine-fixture", "publicKeyThumbprint":key}});
    let harness = LocalRouterTestHarness::with_config(config);
    let fixture = serve_cloud(
        listener,
        vec![
            ("/auth/device/start", "200 OK", device_start()),
            (
                "/auth/device/poll",
                "200 OK",
                approval("account-a", "synthetic-grant-a"),
            ),
            ("/auth/device/start", "200 OK", device_start()),
            (
                "/auth/device/poll",
                "200 OK",
                approval("account-b", "synthetic-grant-b"),
            ),
            ("/kernels/unlink", "200 OK", serde_json::json!({"ok":true})),
        ],
    );
    let enroll = || {
        harness
            .dispatch(LocalDaemonRequest::StartCloudRelayLogin(
                StartCloudRelayLoginRequest {
                    api_url: url.clone(),
                    client_id: None,
                    client_alias: None,
                    machine_id: None,
                },
            ))
            .unwrap();
        harness.dispatch(LocalDaemonRequest::PollCloudRelayLogin(
            PollCloudRelayLoginRequest {
                api_url: url.clone(),
                device_code: "synthetic-device".into(),
                supports_access_denied: None,
            },
        ))
    };
    enroll().unwrap();
    let refused = enroll().unwrap_err().to_string();
    assert!(
        refused.contains("Cloud account conflicts with this root"),
        "{refused}"
    );
    assert!(refused.contains("withdrawn in Cloud"), "{refused}");
    assert!(!refused.contains("synthetic-grant-b"), "{refused}");
    let status = serde_json::to_value(
        harness
            .dispatch(LocalDaemonRequest::CloudRelayStatus(
                CloudRelayStatusRequest,
            ))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        status["CloudRelayStatus"]["profile"]["account_id"], "account-a",
        "the kernel keeps its own account"
    );
    let requests = fixture.join().unwrap();
    assert_eq!(
        requests[4],
        serde_json::json!({"kernelCredential":"synthetic-grant-b", "accountId":"account-b", "realmId":"realm-account-b", "kernelId":"kernel-a"}),
        "the refused approval is unlinked with its own credential"
    );
    drop(harness);
}

fn device_start() -> serde_json::Value {
    serde_json::json!({"deviceCode":"synthetic-device", "userCode":"SAFE-CODE", "verificationUrl":"https://cloud.example.test/device", "expiresAt":"2099-01-01T00:00:00Z", "intervalSeconds":1})
}

/// A Cloud that answers each expected request in order and returns the request bodies it saw.
fn serve_cloud(
    listener: TcpListener,
    exchanges: Vec<(&'static str, &'static str, serde_json::Value)>,
) -> thread::JoinHandle<Vec<serde_json::Value>> {
    thread::spawn(move || {
        let mut observed = Vec::new();
        for (expected, status, response) in exchanges {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "Cloud fixture timed out waiting for {expected}"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("Cloud fixture accept failed: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let (start, length) = loop {
                let mut chunk = [0; 4096];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
                if let Some(end) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
                    let header = std::str::from_utf8(&bytes[..end]).unwrap();
                    assert_eq!(
                        header
                            .lines()
                            .next()
                            .unwrap()
                            .split_whitespace()
                            .nth(1)
                            .unwrap(),
                        expected
                    );
                    let length = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    break (end + 4, length);
                }
            };
            while bytes.len() < start + length {
                let mut chunk = [0; 4096];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&bytes[start..start + length]).unwrap();
            observed.push(body);
            let response = response.to_string();
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
        observed
    })
}
