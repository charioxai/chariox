use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, PairCloudRelayClientRequest, PairCloudRelayMachineRequest,
};
use crate::runtime::cloud_api_client::{
    cloud_profile_from_persisted, post_cloud_json, request_account_pairing_token,
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
    let pairing = request_account_pairing_token(&profile, "client").await?;
    let mut body = serde_json::json!({
        "accountId": profile.account_id,
        "token": pairing.token,
        "clientId": request.client_id,
        "userId": profile.user_id,
    });
    if let Some(alias) = request.alias.as_deref() {
        body["alias"] = serde_json::Value::String(alias.to_string());
    }
    post_cloud_json::<serde_json::Value>(profile.api_url.clone(), "/clients/pair", body).await?;
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
    let pairing = request_account_pairing_token(&profile, "machine").await?;
    let body = machine_pairing_body(
        &profile,
        &config_projection.snapshot().relay_public_key,
        &pairing.token,
        &request,
        machine_runtime_profile_payload(
            config_projection,
            provider_catalog_projection,
            runtime_state.provider_account_profile_registry(),
        )
        .await,
    );
    post_cloud_json::<serde_json::Value>(profile.api_url.clone(), "/machines/pair", body).await?;
    profile.machine_id = Some(request.machine_id);
    if request.alias.is_some() {
        profile.machine_alias = request.alias;
    }
    let saved = persist_cloud_profile(runtime_state, profile).await?;
    Ok(LocalDaemonResponse::CloudRelayMachinePaired {
        profile: cloud_profile_from_persisted(&saved),
    })
}

fn machine_pairing_body(
    profile: &crate::config::PersistedCloudRelayProfile,
    relay_public_key: &str,
    token: &str,
    request: &PairCloudRelayMachineRequest,
    mut runtime_profile: serde_json::Value,
) -> serde_json::Value {
    // MP-08: this profile is an optional startup hint. Large provider catalogs
    // must be queried from the kernel, not embedded in machine admission.
    const PROFILE_LIMIT_BYTES: usize = 192 * 1024;
    let fits = |value: &serde_json::Value| {
        serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= PROFILE_LIMIT_BYTES)
    };
    if !fits(&runtime_profile) {
        if let Some(profile) = runtime_profile.as_object_mut() {
            profile.remove("providerCatalog");
        }
        if !fits(&runtime_profile) {
            runtime_profile = serde_json::json!({"profileVersion": 1});
        }
    }
    let mut body = serde_json::json!({
        "accountId": profile.account_id,
        "token": token,
        "machineId": request.machine_id,
        "userId": profile.user_id,
        "runtimeProfile": runtime_profile,
        "publicKeyThumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(relay_public_key),
    });
    if let Some(alias) = request.alias.as_deref() {
        body["alias"] = serde_json::Value::String(alias.to_string());
    }
    body
}

async fn machine_runtime_profile_payload(
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    account_profiles: &crate::account_profile::ProviderAccountProfileRegistry,
) -> serde_json::Value {
    let config = config_projection.snapshot();
    let user_config = config.user_config.clone();
    let provider_catalog = provider_catalog_json_value(
        provider_catalog_projection,
        config_projection,
        account_profiles,
    )
    .await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(runtime_profile: serde_json::Value) -> serde_json::Value {
        let config = crate::config::DaemonConfig::for_tests();
        machine_pairing_body(
            &crate::config::PersistedCloudRelayProfile {
                account_id: "account-fixture".into(),
                user_id: "user-fixture".into(),
                ..Default::default()
            },
            "synthetic-public-key",
            "synthetic-pairing-token",
            &PairCloudRelayMachineRequest {
                machine_id: config.host_machine_id.clone(),
                alias: Some("MP-08 fresh machine".into()),
            },
            runtime_profile,
        )
    }

    #[test]
    fn machine_pairing_registers_the_product_relay_key_thumbprint() {
        let value = body(json!({"profileVersion": 1}));
        assert_eq!(
            value["publicKeyThumbprint"],
            crate::runtime::terminal_pairings::public_key_thumbprint("synthetic-public-key")
        );
        assert_eq!(value["alias"], "MP-08 fresh machine");
        assert!(value.get("relayPrivateKey").is_none());
    }

    #[test]
    fn machine_pairing_bounds_optional_profile_metadata_without_truncating_catalogs() {
        let small = json!({"profileVersion": 1, "providerCatalog": {"all": [], "connected": []}});
        assert_eq!(body(small.clone())["runtimeProfile"], small);
        let large = json!({
            "profileVersion": 1,
            "providerCatalog": {"all": [{"id": "opencode", "models": {"model": {
                "variants": {"default": {"providerOptions": "x".repeat(2 * 1024 * 1024)}}
            }}}]},
            "userConfig": {"providers": {"default": "opencode"}},
            "os": "linux",
        });
        let value = body(large);
        assert!(
            serde_json::to_vec(&value).unwrap().len() <= 256 * 1024,
            "MP-08 normal pairing must fit Cloud's bounded JSON request"
        );
        assert!(
            value["runtimeProfile"].get("providerCatalog").is_none(),
            "an oversized optional catalog must be omitted, never partially advertised"
        );
        assert_eq!(
            value["runtimeProfile"]["userConfig"]["providers"]["default"],
            "opencode"
        );
        assert_eq!(value["runtimeProfile"]["os"], "linux");
        let oversized_config = body(
            json!({"profileVersion":1, "userConfig":{"providers":{"model":"x".repeat(1024*1024)}}}),
        );
        assert!(serde_json::to_vec(&oversized_config).unwrap().len() <= 256 * 1024);
    }
}
