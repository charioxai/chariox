use crate::config::DaemonConfig;
use crate::error::DaemonError;

// MP-08 / MP-10 / MP-11: hosted metadata reads use the client-scoped grant
// (owner-visible targets); peer registration keeps the kernel grant. A
// self-hosted relay keeps its own token.
pub(crate) async fn relay_metadata_discovery_config(
    config: &DaemonConfig,
) -> Result<DaemonConfig, DaemonError> {
    let hosted = config
        .relay_url
        .as_deref()
        .is_some_and(|url| config.relay_url_uses_cloud_profile(url));
    scoped_metadata_config(config, hosted).await
}

// A read of one named target. A hosted slice worker never reports presence to
// Cloud, so the owner-visible grant above (Cloud-online targets only) cannot see
// it; its home kernel reads it with the exact-target slice discovery grant, as
// the agent binding does.
pub(crate) async fn relay_metadata_discovery_config_for(
    config: &DaemonConfig,
    target_ref: &str,
) -> Result<DaemonConfig, DaemonError> {
    let hosted = config
        .relay_url
        .as_deref()
        .is_some_and(|url| config.relay_url_uses_cloud_profile(url));
    match config.cloud_relay.as_ref() {
        Some(profile) if hosted && target_ref.starts_with("slice:") => {
            let grant =
                super::issue_cloud_slice_discovery_token(profile, &config.daemon_id, target_ref)
                    .await?;
            Ok(crate::transport::relay_discovery::with_metadata_token(
                config,
                grant.token,
            ))
        }
        _ => relay_metadata_discovery_config(config).await,
    }
}

// The returned config carries the issued grant and no Cloud profile, so relay
// discovery reads it as-is rather than issuing a second grant.
async fn scoped_metadata_config(
    config: &DaemonConfig,
    issue_cloud_grant: bool,
) -> Result<DaemonConfig, DaemonError> {
    let mut discovery = config.clone();
    if let Some(profile) = config.cloud_relay.as_ref().filter(|_| issue_cloud_grant) {
        let targets =
            crate::transport::relay_discovery::authorization::visible_targets(profile).await?;
        // MP-08 / MP-10 / MP-11: defer large inventories to the query's bounded
        // grants. A single API grant can authorize at most 64 targets.
        if targets.as_ref().is_some_and(|targets| targets.len() > 64) {
            return Ok(discovery);
        }
        discovery.relay_token = Some(
            crate::transport::relay_discovery::authorization::cloud_metadata_token_for_targets(
                config, targets,
            )
            .await?,
        );
    }
    discovery.cloud_relay = None;
    Ok(discovery)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A Cloud that answers one runtime-token request and returns what it was asked.
    fn cloud() -> (String, std::thread::JoinHandle<serde_json::Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            let body = loop {
                let mut chunk = [0; 4096];
                let read = stream.read(&mut chunk).unwrap();
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                    }
                }
            };
            let payload = r#"{"token":"slice-discovery-grant","expiresAt":"2026-10-01T00:00:00Z"}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
            body
        });
        (url, seen)
    }

    fn hosted_config(api_url: String) -> DaemonConfig {
        let mut config = DaemonConfig::for_tests();
        config.daemon_id = "home-kernel".into();
        config.relay_url = Some("wss://relay.example.test/ws".into());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url,
            account_id: "account-fixture".into(),
            realm_id: "realm-fixture".into(),
            relay_url: "wss://relay.example.test/ws".into(),
            machine_id: Some("machine-fixture".into()),
            kernel_id: Some("home-kernel".into()),
            kernel_credential: Some("synthetic-kernel-credential".into()),
            kernel_public_key_thumbprint: Some("e".repeat(64)),
            ..Default::default()
        });
        config
    }

    #[tokio::test]
    async fn a_hosted_slice_worker_is_read_with_its_own_discovery_grant() {
        let (url, cloud) = cloud();
        let discovery = relay_metadata_discovery_config_for(&hosted_config(url), "slice:m:1")
            .await
            .unwrap();
        assert_eq!(
            discovery.relay_token.as_deref(),
            Some("slice-discovery-grant")
        );
        assert!(discovery.cloud_relay.is_none());
        let asked = cloud.join().unwrap();
        assert_eq!(asked["subject"], "slice-discovery:home-kernel:slice:m:1");
        assert_eq!(asked["allowedTargets"], serde_json::json!(["slice:m:1"]));
    }
}
