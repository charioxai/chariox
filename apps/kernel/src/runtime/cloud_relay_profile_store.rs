use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::is_stale_cloud_link_error;
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

pub(crate) async fn persist_cloud_profile(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    profile: PersistedCloudRelayProfile,
) -> Result<PersistedCloudRelayProfile, DaemonError> {
    {
        let mut app = app.lock().await;
        app.persist_cloud_relay_profile(Some(profile.clone()))?;
        config_projection.update(app.config().clone());
    }
    Ok(profile)
}

pub(crate) async fn clear_cloud_profile(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
) -> Result<(), DaemonError> {
    {
        let mut app = app.lock().await;
        app.persist_cloud_relay_profile(None)?;
        config_projection.update(app.config().clone());
    }
    Ok(())
}

pub(crate) async fn clear_cloud_profile_if_stale(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    error: &DaemonError,
) -> Result<(), DaemonError> {
    if !is_stale_cloud_link_error(error) {
        return Ok(());
    }
    clear_cloud_profile(app, config_projection).await
}
