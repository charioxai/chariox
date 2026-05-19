use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, PairCloudRelayClientRequest, PairCloudRelayMachineRequest,
};
use crate::runtime::cloud_api_client::{
    cloud_profile_from_persisted, post_cloud_json, CloudPairingTokenResponse,
};
use crate::runtime::cloud_relay_profile_store::{
    persist_cloud_profile, required_cloud_relay_profile,
};
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use crate::runtime::provider_catalog_control::provider_catalog_json_value;
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::waiting_room_public_projection::infer_waiting_room_launch_target;

pub(crate) async fn execute_pair_cloud_relay_client_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: PairCloudRelayClientRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut profile = required_cloud_relay_profile(config_projection)?;
    let pairing: CloudPairingTokenResponse = post_cloud_json(
        profile.api_url.clone(),
        "/pairing-tokens",
        serde_json::json!({
            "accountId": profile.account_id,
            "createdByUserId": profile.user_id,
            "subjectKind": "client",
        }),
    )
    .await?;
    post_cloud_json::<serde_json::Value>(
        profile.api_url.clone(),
        "/clients/pair",
        serde_json::json!({
            "accountId": profile.account_id,
            "token": pairing.token,
            "clientId": request.client_id,
            "userId": profile.user_id,
            "alias": request.alias,
        }),
    )
    .await?;
    profile.client_id = Some(request.client_id);
    if request.alias.is_some() {
        profile.client_alias = request.alias;
    }
    let saved = persist_cloud_profile(runtime_state, profile).await?;
    Ok(LocalDaemonResponse::CloudRelayClientPaired {
        profile: cloud_profile_from_persisted(&saved),
    })
}

pub(crate) async fn execute_pair_cloud_relay_machine_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    request: PairCloudRelayMachineRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut profile = required_cloud_relay_profile(config_projection)?;
    let pairing: CloudPairingTokenResponse = post_cloud_json(
        profile.api_url.clone(),
        "/pairing-tokens",
        serde_json::json!({
            "accountId": profile.account_id,
            "createdByUserId": profile.user_id,
            "subjectKind": "machine",
        }),
    )
    .await?;
    post_cloud_json::<serde_json::Value>(
        profile.api_url.clone(),
        "/machines/pair",
        serde_json::json!({
            "accountId": profile.account_id,
            "token": pairing.token,
            "machineId": request.machine_id,
            "userId": profile.user_id,
            "alias": request.alias,
            "runtimeProfile": machine_runtime_profile_payload(
                config_projection,
                provider_catalog_projection,
            ).await,
        }),
    )
    .await?;
    profile.machine_id = Some(request.machine_id);
    if request.alias.is_some() {
        profile.machine_alias = request.alias;
    }
    let saved = persist_cloud_profile(runtime_state, profile).await?;
    Ok(LocalDaemonResponse::CloudRelayMachinePaired {
        profile: cloud_profile_from_persisted(&saved),
    })
}

async fn machine_runtime_profile_payload(
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
) -> serde_json::Value {
    let config = config_projection.snapshot();
    let user_config = config.user_config.clone();
    let provider_catalog =
        provider_catalog_json_value(provider_catalog_projection, config_projection).await;
    let launch_target = infer_waiting_room_launch_target();
    serde_json::json!({
        "profileVersion": 1,
        "providerCatalog": provider_catalog,
        "userConfig": {
            "providers": user_config.providers,
            "ui": user_config.ui,
        },
        "defaultWorkspaceId": launch_target.as_ref().map(|target| target.workspace_id.clone()),
        "defaultWorktreeId": launch_target.as_ref().map(|target| target.worktree_id.clone()),
        "workspaces": launch_target.as_ref().map(|target| serde_json::json!([{
            "workspaceId": target.workspace_id,
            "worktreeId": target.worktree_id,
        }])),
        "os": std::env::consts::OS,
        "homeDir": std::env::var("HOME").ok(),
    })
}
