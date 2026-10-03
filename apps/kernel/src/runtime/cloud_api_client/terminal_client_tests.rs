use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Request {
    path: String,
    authorization: Option<String>,
    body: serde_json::Value,
}

fn server(paths: &[&str]) -> (String, thread::JoinHandle<Vec<Request>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let paths: Vec<String> = paths.iter().map(|path| path.to_string()).collect();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for expected in paths {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "expected Cloud request was not made"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let (header_end, content_length) = loop {
                let mut chunk = [0; 4096];
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                assert!(bytes.len() < 65536);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let header = std::str::from_utf8(&bytes[..end]).unwrap();
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
            while bytes.len() < header_end + content_length {
                let mut chunk = [0; 4096];
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                assert!(bytes.len() < 65536);
            }
            let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
            let path = header
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string();
            assert_eq!(path, expected);
            let authorization = header.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("authorization")
                    .then(|| value.trim().to_string())
            });
            requests.push(Request {
                path,
                authorization,
                body: serde_json::from_slice(&bytes[header_end..header_end + content_length])
                    .unwrap(),
            });
            let payload = r#"{"token":"synthetic-token","expiresAt":"2026-10-01T00:00:00Z"}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
        }
        requests
    });
    (url, handle)
}

fn profile(url: String) -> PersistedCloudRelayProfile {
    PersistedCloudRelayProfile {
        api_url: url,
        account_id: "account-fixture".into(),
        user_id: "user-fixture".into(),
        realm_id: "realm-fixture".into(),
        machine_id: Some("machine-fixture".into()),
        machine_credential: Some("synthetic-machine-credential".into()),
        ..Default::default()
    }
}

#[test]
fn subject_matches_cloud_utf8_contract_and_refresh_is_idempotent() {
    let subject = machine_client_subject("mächine", "客戶").unwrap();
    assert_eq!(subject, "machine-client:89eb5ca66d1d1839b75ed39dea4ce10bc2a2ca527b3c9ae99a08169d3b0cdb96:730bf0c798da3e1d5cf241da37ab7f6f3ba8323a0b2a00d403549edb0f192339");
    assert_eq!(subject.len(), 144);
    assert_eq!(
        machine_client_subject("mächine", &subject).unwrap(),
        subject
    );
    assert!(machine_client_subject("another-machine", &subject).is_err());
    assert!(machine_client_subject("mächine", "machine-client:malformed").is_err());
    let bad_suffix = format!("{}{}", &subject[..80], subject[80..].to_uppercase());
    assert!(machine_client_subject("mächine", &bad_suffix).is_err());
    assert!(machine_client_subject("", "client").is_err());
    assert!(machine_client_subject("machine", " ").is_err());
}

#[tokio::test]
async fn machine_terminal_create_and_key_bound_join_use_only_scoped_runtime_tokens() {
    let (url, fixture) = server(&["/relay/token", "/relay/token"]);
    let profile = profile(url);
    let (client_id, _) = issue_cloud_terminal_client_token(
        &profile,
        "client-fixture",
        "kernel-fixture",
        CloudTerminalClientOptions {
            pair_account_client: true,
            target_alias: Some("friendly-name".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (joined_id, _) = issue_cloud_terminal_client_token(
        &profile,
        &client_id,
        "kernel-fixture",
        CloudTerminalClientOptions {
            public_key_thumbprint: Some("a".repeat(64)),
            ttl_ms: Some(crate::runtime::cloud_relay_control::CLOUD_RELAY_CLIENT_TOKEN_TTL_MS),
            target_alias: Some("friendly-name".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(joined_id, client_id);
    assert_eq!(profile.client_id, None);
    let requests = fixture.join().unwrap();
    for request in &requests {
        assert_eq!(request.path, "/relay/token");
        assert_eq!(request.authorization, None);
        assert_eq!(
            request.body["machineCredential"],
            "synthetic-machine-credential"
        );
        assert!(request.body.get("sessionToken").is_none());
        assert_eq!(request.body["machineId"], "machine-fixture");
        assert_eq!(request.body["subject"], client_id);
        assert_eq!(request.body["clientId"], client_id);
        assert_eq!(request.body["subjectKind"], "client");
        assert_eq!(request.body["allowUnpairedClientSubject"], true);
        assert_eq!(
            request.body["allowedTargets"],
            serde_json::json!(["kernel-fixture"])
        );
    }
    assert_eq!(requests[1].body["publicKeyThumbprint"], "a".repeat(64));
    assert_eq!(requests[1].body["ttlMs"], 30 * 60_000);
}

#[tokio::test]
async fn account_terminal_pairs_with_bearer_and_uses_session_instead_of_machine_authority() {
    let (url, fixture) = server(&["/pairing-tokens", "/clients/pair", "/relay/token"]);
    let mut profile = profile(url);
    profile.cloud_session_token = Some("synthetic-account-session".into());
    let (client_id, _) = issue_cloud_terminal_client_token(
        &profile,
        "account-client",
        "kernel-fixture",
        CloudTerminalClientOptions {
            pair_account_client: true,
            client_alias: Some("CLI terminal".into()),
            target_alias: Some("friendly-name".into()),
            session_id: Some("shared-session".into()),
            public_key_thumbprint: Some("b".repeat(64)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(client_id, "account-client");
    assert_eq!(
        profile.machine_credential.as_deref(),
        Some("synthetic-machine-credential")
    );
    assert_eq!(profile.client_id, None);
    let requests = fixture.join().unwrap();
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer synthetic-account-session")
    );
    assert_eq!(requests[1].body["clientId"], "account-client");
    assert_eq!(requests[1].body["alias"], "CLI terminal");
    let body = &requests[2].body;
    assert_eq!(body["sessionToken"], "synthetic-account-session");
    assert!(body.get("machineCredential").is_none());
    assert!(body.get("allowUnpairedClientSubject").is_none());
    assert_eq!(
        body["allowedTargets"],
        serde_json::json!(["kernel-fixture", "friendly-name"])
    );
    assert_eq!(body["sessionId"], "shared-session");
    assert_eq!(body["publicKeyThumbprint"], "b".repeat(64));
}

#[tokio::test]
async fn foreign_machine_client_is_rejected_without_network_or_profile_mutation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let profile = profile(format!("http://{}", listener.local_addr().unwrap()));
    let foreign = machine_client_subject("foreign-machine", "client").unwrap();
    assert!(issue_cloud_terminal_client_token(
        &profile,
        &foreign,
        "kernel-fixture",
        Default::default()
    )
    .await
    .is_err());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(profile.client_id, None);
}

#[tokio::test]
async fn account_auto_pair_omits_absent_alias_from_redemption() {
    let (url, fixture) = server(&["/pairing-tokens", "/clients/pair", "/relay/token"]);
    let mut profile = profile(url);
    profile.cloud_session_token = Some("synthetic-account-session".into());
    issue_cloud_terminal_client_token(
        &profile,
        "auto-client",
        "kernel-fixture",
        CloudTerminalClientOptions {
            pair_account_client: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let requests = fixture.join().unwrap();
    assert!(!requests[1].body.as_object().unwrap().contains_key("alias"));
}
