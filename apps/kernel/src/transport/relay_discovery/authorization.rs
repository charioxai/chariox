//! Authorization for read-only relay metadata queries.

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::issue_cloud_relay_inventory_discovery_token;

pub(super) async fn metadata_token(config: &DaemonConfig) -> Result<String, DaemonError> {
    if let Some(profile) = config.cloud_relay.as_ref() {
        // Use the same scoped grant for projected inventory and fresh lookups.
        // Issuance failure must not fall back to the machine runtime grant.
        return Ok(
            issue_cloud_relay_inventory_discovery_token(profile, &config.daemon_id)
                .await?
                .token,
        );
    }
    config
        .relay_token
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "relay_metadata_query",
            message: "relay_token is not configured".to_string(),
        })
}
