//! Temporary credentials for relay metadata, distinct from kernel peer authority.

use crate::config::DaemonConfig;
use crate::error::DaemonError;

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
    crate::runtime::cloud_api_client::relay_metadata_discovery_config(config).await
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
