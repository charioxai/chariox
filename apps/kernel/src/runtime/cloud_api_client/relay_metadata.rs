// MP-08 / MP-10 / MP-11: metadata grants stay separate from kernel peer grants.
use crate::{config::DaemonConfig, error::DaemonError};

pub(crate) async fn relay_metadata_discovery_config(
    config: &DaemonConfig,
) -> Result<DaemonConfig, DaemonError> {
    let mut discovery = config.clone();
    if config
        .relay_url
        .as_deref()
        .is_some_and(|url| config.relay_url_uses_cloud_profile(url))
    {
        if let Some(profile) = config.cloud_relay.as_ref() {
            let token =
                super::issue_cloud_relay_inventory_discovery_token(profile, &config.daemon_id)
                    .await?;
            discovery.relay_token = Some(token.token);
        }
    }
    Ok(discovery)
}
