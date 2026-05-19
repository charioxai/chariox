use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::DaemonError;
use crate::local::{
    CloudRelayRuntimeToken, ConnectCloudRelayRequest, IssueCloudRelayClientTokenRequest,
    LocalDaemonResponse,
};
use crate::runtime::cloud_api_client::{
    cloud_profile_from_persisted, issue_cloud_runtime_token, post_cloud_json,
    CloudPairingTokenResponse,
};
use crate::runtime::cloud_relay_control::{
    cloud_relay_profile_has_runtime_credentials, cloud_relay_runtime_token_is_fresh,
    cloud_runtime_token_subject, CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS,
};
use crate::runtime::cloud_relay_profile_store::{
    clear_cloud_profile_if_stale, persist_cloud_profile, required_cloud_relay_profile,
};
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use crate::runtime::remote_relay_inventory::projected_relay_status;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_client::RelayClientState;

pub(crate) async fn execute_cloud_relay_status_request(
    config_projection: &DaemonConfigProjectionStore,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = config_projection
        .snapshot()
        .cloud_relay
        .as_ref()
        .map(cloud_profile_from_persisted);
    Ok(LocalDaemonResponse::CloudRelayStatus { profile })
}

pub(crate) async fn ensure_cloud_relay_connection(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
) -> Result<(), DaemonError> {
    let config = config_projection.snapshot();
    let Some(profile) = config.cloud_relay.clone() else {
        return Ok(());
    };
    if !cloud_relay_profile_has_runtime_credentials(&profile) {
        return Ok(());
    }
    let now_ms = crate::session::unix_epoch_ms();
    if cloud_relay_runtime_token_is_fresh(&config, &profile, now_ms) {
        return Ok(());
    }

    let token_subject = cloud_runtime_token_subject(&config, &profile);
    let issued = match issue_cloud_runtime_token(
        &profile,
        &token_subject.subject,
        token_subject.subject_kind,
        None,
        None,
        token_subject.machine_id,
        None,
    )
    .await
    {
        Ok(issued) => issued,
        Err(error) => {
            clear_cloud_profile_if_stale(runtime_state, &error).await?;
            return Err(error);
        }
    };
    let mut updated_profile = profile.clone();
    updated_profile.token_expires_at_ms = Some(now_ms + CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS);
    runtime_state
        .configure_relay_with_cloud_profile(
            Some(profile.relay_url),
            Some(issued.token),
            Some(updated_profile),
            false,
        )
        .await?;
    Ok(())
}

pub(crate) async fn execute_connect_cloud_relay_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    _request: ConnectCloudRelayRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut profile = required_cloud_relay_profile(config_projection)?;
    let daemon_id = config_projection.snapshot().daemon_id;
    let (subject, subject_kind, machine_id) = if let Some(machine_id) = profile.machine_id.clone() {
        (machine_id.clone(), "machine", Some(machine_id))
    } else {
        (daemon_id, "kernel", None)
    };
    let issued = issue_cloud_runtime_token(
        &profile,
        &subject,
        subject_kind,
        None,
        None,
        machine_id,
        None,
    )
    .await?;
    profile.token_expires_at_ms =
        Some(crate::session::unix_epoch_ms() + CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS);
    let saved = persist_cloud_profile(runtime_state, profile.clone()).await?;
    runtime_state
        .configure_relay(
            Some(profile.relay_url.clone()),
            Some(issued.token.clone()),
            true,
        )
        .await?;
    provider_catalog_projection.invalidate();
    let token = CloudRelayRuntimeToken {
        relay_url: profile.relay_url,
        relay_token: issued.token,
        token_expires_at: issued.expires_at,
    };
    Ok(LocalDaemonResponse::CloudRelayConnected {
        status: projected_relay_status(relay_state, config_projection.clone()).await,
        profile: cloud_profile_from_persisted(&saved),
        token,
    })
}

pub(crate) async fn execute_issue_cloud_relay_client_token_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: IssueCloudRelayClientTokenRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut profile = required_cloud_relay_profile(config_projection)?;
    if profile.client_id.is_none() {
        let pairing: CloudPairingTokenResponse = match post_cloud_json(
            profile.api_url.clone(),
            "/pairing-tokens",
            serde_json::json!({
                "accountId": profile.account_id,
                "createdByUserId": profile.user_id,
                "subjectKind": "client",
            }),
        )
        .await
        {
            Ok(pairing) => pairing,
            Err(error) => {
                clear_cloud_profile_if_stale(runtime_state, &error).await?;
                return Err(error);
            }
        };
        if let Err(error) = post_cloud_json::<serde_json::Value>(
            profile.api_url.clone(),
            "/clients/pair",
            serde_json::json!({
                "accountId": profile.account_id,
                "token": pairing.token,
                "clientId": request.client_id,
                "userId": profile.user_id,
            }),
        )
        .await
        {
            clear_cloud_profile_if_stale(runtime_state, &error).await?;
            return Err(error);
        }
        profile.client_id = Some(request.client_id.clone());
        profile = persist_cloud_profile(runtime_state, profile).await?;
    }
    let client_id = profile
        .client_id
        .clone()
        .unwrap_or_else(|| request.client_id.clone());
    let issued = match issue_cloud_runtime_token(
        &profile,
        &client_id,
        "client",
        Some(vec![request.target_daemon_alias]),
        Some(client_id.clone()),
        profile
            .machine_credential
            .as_ref()
            .and(profile.machine_id.clone()),
        request.session_id,
    )
    .await
    {
        Ok(issued) => issued,
        Err(error) => {
            clear_cloud_profile_if_stale(runtime_state, &error).await?;
            return Err(error);
        }
    };
    let token = CloudRelayRuntimeToken {
        relay_url: profile.relay_url.clone(),
        relay_token: issued.token,
        token_expires_at: issued.expires_at,
    };
    Ok(LocalDaemonResponse::CloudRelayClientTokenIssued {
        profile: cloud_profile_from_persisted(&profile),
        token,
    })
}
