//! MP-01: worker metadata uses Cloud's read-only discovery authority.
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::issue_cloud_slice_discovery_token;

pub(super) async fn worker_config(
    owner_kernel_ref: &str,
    worker_kernel_ref: &str,
    hosted_shared_slice: bool,
    mut config: DaemonConfig,
) -> Result<DaemonConfig, DaemonError> {
    let uses_cloud = config
        .relay_url
        .as_deref()
        .is_some_and(|url| config.relay_url_uses_cloud_profile(url));
    if !hosted_shared_slice && !uses_cloud {
        return Ok(config);
    }
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "discover hosted slice worker",
            message: "hosted shared slice requires a Cloud relay profile".into(),
        })?;
    let token = if hosted_shared_slice {
        issue_cloud_slice_discovery_token(profile, owner_kernel_ref, worker_kernel_ref).await?
    } else {
        config.relay_token = Some(
            crate::transport::relay_discovery::authorization::cloud_metadata_token(&config).await?,
        );
        return Ok(config);
    };
    config.relay_token = Some(token.token);
    Ok(config)
}
