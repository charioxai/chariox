//! Public loopback fixtures; no persisted credential access.
use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

fn profile(api_url: String) -> PersistedCloudRelayProfile {
    PersistedCloudRelayProfile {
        kernel_id: None,
        kernel_credential: None,
        kernel_public_key_thumbprint: None,
        api_url,
        account_id: "account-fixture".into(),
        user_id: "user-fixture".into(),
        realm_id: "realm-fixture".into(),
        machine_id: Some("machine-a".into()),
        machine_credential: Some("synthetic-machine-credential".into()),
        ..Default::default()
    }
}

fn canonical_ref() -> String {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/slice-worker-identity.json"
    )))
    .unwrap();
    fixture["cases"][0]["workerKernelRef"]
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn slice_worker_identity_rejects_unsafe_token_requests_before_network() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let profile = profile(format!("http://{}", listener.local_addr().unwrap()));
    for unsafe_ref in [
        "slice:drill".to_string(),
        canonical_ref().replacen("slice:", "slice:foreign", 1),
    ] {
        let error = issue_cloud_slice_runtime_token(&profile, &unsafe_ref, "kernel-a", None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("create a new slice"));
        let error =
            issue_cloud_slice_recovery_token(&profile, &unsafe_ref, "kernel-a", "public-fixture")
                .await
                .unwrap_err();
        assert!(error.to_string().contains("create a new slice"));
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

#[tokio::test]
async fn slice_worker_identity_bootstrap_active_and_recovery_requests_bind_same_machine() {
    for phase in ["bootstrap", "active", "recovery"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let profile = profile(format!("http://{}", listener.local_addr().unwrap()));
        let reader = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "loopback request did not arrive");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("loopback accept failed: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            let headers_end = loop {
                let size = stream.read(&mut chunk).unwrap();
                assert!(size > 0);
                bytes.extend_from_slice(&chunk[..size]);
                assert!(bytes.len() <= 16384);
                if let Some(index) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8(bytes[..headers_end].to_vec()).unwrap();
            assert!(headers.starts_with("POST /relay/token HTTP/1.1\r\n"));
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < headers_end + length {
                let size = stream.read(&mut chunk).unwrap();
                assert!(size > 0);
                bytes.extend_from_slice(&chunk[..size]);
                assert!(bytes.len() <= 16384);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&bytes[headers_end..headers_end + length]).unwrap();
            let response = r#"{"token":"synthetic-token","expiresAt":"2030-01-01T00:00:00Z"}"#;
            write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", response.len(), response).unwrap();
            body
        });
        let canonical = canonical_ref();
        let issued = match phase {
            "bootstrap" => {
                issue_cloud_slice_runtime_token(&profile, &canonical, "kernel-a", None).await
            }
            "active" => {
                issue_cloud_slice_runtime_token(
                    &profile,
                    &canonical,
                    "kernel-a",
                    Some("public-fixture"),
                )
                .await
            }
            _ => {
                issue_cloud_slice_recovery_token(&profile, &canonical, "kernel-a", "public-fixture")
                    .await
            }
        };
        issued.unwrap();
        let body = reader.join().unwrap();
        assert_eq!(body["subject"], canonical);
        assert_eq!(body["subjectKind"], "kernel");
        assert_eq!(body["machineId"], "machine-a");
        assert_eq!(body["allowedTargets"], serde_json::json!(["kernel-a"]));
        assert_eq!(body["machineCredential"], "synthetic-machine-credential");
        assert!(body.get("sessionToken").is_none());
        if phase != "bootstrap" {
            assert!(body["publicKeyThumbprint"].as_str().is_some());
        }
    }
}
