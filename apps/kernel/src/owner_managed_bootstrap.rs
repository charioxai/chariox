//! MP-07 / MP-08 / MP-11: per-user SSH bootstrap; independent of managed VM admission.
use crate::runtime::cloud_api_client::CloudDevicePollResponse;
use crate::{DaemonConfig, DaemonError};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};
use zeroize::{Zeroize, Zeroizing};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnrollmentInput {
    ticket: String,
    api_url: String,
    user_id: String,
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
) -> Result<Value, DaemonError> {
    let api_url = admit_api_url(&input.api_url)?;
    let ticket = Zeroizing::new(std::mem::take(&mut input.ticket));
    if ticket.is_empty() || ticket.len() > 4096 || input.user_id.is_empty() {
        return Err(error());
    }
    // Re-running never re-enrolls or replaces an existing kernel's owner.
    if let Some(profile) = &config.cloud_relay {
        if profile.user_id != input.user_id || profile.api_url != api_url {
            return Err(error());
        }
        let mut identity = public_identity(config)?;
        identity["ticketConsumed"] = json!(false);
        return Ok(identity);
    }
    let response: CloudDevicePollResponse = redeem_ticket(
        api_url.clone(),
        "/auth/device/poll",
        ticket_enrollment_body(config, ticket.as_str()),
    )
    .await
    .map_err(|_| error())?;
    let profile =
        crate::runtime::cloud_relay_login_executor::enrolled_profile(config, api_url, response)
            .map_err(|_| error())?;
    if profile.user_id != input.user_id {
        return Err(error());
    }
    // Hosted registration must use TLS; loopback mock relay is allowed for the drill.
    let relay = url::Url::parse(&profile.relay_url).map_err(|_| error())?;
    if !(relay.scheme() == "wss"
        || (relay.scheme() == "ws"
            && matches!(relay.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
    {
        return Err(error());
    }
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
// A ticket must not follow redirects or decode an unbounded control-plane body.
async fn redeem_ticket(
    api_url: String,
    path: &'static str,
    body: Value,
) -> Result<CloudDevicePollResponse, DaemonError> {
    tokio::task::spawn_blocking(move || {
        let response = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .post(&format!("{api_url}{path}"))
            .set("content-type", "application/json")
            .send_string(&body.to_string())
            .map_err(|_| error())?;
        if response.status() != 200 {
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
pub async fn enroll_from_stdin() -> Result<(), DaemonError> {
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
    println!("{}", enroll(&mut config, input).await?);
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
    ws.send(Message::Text(
        json!({"type":"request","request_id":"byom-ready","request":{"RelayStatus":null}})
            .to_string()
            .into(),
    ))
    .await
    .map_err(|_| error())?;
    while let Some(frame) = ws.next().await {
        let frame = frame.map_err(|_| error())?;
        if let Message::Text(text) = frame {
            let value: Value = serde_json::from_str(&text).map_err(|_| error())?;
            if value["request_id"] != "byom-ready" {
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
mod ticket_transport_tests {
    use super::*;
    #[tokio::test]
    async fn byom_mp11_ticket_redemption_never_follows_redirects() {
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
        assert!(redeem_ticket(
            format!("http://{address}"),
            "/auth/device/poll",
            json!({"ticket":"fixture-single-use"})
        )
        .await
        .is_err());
        assert!(!fixture.join().unwrap());
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
                user_id: "user-owner".into(),
            },
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
