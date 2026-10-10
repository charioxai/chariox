//! Account-paired client tokens use the linked operator session.

use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;

use super::{
    issue_cloud_runtime_token, CloudRuntimeTokenRequestOptions, CloudRuntimeTokenResponse,
};

pub(crate) async fn issue_cloud_account_client_runtime_token(
    profile: &PersistedCloudRelayProfile,
    subject: &str,
    options: CloudRuntimeTokenRequestOptions,
) -> Result<CloudRuntimeTokenResponse, DaemonError> {
    if profile
        .cloud_session_token
        .as_deref()
        .is_none_or(|token| token.trim().is_empty())
    {
        return Err(DaemonError::LocalTransport {
            operation: "issue cloud account client relay token",
            message: "account client relay token requires an authenticated Cloud session".into(),
        });
    }
    let mut session_profile = profile.clone();
    session_profile.machine_credential = None;
    issue_cloud_runtime_token(&session_profile, subject, "client", options).await
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};

    use serde_json::{json, Value};

    use super::*;

    fn token_server() -> (String, thread::JoinHandle<Option<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return None;
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("loopback token fixture: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 4096];
            let headers_end = loop {
                let size = stream.read(&mut chunk).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&chunk[..size]);
                assert!(request.len() <= 16 * 1024);
                if let Some(index) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8(request[..headers_end].to_vec()).unwrap();
            assert!(headers.starts_with("POST /relay/token HTTP/1.1\r\n"));
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            while request.len() < headers_end + length {
                let size = stream.read(&mut chunk).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&chunk[..size]);
                assert!(request.len() <= 16 * 1024);
            }
            let body = serde_json::from_slice(&request[headers_end..headers_end + length]).unwrap();
            let response =
                r#"{"token":"synthetic-client-token","expiresAt":"2030-01-01T00:00:00Z"}"#;
            write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", response.len(), response).unwrap();
            Some(body)
        });
        (api_url, handle)
    }

    fn profile(api_url: String) -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            kernel_id: None,
            kernel_credential: None,
            kernel_public_key_thumbprint: None,
            api_url,
            account_id: "account-fixture".into(),
            user_id: "user-fixture".into(),
            realm_id: "realm-fixture".into(),
            client_id: Some("paired-client".into()),
            machine_id: Some("machine-fixture".into()),
            cloud_session_token: Some("synthetic-session".into()),
            machine_credential: Some("synthetic-machine-credential".into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn account_client_token_uses_only_the_session_and_preserves_profile_and_options() {
        let (api_url, server) = token_server();
        let profile = profile(api_url);
        let original = profile.clone();
        let issued = issue_cloud_account_client_runtime_token(
            &profile,
            "paired-client",
            CloudRuntimeTokenRequestOptions {
                ttl_ms: Some(42_000),
                allowed_actions: Some(vec!["packet.route".into()]),
                allowed_targets: Some(vec!["kernel-fixture".into()]),
                client_id: Some("paired-client".into()),
                session_id: Some("room-fixture".into()),
                public_key_thumbprint: Some("synthetic-thumbprint".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let body = server.join().unwrap().unwrap();
        assert_eq!(issued.token, "synthetic-client-token");
        assert_eq!(issued.expires_at, "2030-01-01T00:00:00Z");
        assert_eq!(body["sessionToken"], "synthetic-session");
        assert!(body.get("machineCredential").is_none());
        assert_eq!(body["subject"], "paired-client");
        assert_eq!(body["subjectKind"], "client");
        assert_eq!(body["accountId"], "account-fixture");
        assert_eq!(body["userId"], "user-fixture");
        assert_eq!(body["realmId"], "realm-fixture");
        assert_eq!(body["ttlMs"], 42_000);
        assert_eq!(body["allowedActions"], json!(["packet.route"]));
        assert_eq!(body["allowedTargets"], json!(["kernel-fixture"]));
        assert_eq!(body["clientId"], "paired-client");
        assert_eq!(body["sessionId"], "room-fixture");
        assert_eq!(body["publicKeyThumbprint"], "synthetic-thumbprint");
        assert_eq!(profile, original);
    }

    #[tokio::test]
    async fn account_client_token_rejects_missing_or_blank_session_before_http() {
        for session in [None, Some(""), Some("   ")] {
            let (api_url, server) = token_server();
            let mut profile = profile(api_url);
            profile.cloud_session_token = session.map(str::to_string);
            let original = profile.clone();
            let result = issue_cloud_account_client_runtime_token(
                &profile,
                "paired-client",
                CloudRuntimeTokenRequestOptions::default(),
            )
            .await;
            let request = server.join().unwrap();
            let error = result.expect_err("missing session must not fall back to machine auth");
            assert!(error
                .to_string()
                .contains("requires an authenticated Cloud session"));
            assert!(
                request.is_none(),
                "missing session must not make an HTTP request"
            );
            assert_eq!(profile, original);
        }
    }

    #[tokio::test]
    async fn shared_slice_discovery_still_prefers_machine_auth_with_exact_metadata_scope() {
        let (api_url, server) = token_server();
        let profile = profile(api_url);
        super::super::issue_cloud_slice_discovery_token(&profile, "owner-kernel", "worker-kernel")
            .await
            .unwrap();
        let body = server.join().unwrap().unwrap();
        assert_eq!(body["machineCredential"], "synthetic-machine-credential");
        assert!(body.get("sessionToken").is_none());
        assert_eq!(body["machineId"], "machine-fixture");
        assert_eq!(
            body["subject"],
            "slice-discovery:owner-kernel:worker-kernel"
        );
        assert_eq!(body["subjectKind"], "client");
        assert_eq!(body["allowedActions"], json!(["client.metadata.read"]));
        assert_eq!(body["allowUnpairedClientSubject"], true);
    }

    #[tokio::test]
    async fn shared_relay_inventory_still_prefers_machine_auth_with_exact_metadata_scope() {
        let (api_url, server) = token_server();
        let profile = profile(api_url);
        super::super::issue_cloud_relay_inventory_discovery_token(&profile, "owner-kernel", None)
            .await
            .unwrap();
        let body = server.join().unwrap().unwrap();
        assert_eq!(body["machineCredential"], "synthetic-machine-credential");
        assert!(body.get("sessionToken").is_none());
        assert_eq!(body["machineId"], "machine-fixture");
        assert_eq!(body["subject"], "relay-inventory:owner-kernel");
        assert_eq!(body["subjectKind"], "client");
        assert_eq!(body["allowedActions"], json!(["client.metadata.read"]));
        assert_eq!(body["allowUnpairedClientSubject"], true);
    }
}
