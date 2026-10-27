use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

fn profile(api_url: String) -> PersistedCloudRelayProfile {
    serde_json::from_value(serde_json::json!({
        "api_url": api_url,
        "email": "fixture@example.invalid",
        "account_id": "fixture-account",
        "user_id": "fixture-user",
        "account_slug": "fixture",
        "realm_id": "fixture-realm",
        "relay_url": "wss://relay.example.invalid",
        "issuer_id": "fixture-issuer",
        "client_id": "prior-client",
        "machine_id": "prior-machine",
        "cloud_session_token": "synthetic-cloud-session",
        "machine_credential": "synthetic-machine-credential",
    }))
    .expect("synthetic Cloud profile")
}

fn response_fixture(status: u16) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind pairing fixture");
    let address = listener.local_addr().expect("pairing fixture address");
    let fixture = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept pairing request");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .expect("bound fixture reads");
        let mut request = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let read = stream.read(&mut chunk).expect("read pairing request");
            assert!(read > 0, "complete request must arrive before EOF");
            request.extend_from_slice(&chunk[..read]);
            assert!(request.len() <= 8192, "fixture request is bounded");
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(str::trim)
                            .map(str::to_string)
                    })
                    .expect("JSON request length")
                    .parse()
                    .expect("valid request length");
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let body = if status == 200 {
            r#"{"token":"synthetic-pairing-token"}"#
        } else {
            r#"{"error":{"code":"authorization_denied"}}"#
        };
        write!(
            stream,
            "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("reply to pairing admission");
        String::from_utf8(request).expect("fixture JSON request")
    });
    (format!("http://{address}"), fixture)
}

#[tokio::test]
async fn account_pairing_forwards_the_session_bearer_for_clients_and_machines() {
    for kind in ["client", "machine"] {
        let (api_url, fixture) = response_fixture(200);
        let linked = profile(api_url);
        let result = request_account_pairing_token(&linked, kind)
            .await
            .expect("authenticated admission");
        assert_eq!(result.token, "synthetic-pairing-token");
        let request = fixture.join().expect("pairing fixture");
        let (headers, body) = request.split_once("\r\n\r\n").expect("HTTP request");
        assert!(headers.starts_with("POST /pairing-tokens "));
        assert!(headers.lines().any(|line| {
            line.eq_ignore_ascii_case("authorization: Bearer synthetic-cloud-session")
        }));
        let body: serde_json::Value = serde_json::from_str(body).expect("pairing body");
        assert_eq!(body["accountId"], "fixture-account");
        assert_eq!(body["subjectKind"], kind);
        assert!(body.get("sessionToken").is_none());
        assert!(body.get("machineCredential").is_none());
    }
}

#[tokio::test]
async fn missing_account_session_never_contacts_cloud_or_changes_the_linked_profile() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind no-request fixture");
    listener.set_nonblocking(true).expect("nonblocking fixture");
    for token in [None, Some(""), Some(" "), Some("\n\t")] {
        for kind in ["client", "machine"] {
            let mut linked = profile(format!("http://{}", listener.local_addr().unwrap()));
            linked.cloud_session_token = token.map(str::to_string);
            let before = serde_json::to_value(&linked).unwrap();
            let error = request_account_pairing_token(&linked, kind)
                .await
                .expect_err("account-wide pairing requires a session");
            assert!(error.to_string().contains("cloud session required"));
            assert_eq!(serde_json::to_value(&linked).unwrap(), before);
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }
}

#[tokio::test]
async fn denied_account_pairing_admission_preserves_the_existing_identities() {
    for kind in ["client", "machine"] {
        let (api_url, fixture) = response_fixture(403);
        let linked = profile(api_url);
        let before = serde_json::to_value(&linked).unwrap();
        let error = request_account_pairing_token(&linked, kind)
            .await
            .expect_err("Cloud can deny account operate admission");
        assert!(error.to_string().contains("authorization_denied"));
        fixture.join().expect("denied admission fixture");
        assert_eq!(serde_json::to_value(&linked).unwrap(), before);
    }
}
