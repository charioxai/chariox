//! Temporary credentials for relay metadata, distinct from kernel peer authority.

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::issue_cloud_relay_inventory_discovery_token;

/// A transport-only snapshot for an already issued metadata credential. Never
/// persist it: the original Cloud pairing and kernel peer token stay intact.
/// Removing the bootstrap profile here prevents per-query preparation from
/// replacing a caller's narrower slice credential or fetching it twice.
pub(crate) fn with_metadata_token(config: &DaemonConfig, token: String) -> DaemonConfig {
    let mut discovery = config.clone();
    discovery.relay_token = Some(token);
    discovery.cloud_relay = None;
    discovery
}

pub(crate) async fn metadata_discovery_config(
    config: &DaemonConfig,
) -> Result<DaemonConfig, DaemonError> {
    let Some(profile) = config.cloud_relay.as_ref().filter(|_| {
        config
            .relay_url
            .as_deref()
            .is_some_and(|url| config.relay_url_uses_cloud_profile(url))
    }) else {
        // Self-hosted relays retain their configured opaque credential.
        return Ok(config.clone());
    };
    let issued = tokio::time::timeout(
        std::time::Duration::from_millis(config.relay_request_timeout_ms),
        issue_cloud_relay_inventory_discovery_token(profile, &config.daemon_id),
    )
    .await
    .map_err(|_| DaemonError::LocalTransport {
        operation: "issue relay metadata credential",
        message: "Cloud metadata credential request timed out".into(),
    })??;
    Ok(with_metadata_token(config, issued.token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn self_hosted_metadata_keeps_its_configured_token() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("ws://127.0.0.1:12345".into());
        config.relay_token = Some("opaque-self-hosted-token".into());
        let discovery = metadata_discovery_config(&config).await.unwrap();
        assert_eq!(discovery.relay_token, config.relay_token);
    }

    #[tokio::test]
    async fn explicit_slice_metadata_scope_is_not_replaced_and_pairing_is_preserved() {
        let mut config = DaemonConfig::for_tests();
        config.relay_token = Some("kernel-peer-token".into());
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile::default());
        let scoped = with_metadata_token(&config, "slice-metadata-token".into());
        let prepared = metadata_discovery_config(&scoped).await.unwrap();
        assert_eq!(
            prepared.relay_token.as_deref(),
            Some("slice-metadata-token")
        );
        assert_eq!(config.relay_token.as_deref(), Some("kernel-peer-token"));
        assert!(config.cloud_relay.is_some());
    }
}
