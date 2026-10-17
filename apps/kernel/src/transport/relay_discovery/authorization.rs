//! Authorization for read-only relay metadata queries.

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::issue_cloud_relay_inventory_discovery_token;

pub(crate) async fn metadata_token(config: &DaemonConfig) -> Result<String, DaemonError> {
    // MP-08 / MP-10 / MP-11: only the hosted relay of the Cloud profile accepts
    // the Cloud metadata grant; a self-hosted relay keeps its own token (#956).
    let hosted = config
        .relay_url
        .as_deref()
        .is_some_and(|url| config.relay_url_uses_cloud_profile(url));
    if hosted
        && config.cloud_relay.is_some()
        && config
            .relay_token
            .as_deref()
            .is_none_or(|token| uses_own_cloud_runtime_token(config, token))
    {
        return cloud_metadata_token(config).await;
    }
    config
        .relay_token
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "relay_metadata_query",
            message: "relay_token is not configured".to_string(),
        })
}

pub(crate) async fn cloud_metadata_token(config: &DaemonConfig) -> Result<String, DaemonError> {
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "relay_metadata_query",
            message: "Cloud relay profile is not configured".to_string(),
        })?;
    // Use the same scoped grant for projected inventory and fresh lookups.
    // Issuance failure must not fall back to the machine runtime grant.
    Ok(issue_cloud_relay_inventory_discovery_token(
        profile,
        &config.daemon_id,
        visible_targets(profile)
            .await?
            .map(|ids| ids.into_iter().collect()),
    )
    .await?
    .token)
}

// Kernel discovery must use the same owner-visible target set for every query.
// Keep the legacy machine-credential path for compatible self-hosted enrollments.
pub(crate) async fn visible_targets(
    profile: &crate::config::PersistedCloudRelayProfile,
) -> Result<Option<std::collections::BTreeSet<String>>, DaemonError> {
    if profile.kernel_credential.is_none() {
        return Ok(None);
    }
    let directory = crate::runtime::cloud_api_client::get_cloud_kernel_directory(profile).await?;
    cloud_directory_discovery_targets(&directory).map(Some)
}

fn cloud_directory_discovery_targets(
    directory: &serde_json::Value,
) -> Result<std::collections::BTreeSet<String>, DaemonError> {
    Ok(directory
        .get("targets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "read My kernels",
            message: "Cloud returned an invalid kernel directory".into(),
        })?
        .iter()
        // The owner directory retains revoked rows for account history, but
        // Cloud rejects discovery grants containing any revoked target. An
        // unlinked sibling must not prevent the remaining kernels renewing.
        .filter(|target| {
            target.get("status").and_then(serde_json::Value::as_str) != Some("REVOKED")
        })
        .filter_map(|target| {
            target
                .get("daemonId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

fn uses_own_cloud_runtime_token(config: &DaemonConfig, token: &str) -> bool {
    if !config
        .relay_url
        .as_deref()
        .is_some_and(|url| config.relay_url_uses_cloud_profile(url))
    {
        return false;
    }
    // Unverified, bounded claims choose the credential source only. Cloud
    // authenticates issuance; the relay verifies the resulting authorization.
    // Preserve supplied client/slice grants and tokens for other identities.
    crate::runtime::cloud_relay_control::relay_token_payload(token).is_some_and(|claims| {
        claims
            .get("subject_kind")
            .and_then(serde_json::Value::as_str)
            == Some("kernel")
            && claims.get("sub").and_then(serde_json::Value::as_str)
                == Some(config.daemon_id.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloud_directory_excludes_revoked_siblings_from_discovery_grants() {
        let targets = cloud_directory_discovery_targets(&serde_json::json!({
            "targets": [
                {"daemonId": "owner", "status": "ONLINE"},
                {"daemonId": "sibling", "status": "OFFLINE"},
                {"daemonId": "unlinked", "status": "REVOKED"},
                {"daemonId": "sibling", "status": "STALE"},
                {"status": "ONLINE"}
            ]
        }))
        .unwrap();
        assert_eq!(targets, ["owner".to_string(), "sibling".to_string()].into());
    }

    #[test]
    fn malformed_directory_fails_closed() {
        assert!(cloud_directory_discovery_targets(&serde_json::json!({})).is_err());
    }

    use base64::Engine as _;
    use std::io::{Read, Write};

    #[test]
    fn metadata_routing_preserves_provided_scopes_and_other_relays() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".into());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url: "https://cloud.example.test".into(),
            email: "user@example.test".into(),
            account_id: "account".into(),
            user_id: "owner".into(),
            account_slug: "acct".into(),
            realm_id: "realm".into(),
            relay_url: config.relay_url.clone().unwrap(),
            issuer_id: "issuer".into(),
            client_id: None,
            client_alias: None,
            machine_id: Some("machine".into()),
            machine_alias: None,
            machine_credential: Some("fixture-machine".into()),
            kernel_id: None,
            kernel_credential: None,
            kernel_public_key_thumbprint: None,
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        });
        let token = |kind: &str, subject: &str| {
            format!(
                "header.{}.signature",
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(serde_json::json!({"subject_kind":kind,"sub":subject}).to_string())
            )
        };
        assert!(uses_own_cloud_runtime_token(
            &config,
            &token("kernel", &config.daemon_id)
        ));
        assert!(!uses_own_cloud_runtime_token(
            &config,
            &token("client", "slice-discovery:owner:worker")
        ));
        assert!(!uses_own_cloud_runtime_token(
            &config,
            &token("client", "paired-terminal")
        ));
        assert!(!uses_own_cloud_runtime_token(
            &config,
            &token("kernel", "another-kernel")
        ));
        assert!(!uses_own_cloud_runtime_token(
            &config,
            "opaque-self-hosted-token"
        ));
        assert!(!uses_own_cloud_runtime_token(&config, &"x".repeat(16_385)));
        config.relay_url = Some("ws://127.0.0.1:1".into());
        assert!(!uses_own_cloud_runtime_token(
            &config,
            &token("kernel", &config.daemon_id)
        ));
        config.cloud_relay = None;
        assert!(!uses_own_cloud_runtime_token(
            &config,
            &token("kernel", &config.daemon_id)
        ));
    }

    #[tokio::test]
    async fn kernel_peer_metadata_uses_read_only_cloud_token() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let fixture = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => panic!("Cloud fixture accept: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0 && request.len() < 16_384);
                request.extend_from_slice(&buffer[..n]);
            }
            let body = r#"{"token":"read-only-metadata-token","expiresAt":"2099-01-01T00:00:00Z"}"#;
            stream.write_all(format!("HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",body.len(),body).as_bytes()).unwrap();
        });
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example.test".into());
        let payload = serde_json::json!({"subject_kind":"kernel","sub":config.daemon_id,"allowed_actions":["daemon.register","peer.request"]});
        let runtime_token = format!(
            "header.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
        );
        config.relay_token = Some(runtime_token.clone());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url: format!("http://{address}"),
            email: "user@example.test".into(),
            account_id: "account".into(),
            user_id: "owner".into(),
            account_slug: "acct".into(),
            realm_id: "realm".into(),
            relay_url: config.relay_url.clone().unwrap(),
            issuer_id: "issuer".into(),
            client_id: None,
            client_alias: None,
            machine_id: Some("machine".into()),
            machine_alias: None,
            machine_credential: Some("fixture-machine".into()),
            kernel_id: None,
            kernel_credential: None,
            kernel_public_key_thumbprint: None,
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        });
        assert_eq!(
            metadata_token(&config).await.unwrap(),
            "read-only-metadata-token"
        );
        assert_eq!(config.relay_token.as_deref(), Some(runtime_token.as_str()));
        fixture.join().unwrap();
    }
}
