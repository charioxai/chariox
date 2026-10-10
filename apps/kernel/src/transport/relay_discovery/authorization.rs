//! Authorization for read-only relay metadata queries.

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::issue_cloud_relay_inventory_discovery_token;

pub(super) async fn metadata_token(config: &DaemonConfig) -> Result<String, DaemonError> {
    if let Some(profile) = config.cloud_relay.as_ref() {
        // Use the same scoped grant for projected inventory and fresh lookups.
        // Issuance failure must not fall back to the machine runtime grant.
        return Ok(issue_cloud_relay_inventory_discovery_token(
            profile,
            &config.daemon_id,
            visible_targets(profile)
                .await?
                .map(|ids| ids.into_iter().collect()),
        )
        .await?
        .token);
    }
    config
        .relay_token
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "relay_metadata_query",
            message: "relay_token is not configured".to_string(),
        })
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
}
