use super::*;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use std::io::{BufRead, BufReader};

#[test]
fn web_bot_auth_signs_real_plain_and_query_credential_http_requests() {
    let now = crate::session::unix_epoch_ms() / 1000;
    let signer = crate::web_bot_auth::WebBotAuthSigner::generate(
        "https://operator.example",
        now - 1,
        now + 3600,
    )
    .unwrap();
    for with_credential in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let url = format!("{origin}/api?existing=1");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut input = BufReader::new(stream);
            let mut first = String::new();
            input.read_line(&mut first).unwrap();
            let mut headers = BTreeMap::new();
            loop {
                let mut line = String::new();
                input.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.split_once(':').unwrap();
                headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
            }
            let mut body = vec![0; headers.get("content-length").unwrap().parse().unwrap()];
            input.read_exact(&mut body).unwrap();
            input
                .get_mut()
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                .unwrap();
            (first, headers, body)
        });
        let prepared = if with_credential {
            let store = Arc::new(MemoryVaultStore::default());
            store
                .set_secret("wba", "api", "fixture-query-value")
                .unwrap();
            let service = RuntimeSecretService::with_vault_store(
                vec![UserCredentialConfig {
                    id: "api".into(),
                    description: None,
                    source: UserCredentialSourceConfig::Vault { key: "api".into() },
                    allowed_hosts: vec!["127.0.0.1".into()],
                    allowed_uses: vec![UserCredentialUse::Http],
                    injection: UserCredentialInjectionConfig::Query {
                        name: "api_key".into(),
                    },
                    metadata: None,
                }],
                "wba",
                store,
            );
            service
                .prepare_http_request_with_credential(CredentialHttpRequest {
                    credential_id: "api".into(),
                    method: "POST".into(),
                    url,
                    headers: Default::default(),
                    body_text: Some("request-body".into()),
                    body_json: None,
                    timeout_ms: 5000,
                    max_response_bytes: 1024,
                })
                .unwrap()
        } else {
            RuntimeSecretService::prepare_plain_http_request(
                crate::transport::runtime_tools::HttpRequestArgs {
                    method: "POST".into(),
                    url,
                    headers: Default::default(),
                    body_text: Some("request-body".into()),
                    body_json: None,
                    timeout_ms: 5000,
                    max_response_bytes: 1024,
                },
            )
            .unwrap()
        };
        assert_eq!(
            prepared
                .with_web_bot_auth(Some(&signer))
                .unwrap()
                .execute()
                .unwrap()
                .status,
            200
        );
        let (first, headers, body) = server.join().unwrap();
        let request_path = first.split_whitespace().nth(1).unwrap();
        assert_eq!(
            request_path.contains("api_key=fixture-query-value"),
            with_credential
        );
        let target = format!("{origin}{request_path}");
        let params = headers["signature-input"].strip_prefix("sig1=").unwrap();
        let agent = headers["signature-agent"].strip_prefix("sig1=").unwrap();
        let base = format!("\"@method\": POST\n\"@target-uri\": {target}\n\"@authority\": {}\n\"signature-agent\";key=\"sig1\": {agent}\n\"content-digest\": {}\n\"@signature-params\": {params}", headers["host"], headers["content-digest"]);
        assert_eq!(
            headers["content-digest"],
            format!("sha-256=:{}:", STANDARD.encode(Sha256::digest(&body)))
        );
        let key: [u8; 32] = URL_SAFE_NO_PAD
            .decode(&signer.public_jwk().x)
            .unwrap()
            .try_into()
            .unwrap();
        let key = VerifyingKey::from_bytes(&key).unwrap();
        let signature = Signature::from_slice(
            &STANDARD
                .decode(
                    headers["signature"]
                        .strip_prefix("sig1=:")
                        .unwrap()
                        .strip_suffix(':')
                        .unwrap(),
                )
                .unwrap(),
        )
        .unwrap();
        assert!(key.verify(base.as_bytes(), &signature).is_ok());
        assert!(key
            .verify(
                base.replace("request-body", "tampered")
                    .replace("existing=1", "existing=2")
                    .as_bytes(),
                &signature
            )
            .is_err());
    }
}
