use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::runtime::projection::DaemonConfigProjectionStore;

pub(crate) fn required_cloud_relay_profile(
    config_projection: &DaemonConfigProjectionStore,
) -> Result<PersistedCloudRelayProfile, DaemonError> {
    config_projection
        .snapshot()
        .cloud_relay
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "load cloud relay profile",
            message: "cloud relay profile missing; run /relay cloud login first".to_string(),
        })
}

pub(crate) fn required_cloud_relay_profile_with_session(
    config_projection: &DaemonConfigProjectionStore,
) -> Result<PersistedCloudRelayProfile, DaemonError> {
    let profile = required_cloud_relay_profile(config_projection)?;
    if profile
        .cloud_session_token
        .as_deref()
        .unwrap_or("")
        .is_empty()
    {
        return Err(DaemonError::LocalTransport {
            operation: "load cloud relay session",
            message: "cloud session token missing; run /relay cloud login first".to_string(),
        });
    }
    Ok(profile)
}
