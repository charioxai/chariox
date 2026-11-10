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
    // Issuance failure must never fall back to the kernel's runtime grant.
    Ok(issue_cloud_relay_inventory_discovery_token(
        profile,
        &config.daemon_id,
        targets.map(|ids| ids.into_iter().collect()),
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
    let answer = crate::runtime::cloud_api_client::get_cloud_kernel_directory(profile)
        .await
        .and_then(|directory| cloud_directory_discovery_targets(&directory));
    let key = format!(
        "{}/{}/{}",
        profile.api_url, profile.account_id, profile.realm_id
    );
    let mut last = LAST_VISIBLE_TARGETS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match answer {
        Ok(targets) => {
            last.insert(key, targets.clone());
            Ok(Some(targets))
        }
        // The list only decides which kernels to ask the relay about; a Cloud that does not
        // answer for a moment must not take the kernel list down once it has answered.
        Err(error) => last.get(&key).cloned().map(Some).ok_or(error),
    }
}

/// Cloud's last answer to "which kernels are visible", by account and realm.
static LAST_VISIBLE_TARGETS: std::sync::Mutex<
    std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());

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

    // MP-08 / MP-09 / MP-10 / MP-11: Cloud answered once, then went silent (a saturated
    // link): the kernel list keeps routing by that answer; a kernel Cloud never answered for
    // still fails.
    #[tokio::test]
    async fn a_silent_cloud_leaves_the_last_visible_targets_in_use() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let serve = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            let body = r#"{"targets":[{"daemonId":"k-b3","status":"ONLINE"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let profile = crate::config::PersistedCloudRelayProfile {
            api_url,
            account_id: "account-last-answer".into(),
            kernel_credential: Some("synthetic-enrollment".into()),
            ..Default::default()
        };
        let first = visible_targets(&profile).await.unwrap().unwrap();
        assert_eq!(first, ["k-b3".to_string()].into());
        serve.join().unwrap();
        let second = visible_targets(&profile)
            .await
            .expect("the last answer routes while Cloud is silent");
        assert_eq!(second, Some(first));
        let never_answered = crate::config::PersistedCloudRelayProfile {
            account_id: "account-never-answered".into(),
            ..profile
        };
        assert!(visible_targets(&never_answered).await.is_err());
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
