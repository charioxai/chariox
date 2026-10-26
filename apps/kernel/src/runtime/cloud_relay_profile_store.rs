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
            message: "this operation requires a browser/client session; kernel enrollment does not confer human account authority".to_string(),
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

/// Upgrade only a previously registered, key-pinned kernel. A shared predecessor
/// is a migration source, never authority to enroll another kernel.
pub(crate) async fn migrate_legacy_kernel_profile(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
) -> Result<(), DaemonError> {
    let config = config_projection.snapshot();
    let Some(mut profile) = config.cloud_relay.clone() else {
        return Ok(());
    };
    if profile.kernel_credential.is_some() {
        return Ok(());
    }
    // Managed bootstrap remains its explicit dedicated capability path.
    if crate::managed_bootstrap::confirmed_managed_kernel_registration_from_env()?
        .is_some_and(|r| r.kernel_id == config.daemon_id && r.machine_id == config.host_machine_id)
    {
        return Ok(());
    }
    let credential = profile.machine_credential.as_deref().ok_or_else(|| DaemonError::LocalTransport {
        operation: "migrate kernel Cloud enrollment", message: "legacy human session cannot enroll a kernel; run /cloud link to enroll this kernel independently".into(),
    })?;
    let response: serde_json::Value = crate::runtime::cloud_api_client::post_cloud_json(profile.api_url.clone(), "/auth/kernel/migrate", serde_json::json!({
        "accountId": profile.account_id, "realmId": profile.realm_id,
        "machineId": config.host_machine_id, "kernelId": config.daemon_id,
        "machineCredential": credential,
        "publicKeyThumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
    })).await?;
    let grant = response
        .get("kernelCredential")
        .and_then(serde_json::Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "migrate kernel Cloud enrollment",
            message: "Cloud migration did not return a kernel credential".into(),
        })?;
    profile.kernel_public_key_thumbprint = Some(
        crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
    );
    profile.kernel_id = Some(config.daemon_id);
    profile.kernel_credential = Some(grant.to_string());
    profile.machine_id = Some(config.host_machine_id);
    profile.machine_credential = None;
    profile.cloud_session_token = None;
    profile.cloud_session_expires_at_ms = None;
    profile.client_id = None;
    profile.client_alias = None;
    profile.token_expires_at_ms = None;
    persist_cloud_profile(runtime_state, profile).await?;
    Ok(())
}
