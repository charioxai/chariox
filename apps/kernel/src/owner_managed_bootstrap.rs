//! MP-07 / MP-08 / MP-11: per-user SSH bootstrap; independent of managed VM admission.
use crate::runtime::cloud_api_client::{CloudDevicePollResponse, CloudDeviceStartResponse};
use crate::{DaemonConfig, DaemonError};
use futures_util::{SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::io::Read;
use std::io::Write;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};
use zeroize::{Zeroize, Zeroizing};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnrollmentInput {
    ticket: String,
    api_url: String,
    #[serde(default)]
    user_id: Option<String>,
}
fn error() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "owner-managed bootstrap",
        message: "owner-managed enrollment or bound relay readiness failed".into(),
    }
}
pub(crate) fn admit_api_url(value: &str) -> Result<String, DaemonError> {
    let url = url::Url::parse(value).map_err(|_| error())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
    {
        return Err(error());
    }
    Ok(value.trim_end_matches('/').into())
}
fn public_identity(config: &DaemonConfig) -> Result<Value, DaemonError> {
    let profile = config.cloud_relay.as_ref().ok_or_else(error)?;
    let thumbprint =
        crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key);
    if profile
        .kernel_credential
        .as_deref()
        .unwrap_or("")
        .is_empty()
        || profile.kernel_id.as_deref() != Some(config.daemon_id.as_str())
        || profile.machine_id.as_deref() != Some(config.host_machine_id.as_str())
        || profile.kernel_public_key_thumbprint.as_deref() != Some(thumbprint.as_str())
    {
        return Err(error());
    }
    Ok(
        json!({"kernelId":config.daemon_id,"machineId":config.host_machine_id,"userId":profile.user_id,"publicKeyThumbprint":thumbprint}),
    )
}
async fn enroll(
    config: &mut DaemonConfig,
    mut input: EnrollmentInput,
    require_owner: bool,
) -> Result<Value, DaemonError> {
    let api_url = admit_api_url(&input.api_url)?;
    let ticket = Zeroizing::new(std::mem::take(&mut input.ticket));
    if ticket.is_empty()
        || ticket.len() > 4096
        || (require_owner && input.user_id.as_deref().unwrap_or("").is_empty())
    {
        return Err(error());
    }
    // Re-running never re-enrolls or replaces an existing kernel's owner.
    if let Some(profile) = &config.cloud_relay {
        // A browser code has no expected-owner hint. Never silently accept it for another existing owner.
        if input.user_id.is_none() {
            return Err(error());
        }
        if input
            .user_id
            .as_ref()
            .is_some_and(|id| profile.user_id != *id)
            || profile.api_url != api_url
        {
            return Err(error());
        }
        let mut identity = public_identity(config)?;
        identity["ticketConsumed"] = json!(false);
        return Ok(identity);
    }
    let response: CloudDevicePollResponse = enrollment_exchange(
        api_url.clone(),
        "/auth/device/poll",
        200,
        ticket_enrollment_body(config, ticket.as_str()),
    )
    .await
    .map_err(|_| error())?;
    let profile =
        crate::runtime::cloud_relay_login_executor::enrolled_profile(config, api_url, response)
            .map_err(|_| error())?;
    if input
        .user_id
        .as_ref()
        .is_some_and(|id| profile.user_id != *id)
    {
        return Err(error());
    }
    // Hosted registration must use TLS; loopback mock relay is allowed for the drill.
    admit_relay_url(&profile.relay_url)?;
    config.persist_cloud_relay_profile(Some(profile))?;
    let mut identity = public_identity(config)?;
    identity["ticketConsumed"] = json!(true);
    Ok(identity)
}
fn ticket_enrollment_body(config: &DaemonConfig, ticket: &str) -> Value {
    let mut body = json!({"ticket":ticket,"kernelId":config.daemon_id,"machineId":config.host_machine_id,
        "publicKeyThumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)});
    if let Some(alias) = &config.daemon_alias {
        body["kernelAlias"] = json!(alias);
    }
    body
}
// Enrollment must use the endpoint's expected status without redirects or unbounded decoding.
async fn enrollment_exchange<T: DeserializeOwned + Send + 'static>(
    api_url: String,
    path: &'static str,
    expected_status: u16,
    body: Value,
) -> Result<T, DaemonError> {
    tokio::task::spawn_blocking(move || {
        let response = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .post(&format!("{api_url}{path}"))
            .set("content-type", "application/json")
            .send_string(&body.to_string())
            .map_err(|_| error())?;
        if response.status() != expected_status {
            return Err(error());
        }
        let mut bytes = Zeroizing::new(Vec::new());
        response
            .into_reader()
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| error())?;
        if bytes.len() > 65536 {
            return Err(error());
        }
        serde_json::from_slice(&bytes).map_err(|_| error())
    })
    .await
    .map_err(|_| error())?
}

/// Read the one-time ticket before any process logger or normal daemon starts.
pub async fn enroll_from_stdin(require_owner: bool) -> Result<(), DaemonError> {
    let mut bytes = Zeroizing::new(Vec::new());
    std::io::stdin()
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| error())?;
    if bytes.len() > 8192 {
        return Err(error());
    }
    let input: EnrollmentInput = serde_json::from_slice(&bytes).map_err(|_| error())?;
    bytes.zeroize();
    require_explicit_root()?;
    let mut config = DaemonConfig::load_from_env();
    println!("{}", enroll(&mut config, input, require_owner).await?);
    Ok(())
}
// MP-08 / MP-11: setup device flow stores only the same key-bound kernel profile.
// Verification URL/code are public; device code and credential never reach stdout.
pub async fn enroll_device_from_stdin() -> Result<(), DaemonError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Input {
        api_url: String,
        user_id: Option<String>,
    }
    let mut bytes = Zeroizing::new(Vec::new());
    std::io::stdin()
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| error())?;
    if bytes.len() > 8192 {
        return Err(error());
    }
    let input: Input = serde_json::from_slice(&bytes).map_err(|_| error())?;
    bytes.zeroize();
    require_explicit_root()?;
    let api_url = admit_api_url(&input.api_url)?;
    let mut config = DaemonConfig::load_from_env();
    if let Some(profile) = &config.cloud_relay {
        if profile.api_url != api_url
            || input
                .user_id
                .as_ref()
                .is_some_and(|id| profile.user_id != *id)
        {
            return Err(error());
        }
        println!("{}", public_identity(&config)?);
        return Ok(());
    }
    let request = serde_json::from_value(json!({"api_url":api_url})).map_err(|_| error())?;
    let started: CloudDeviceStartResponse = enrollment_exchange(
        api_url.clone(),
        "/auth/device/start",
        201,
        crate::runtime::cloud_relay_login_executor::kernel_device_enrollment_body(
            &config, &request,
        ),
    )
    .await?;
    let code = Zeroizing::new(started.device_code);
    if code.is_empty() || code.len() > 4096 {
        return Err(error());
    }
    let verification = url::Url::parse(&started.verification_url).map_err(|_| error())?;
    let api = url::Url::parse(&api_url).map_err(|_| error())?;
    if verification.origin() != api.origin()
        || !verification.username().is_empty()
        || verification.password().is_some()
    {
        return Err(error());
    }
    let expiry = chrono::DateTime::parse_from_rfc3339(&started.expires_at)
        .map_err(|_| error())?
        .timestamp();
    let duration = (expiry - chrono::Utc::now().timestamp()).clamp(0, 600) as u64;
    if duration == 0 {
        return Err(error());
    }
    println!(
        "{}",
        json!({"verificationUrl":started.verification_url,"userCode":started.user_code})
    );
    std::io::stdout().flush().map_err(|_| error())?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(duration);
    let mut interval = started.interval_seconds.clamp(1, 10);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        let response: CloudDevicePollResponse = enrollment_exchange(
            api_url.clone(),
            "/auth/device/poll",
            200,
            json!({"deviceCode":code.as_str()}),
        )
        .await?;
        match response.status.as_str() {
            "authorization_pending" => {
                interval = response.interval_seconds.unwrap_or(interval).clamp(1, 10);
            }
            "approved" => {
                let profile = crate::runtime::cloud_relay_login_executor::enrolled_profile(
                    &config, api_url, response,
                )
                .map_err(|_| error())?;
                if input
                    .user_id
                    .as_ref()
                    .is_some_and(|id| profile.user_id != *id)
                {
                    return Err(error());
                }
                admit_relay_url(&profile.relay_url)?;
                config.persist_cloud_relay_profile(Some(profile))?;
                println!("{}", public_identity(&config)?);
                return Ok(());
            }
            _ => return Err(error()),
        }
    }
    Err(error())
}
fn admit_relay_url(value: &str) -> Result<(), DaemonError> {
    let relay = url::Url::parse(value).map_err(|_| error())?;
    if !relay.username().is_empty()
        || relay.password().is_some()
        || !(relay.scheme() == "wss"
            || (relay.scheme() == "ws"
                && matches!(relay.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
    {
        return Err(error());
    }
    Ok(())
}
fn require_explicit_root() -> Result<(), DaemonError> {
    let root = std::env::var("CHARIOX_HOME").map_err(|_| error())?;
    if !std::path::Path::new(&root).is_absolute()
        || root == "/"
        || crate::managed_bootstrap::confirmed_managed_kernel_registration_from_env()?.is_some()
    {
        return Err(error());
    }
    Ok(())
}
pub async fn wait_ready() -> Result<(), DaemonError> {
    require_explicit_root()?;
    let config = DaemonConfig::load_from_env();
    let mut identity = public_identity(&config)?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(90);
    loop {
        if tokio::time::timeout(std::time::Duration::from_secs(3), ready_once(&config))
            .await
            .is_ok_and(|r| r.is_ok())
        {
            identity["connected"] = json!(true);
            println!("{identity}");
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(error());
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}
async fn ready_once(config: &DaemonConfig) -> Result<(), DaemonError> {
    let path = DaemonConfig::default_kernel_local_auth_token_path(config.kernel_websocket_port);
    let token = Zeroizing::new(std::fs::read_to_string(path).map_err(|_| error())?);
    let mut request = config
        .kernel_websocket_url()
        .into_client_request()
        .map_err(|_| error())?;
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", token.trim())).map_err(|_| error())?,
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|_| error())?;
    // MP-07 / MP-08: each observation must bypass prior command receipts.
    let request_id = format!("byom-ready-{:016x}", rand::random::<u64>());
    ws.send(Message::Text(
        json!({"type":"request","request_id":request_id,"request":{"RelayStatus":null}})
            .to_string()
            .into(),
    ))
    .await
    .map_err(|_| error())?;
    while let Some(frame) = ws.next().await {
        let frame = frame.map_err(|_| error())?;
        if let Message::Text(text) = frame {
            let value: Value = serde_json::from_str(&text).map_err(|_| error())?;
            if value["request_id"] != request_id {
                continue;
            }
            let status = &value["response"]["RelayStatus"]["status"];
            if status["connected"] != true
                || status["daemon_id"] != config.daemon_id
                || status["machine_id"] != config.host_machine_id
            {
                return Err(error());
            }
            ws.close(None).await.map_err(|_| error())?;
            return Ok(());
        }
    }
    Err(error())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byom_mp11_bootstrap_rejects_credential_urls_and_cleartext_remote_cloud() {
        for url in [
            "http://cloud.example",
            "https://user:password@cloud.example",
            "https://cloud.example?ticket=secret",
            "https://cloud.example#secret",
        ] {
            assert!(admit_api_url(url).is_err());
        }
        assert!(admit_api_url("https://cloud.example").is_ok());
        assert!(admit_api_url("http://127.0.0.1:1234").is_ok());
    }
}

#[cfg(test)]
mod enrollment_transport_tests {
    use super::*;
    fn response_fixture(status: u16, body: String) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            // Consume the complete request without printing enrollment inputs.
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                use std::io::BufRead;
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        });
        (api_url, fixture)
    }
    #[tokio::test]
    async fn byom_mp11_device_start_accepts_created() {
        let (api_url, fixture) = response_fixture(
            201,
            json!({"deviceCode":"synthetic-device-code","userCode":"PUBLIC","verificationUrl":"http://127.0.0.1/approve","expiresAt":"2099-01-01T00:00:00Z","intervalSeconds":1}).to_string(),
        );
        let result = enrollment_exchange::<CloudDeviceStartResponse>(
            api_url,
            "/auth/device/start",
            201,
            json!({"enrollmentKind":"KERNEL"}),
        )
        .await;
        fixture.join().unwrap();
        assert!(
            result.is_ok(),
            "Cloud device start creates a login with HTTP 201"
        );
    }
    #[tokio::test]
    async fn byom_mp11_enrollment_transport_keeps_bounded_json() {
        for length in [65536, 65537] {
            let (api_url, fixture) =
                response_fixture(200, format!("\"{}\"", "a".repeat(length - 2)));
            let result =
                enrollment_exchange::<Value>(api_url, "/auth/device/poll", 200, json!({})).await;
            fixture.join().unwrap();
            assert_eq!(result.is_ok(), length == 65536);
        }
    }
    #[tokio::test]
    async fn byom_mp11_enrollment_uses_endpoint_success_status() {
        for (path, expected) in [("/auth/device/start", 201), ("/auth/device/poll", 200)] {
            for status in [200, 201, 202, 204, 400] {
                let (api_url, fixture) = response_fixture(status, "{}".into());
                let result = enrollment_exchange::<Value>(api_url, path, expected, json!({})).await;
                fixture.join().unwrap();
                assert_eq!(result.is_ok(), status == expected, "{path}: HTTP {status}");
            }
        }
    }
    #[tokio::test]
    async fn byom_mp11_enrollment_never_follows_redirects() {
        for (path, expected) in [("/auth/device/start", 201), ("/auth/device/poll", 200)] {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let fixture = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 8192];
                let _ = stream.read(&mut request).unwrap();
                write!(stream,"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{address}/must-not-receive-ticket\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                drop(stream);
                listener.set_nonblocking(true).unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
                while std::time::Instant::now() < deadline {
                    if listener.accept().is_ok() {
                        return true;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                false
            });
            assert!(enrollment_exchange::<Value>(
                format!("http://{address}"),
                path,
                expected,
                json!({"ticket":"fixture-single-use"})
            )
            .await
            .is_err());
            assert!(!fixture.join().unwrap());
        }
    }
}

#[cfg(test)]
mod self_setup_tests {
    use super::*;
    #[tokio::test]
    async fn byom_mp11_ssh_ticket_cannot_omit_expected_owner() {
        let mut config = DaemonConfig::for_tests();
        assert!(enroll(
            &mut config,
            EnrollmentInput {
                ticket: "synthetic-one-use".into(),
                api_url: "http://127.0.0.1:1".into(),
                user_id: None
            },
            true
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn byom_mp11_browser_code_does_not_silently_accept_existing_kernel_owner() {
        let mut config = DaemonConfig::for_tests();
        let pin =
            crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key);
        let response = serde_json::from_value(json!({ "status":"approved", "kernelCredential":"synthetic-kernel-credential", "profile": { "email":"fixture@example.test", "accountId":"owner", "userId":"owner", "accountSlug":"owner", "realmId":"owner", "relayUrl":"ws://127.0.0.1:1234", "issuerId":"fixture", "kernelId":config.daemon_id, "machineId":config.host_machine_id, "publicKeyThumbprint":pin } })).unwrap();
        config.cloud_relay = Some(
            crate::runtime::cloud_relay_login_executor::enrolled_profile(
                &config,
                "http://127.0.0.1:1".into(),
                response,
            )
            .unwrap(),
        );
        assert!(enroll(
            &mut config,
            EnrollmentInput {
                ticket: "synthetic-browser-code".into(),
                api_url: "http://127.0.0.1:1".into(),
                user_id: None
            },
            false
        )
        .await
        .is_err());
        let identity = enroll(
            &mut config,
            EnrollmentInput {
                ticket: "synthetic-unused-code".into(),
                api_url: "http://127.0.0.1:1".into(),
                user_id: Some("owner".into()),
            },
            true,
        )
        .await
        .unwrap();
        assert_eq!(identity["ticketConsumed"], false);
        assert_eq!(identity["userId"], "owner");
    }
    #[test]
    fn byom_mp11_self_setup_rejects_cleartext_and_credential_bearing_relays() {
        for relay in [
            "ws://remote.example",
            "wss://user:password@remote.example",
            "https://remote.example",
        ] {
            assert!(admit_relay_url(relay).is_err());
        }
        assert!(admit_relay_url("wss://relay.example").is_ok());
    }
}

#[cfg(test)]
mod review_contract_tests {
    use super::*;
    #[test]
    fn byom_mp11_ticket_body_matches_cloud302_strict_schema() {
        let config = DaemonConfig::for_tests();
        let body = ticket_enrollment_body(&config, "synthetic-single-use-code");
        let allowed = [
            "ticket",
            "machineId",
            "kernelId",
            "publicKeyThumbprint",
            "kernelAlias",
        ];
        assert!(body
            .as_object()
            .unwrap()
            .keys()
            .all(|field| allowed.contains(&field.as_str())));
        assert!(body.get("kernelAlias").is_none());
    }
    #[tokio::test]
    async fn byom_mp11_distinct_account_and_user_enroll_without_losing_consumed_credential() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let mut config = DaemonConfig::for_tests();
        let scratch =
            std::env::temp_dir().join(format!("chariox-byom-review-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&scratch).unwrap();
        config.user_config_path = scratch.join("config.toml");
        let response = json!({"status":"approved","kernelCredential":"synthetic-credential","profile":{"email":"fixture@example.test","accountId":"account-owner","userId":"user-owner","accountSlug":"owner","realmId":"realm-owner","relayUrl":"ws://127.0.0.1:1234","issuerId":"fixture","kernelId":config.daemon_id,"machineId":config.host_machine_id,"publicKeyThumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)}}).to_string();
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0u8; 8192];
            let _ = stream.read(&mut bytes).unwrap();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
        });
        let result = enroll(
            &mut config,
            EnrollmentInput {
                ticket: "synthetic-one-use-code".into(),
                api_url,
                user_id: Some("user-owner".into()),
            },
            true,
        )
        .await;
        fixture.join().unwrap();
        std::fs::remove_dir_all(&scratch).unwrap();
        assert!(
            result.is_ok(),
            "Cloud account ID is distinct from its owner user ID"
        );
    }
}
