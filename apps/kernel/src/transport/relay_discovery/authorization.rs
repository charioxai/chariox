//! Authorization for read-only relay metadata queries.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::issue_cloud_relay_inventory_discovery_token;

/// A refresh reads the kernel directory and a discovery grant once per machine it lists; these
/// reads share one answer for a moment instead of each asking Cloud again. The directory is a
/// freshness filter, so its answer is reused briefly; a grant lives 60 s and is reused for half.
const DIRECTORY_REUSE: Duration = Duration::from_secs(10);
const GRANT_REUSE: Duration = Duration::from_secs(30);

/// The latest answer for one key. The lock is held while Cloud is asked, so concurrent readers
/// wait for that one answer rather than each sending a request.
struct Reuse<T>(tokio::sync::Mutex<Option<(String, Instant, T)>>);

impl<T: Clone> Reuse<T> {
    const fn new() -> Self {
        Self(tokio::sync::Mutex::const_new(None))
    }

    async fn get<F>(&self, key: String, max_age: Duration, ask: F) -> Result<T, DaemonError>
    where
        F: std::future::Future<Output = Result<T, DaemonError>>,
    {
        let mut latest = self.0.lock().await;
        if let Some((cached_key, at, value)) = latest.as_ref() {
            if *cached_key == key && at.elapsed() < max_age {
                return Ok(value.clone());
            }
        }
        let value = ask.await?;
        *latest = Some((key, Instant::now(), value.clone()));
        Ok(value)
    }
}

static DIRECTORY: Reuse<Option<BTreeSet<String>>> = Reuse::new();
static GRANT: Reuse<String> = Reuse::new();

pub(crate) async fn metadata_token(config: &DaemonConfig) -> Result<String, DaemonError> {
    // MP-08 / MP-10 / MP-11: only the hosted relay of the Cloud profile accepts
    // the Cloud metadata grant; a self-hosted relay keeps its own token (#956).
    let hosted = config
        .relay_url
        .as_deref()
        .is_some_and(|url| config.relay_url_uses_cloud_profile(url));
    if hosted && config.cloud_relay.is_some() {
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
    cloud_metadata_token_for_targets(config, visible_targets(profile).await?).await
}

pub(crate) async fn cloud_metadata_token_for_targets(
    config: &DaemonConfig,
    targets: Option<std::collections::BTreeSet<String>>,
) -> Result<String, DaemonError> {
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "relay_metadata_query",
            message: "Cloud relay profile is not configured".into(),
        })?;
    let key = format!("{}|{}|{targets:?}", profile.api_url, config.daemon_id);
    // Issuance failure must never fall back to the kernel's runtime grant.
    GRANT
        .get(key, GRANT_REUSE, async {
            Ok(issue_cloud_relay_inventory_discovery_token(
                profile,
                &config.daemon_id,
                targets.map(|ids| ids.into_iter().collect()),
            )
            .await?
            .token)
        })
        .await
}

// Kernel discovery must use the same owner-visible target set for every query.
// Keep the legacy machine-credential path for compatible self-hosted enrollments.
pub(crate) async fn visible_targets(
    profile: &crate::config::PersistedCloudRelayProfile,
) -> Result<Option<BTreeSet<String>>, DaemonError> {
    if profile.kernel_credential.is_none() {
        return Ok(None);
    }
    let key = format!(
        "{}|{}|{}|{:?}",
        profile.api_url, profile.account_id, profile.realm_id, profile.kernel_credential
    );
    DIRECTORY
        .get(key, DIRECTORY_REUSE, async {
            let directory =
                crate::runtime::cloud_api_client::get_cloud_kernel_directory(profile).await?;
            cloud_directory_discovery_targets(&directory).map(Some)
        })
        .await
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
        // The owner directory retains enrollment history. Live discovery needs
        // only Cloud-fresh online targets; dormant history must not exhaust the
        // bounded inventory grant or make an offline sibling look available.
        .filter(|target| target.get("status").and_then(serde_json::Value::as_str) == Some("ONLINE"))
        .filter_map(|target| {
            target
                .get("daemonId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloud_directory_excludes_inactive_siblings_from_discovery_grants() {
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
        assert_eq!(targets, ["owner".to_string()].into());
    }

    /// A Cloud that answers the directory and the grant, and counts what it was asked.
    fn counting_cloud() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = std::sync::Arc::clone(&asked);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0_u8; 4096];
                let (header_end, length) = loop {
                    let n = stream.read(&mut chunk).unwrap();
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .map_or(0, |value| value.trim().parse().unwrap());
                        break (end + 4, length);
                    }
                };
                while request.len() < header_end + length {
                    let n = stream.read(&mut chunk).unwrap();
                    request.extend_from_slice(&chunk[..n]);
                }
                let line = String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .unwrap()
                    .to_string();
                let body = if line.starts_with("GET /relay/targets") {
                    r#"{"targets":[{"daemonId":"owner","status":"ONLINE"}]}"#
                } else {
                    r#"{"token":"grant","expiresAt":"2030-01-01T00:00:00Z"}"#
                };
                seen.lock().unwrap().push(line);
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
            }
        });
        (api_url, asked)
    }

    // One inventory refresh reads the directory and a grant for itself and for every machine it
    // lists; staging answered those 5-13 requests per refresh with 429.
    #[tokio::test]
    async fn readers_in_a_moment_share_one_directory_read_and_one_grant() {
        let (api_url, asked) = counting_cloud();
        let mut config = DaemonConfig::for_tests();
        config.daemon_id = "reader".into();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url,
            account_id: "account".into(),
            realm_id: "realm".into(),
            machine_id: Some("machine".into()),
            kernel_credential: Some("credential".into()),
            kernel_public_key_thumbprint: Some("thumbprint".into()),
            ..Default::default()
        });
        let readers = (0..6).map(|_| cloud_metadata_token(&config));
        for token in futures_util::future::join_all(readers).await {
            assert_eq!(token.unwrap(), "grant");
        }
        assert_eq!(cloud_metadata_token(&config).await.unwrap(), "grant");
        let asked = asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 2, "{asked:?}");
        assert!(
            asked[0].starts_with("GET /relay/targets") && asked[1].starts_with("POST /relay/token")
        );
    }

    #[test]
    fn malformed_directory_fails_closed() {
        assert!(cloud_directory_discovery_targets(&serde_json::json!({})).is_err());
    }

    #[test]
    fn live_discovery_does_not_grant_offline_enrollment_history() {
        let mut targets = (0..71)
            .map(|id| serde_json::json!({"daemonId": format!("retired-run-{id}"), "status": "OFFLINE"}))
            .collect::<Vec<_>>();
        targets.extend([
            serde_json::json!({"daemonId": "owner", "status": "ONLINE"}),
            serde_json::json!({"daemonId": "worker", "status": "ONLINE"}),
            serde_json::json!({"daemonId": "stale", "status": "STALE"}),
            serde_json::json!({"daemonId": "unknown"}),
        ]);
        let visible =
            cloud_directory_discovery_targets(&serde_json::json!({"targets": targets})).unwrap();
        assert_eq!(visible, ["owner".to_string(), "worker".to_string()].into());
    }
}
