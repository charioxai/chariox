//! MP-08/MP-09/MP-10/MP-11: bootstrap enrollment must survive ordinary relay renewal.
use super::*;
use crate::local::test_support::LocalRouterTestHarness;
use crate::local::{ConnectCloudRelayRequest, LocalDaemonRequest};
use std::io::{Read, Write};
use std::net::TcpListener;

#[test]
fn managed_bootstrap_exchange_keeps_its_activity_credential_through_relay_connection() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let _restore = EnvironmentRestoreGuard::capture([
        "HOME",
        "CHARIOX_HOME",
        "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
        MANAGED_PROVIDER_TOPOLOGY_ENV,
        "CHARIOX_DISPOSABLE_WORKER_RECEIPT",
    ]);
    std::env::remove_var("CHARIOX_DISPOSABLE_WORKER_RECEIPT");
    for status in [
        BootstrapReceiptStatus::Exchanged,
        BootstrapReceiptStatus::Confirmed,
    ] {
        let fixture = Fixture::new(&format!("relay-enrollment-{status:?}"));
        std::env::set_var("HOME", &fixture.config.process_home);
        std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
        std::env::set_var(
            "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
            &fixture.config.receipt_path,
        );
        std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "shared_host");
        let mut config = DaemonConfig::for_tests();
        config.daemon_id = "managed-kernel-1".into();
        config.host_machine_id = "managed-machine-1".into();
        config.user_config.state.path = Some(
            fixture
                .config
                .chariox_home
                .join("kernel/state.db")
                .display()
                .to_string(),
        );
        let mut receipt = path1_test_receipt(&fixture.release_digest, 1, status, None);
        receipt.schema_version = 2;
        receipt.relay_public_key = config.relay_public_key.clone();
        receipt.persist(&fixture.config.receipt_path).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut profile = path1_test_profile();
        profile.api_url = format!("http://{}", listener.local_addr().unwrap());
        config.cloud_relay = Some(profile.clone());
        let harness = LocalRouterTestHarness::with_config(config.clone());
        let key =
            crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key);
        let server = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "Cloud fixture deadline"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("Cloud fixture: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let (start, length, path) = loop {
                let mut chunk = [0; 4096];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
                if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                    let path = headers
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .to_string();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    break (end + 4, length, path);
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
            let (status, response) = if path == "/relay/token" {
                let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                    serde_json::json!({"public_key_thumbprint":key,"exp":4102444800_u64})
                        .to_string(),
                );
                (
                    "201 Created",
                    serde_json::json!({"token":format!("header.{claims}.signature"),"expiresAt":"2099-01-01T00:00:00Z"}),
                )
            } else {
                (
                    "403 Forbidden",
                    serde_json::json!({"error":{"code":"authorization_denied"}}),
                )
            };
            let response = response.to_string();
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
            (path, body)
        });
        let connected = harness.dispatch(LocalDaemonRequest::ConnectCloudRelay(
            ConnectCloudRelayRequest,
        ));
        let (path, body) = server.join().unwrap();
        let retained = harness
            .with_app(|app| app.config().cloud_relay.clone())
            .unwrap();
        drop(harness);
        fixture.cleanup();
        assert_eq!(
            path, "/relay/token",
            "{status:?} bootstrap must use its dedicated enrollment, never legacy migration"
        );
        assert!(connected.is_ok());
        assert_eq!(
            body["machineCredential"].as_str(),
            profile.machine_credential.as_deref()
        );
        assert!(body.get("kernelCredential").is_none());
        assert_eq!(retained.machine_credential, profile.machine_credential);
        assert!(retained.kernel_credential.is_none());
    }
}

#[test]
fn managed_bootstrap_relay_enrollment_requires_its_exact_identity_and_profile() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("relay-enrollment-binding");
    let _restore = EnvironmentRestoreGuard::capture([
        "HOME",
        "CHARIOX_HOME",
        "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
        MANAGED_PROVIDER_TOPOLOGY_ENV,
        "CHARIOX_DISPOSABLE_WORKER_RECEIPT",
    ]);
    std::env::set_var("HOME", &fixture.config.process_home);
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    std::env::set_var(
        "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
        &fixture.config.receipt_path,
    );
    std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "shared_host");
    std::env::remove_var("CHARIOX_DISPOSABLE_WORKER_RECEIPT");
    let mut config = DaemonConfig::for_tests();
    config.daemon_id = "managed-kernel-1".into();
    config.host_machine_id = "managed-machine-1".into();
    config.cloud_relay = Some(path1_test_profile());
    for status in [
        BootstrapReceiptStatus::Exchanged,
        BootstrapReceiptStatus::Confirmed,
    ] {
        let mut receipt = path1_test_receipt(&fixture.release_digest, 1, status, None);
        receipt.schema_version = 2;
        receipt.relay_public_key = config.relay_public_key.clone();
        receipt.persist(&fixture.config.receipt_path).unwrap();
        assert!(super::super::uses_bootstrap_cloud_enrollment(&config).unwrap());
        for field in ["kernel", "machine", "key", "profile-machine", "credential"] {
            let mut foreign = config.clone();
            match field {
                "kernel" => foreign.daemon_id = "foreign-kernel".into(),
                "machine" => foreign.host_machine_id = "foreign-machine".into(),
                "key" => foreign.relay_public_key = "foreign-key".into(),
                "profile-machine" => {
                    foreign.cloud_relay.as_mut().unwrap().machine_id =
                        Some("foreign-machine".into())
                }
                _ => foreign.cloud_relay.as_mut().unwrap().machine_credential = None,
            }
            assert!(
                super::super::uses_bootstrap_cloud_enrollment(&foreign).is_err(),
                "{status:?}/{field}"
            );
        }
    }
    std::env::remove_var("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT");
    std::env::remove_var(MANAGED_PROVIDER_TOPOLOGY_ENV);
    assert!(
        !super::super::uses_bootstrap_cloud_enrollment(&config).unwrap(),
        "co-resident receipt cannot select an ordinary kernel"
    );
    fixture.cleanup();
}
