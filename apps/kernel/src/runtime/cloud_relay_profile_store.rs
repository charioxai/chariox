use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::is_stale_cloud_link_error;
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

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

pub(crate) async fn persist_cloud_profile(
    runtime_state: &KernelRuntimeState,
    profile: PersistedCloudRelayProfile,
) -> Result<PersistedCloudRelayProfile, DaemonError> {
    runtime_state
        .persist_cloud_relay_profile(Some(profile.clone()))
        .await?;
    Ok(profile)
}

pub(crate) async fn clear_cloud_profile(
    runtime_state: &KernelRuntimeState,
) -> Result<(), DaemonError> {
    runtime_state.persist_cloud_relay_profile(None).await?;
    Ok(())
}

pub(crate) async fn clear_cloud_profile_if_stale(
    runtime_state: &KernelRuntimeState,
    error: &DaemonError,
) -> Result<(), DaemonError> {
    if !is_stale_cloud_link_error(error) {
        return Ok(());
    }
    clear_cloud_profile(runtime_state).await
}
