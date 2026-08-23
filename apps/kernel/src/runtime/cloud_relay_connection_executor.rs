use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::DaemonError;
use crate::local::{
    CloudRelayProfile, CloudRelayRuntimeToken, ConnectCloudRelayRequest,
    IssueCloudRelayClientTokenRequest, KernelClientConnection, LocalDaemonResponse,
    ResolveKernelClientConnectionRequest,
};
use crate::runtime::cloud_api_client::{
    cloud_profile_from_persisted, issue_cloud_runtime_token, post_cloud_json,
    CloudPairingTokenResponse, CloudRuntimeTokenRequestOptions,
};
use crate::runtime::cloud_relay_control::{
    cloud_relay_profile_has_runtime_credentials, cloud_relay_runtime_token_is_fresh,
    cloud_runtime_token_subject, CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS,
};
use crate::runtime::cloud_relay_profile_store::{
    clear_cloud_profile_if_stale, persist_cloud_profile, required_cloud_relay_profile,
};
use crate::runtime::projection::{
    DaemonConfigProjectionStore, ProviderCatalogProjectionStore,
    RemoteRelayInventoryProjectionStore,
};
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
    let public_key_thumbprint =
        crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key);
    let issued = match issue_cloud_runtime_token(
        &profile,
        &token_subject.subject,
        token_subject.subject_kind,
        CloudRuntimeTokenRequestOptions {
            machine_id: token_subject.machine_id,
            public_key_thumbprint: Some(public_key_thumbprint),
            ..CloudRuntimeTokenRequestOptions::default()
        },
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
    let config = config_projection.snapshot();
    let daemon_id = config.daemon_id;
    let (subject, subject_kind, machine_id) = if let Some(machine_id) = profile.machine_id.clone() {
        (machine_id.clone(), "machine", Some(machine_id))
    } else {
        (daemon_id, "kernel", None)
    };
    let issued = issue_cloud_runtime_token(
        &profile,
        &subject,
        subject_kind,
        CloudRuntimeTokenRequestOptions {
            machine_id,
            public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
                &config.relay_public_key,
            )),
            ..CloudRuntimeTokenRequestOptions::default()
        },
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
    let (profile, token) =
        issue_cloud_relay_client_token(runtime_state, config_projection, request).await?;
    Ok(LocalDaemonResponse::CloudRelayClientTokenIssued { profile, token })
}

pub(crate) async fn execute_resolve_kernel_client_connection_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    remote_relay_inventory_projection: &RemoteRelayInventoryProjectionStore,
    request: ResolveKernelClientConnectionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let kernel = resolve_relay_kernel_presence(remote_relay_inventory_projection, &request)?;
    let target_daemon_alias = relay_kernel_target_alias(&kernel);
    let config = config_projection.snapshot();
    let connection = if config.cloud_relay.is_some() {
        let client_id = request
            .client_id
            .clone()
            .unwrap_or_else(|| format!("chariox-cli-{}", std::process::id()));
        let (_, token) = issue_cloud_relay_client_token(
            runtime_state,
            config_projection,
            IssueCloudRelayClientTokenRequest {
                // The relay authenticates scoped client tokens against the
                // connect target id before alias, so scope resolved kernel
                // connections to the exact kernel id.
                target_daemon_alias: kernel.kernel_id.clone(),
                client_id,
                session_id: request.session_id.clone(),
            },
        )
        .await?;
        KernelClientConnection {
            relay_url: token.relay_url,
            relay_token: token.relay_token,
            target_daemon_id: Some(kernel.kernel_id.clone()),
            target_daemon_alias: Some(target_daemon_alias),
            token_expires_at: Some(token.token_expires_at),
            machine_id: Some(kernel.machine_id.clone()),
            kernel_id: Some(kernel.kernel_id.clone()),
        }
    } else {
        let relay_url = config.relay_url.clone().ok_or_else(|| DaemonError::LocalTransport {
            operation: "kernel client connection resolve",
            message: "relay is not configured; connect the kernel to a relay before selecting another peer as session owner".to_string(),
        })?;
        let relay_token = config.relay_token.clone().ok_or_else(|| DaemonError::LocalTransport {
            operation: "kernel client connection resolve",
            message: "relay token is not configured; connect the kernel to a relay before selecting another peer as session owner".to_string(),
        })?;
        KernelClientConnection {
            relay_url,
            relay_token,
            target_daemon_id: Some(kernel.kernel_id.clone()),
            target_daemon_alias: Some(target_daemon_alias),
            token_expires_at: None,
            machine_id: Some(kernel.machine_id.clone()),
            kernel_id: Some(kernel.kernel_id.clone()),
        }
    };
    Ok(LocalDaemonResponse::KernelClientConnectionResolved { connection })
}

async fn issue_cloud_relay_client_token(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: IssueCloudRelayClientTokenRequest,
) -> Result<(CloudRelayProfile, CloudRelayRuntimeToken), DaemonError> {
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
        CloudRuntimeTokenRequestOptions {
            allowed_targets: Some(vec![request.target_daemon_alias]),
            client_id: Some(client_id.clone()),
            machine_id: profile
                .machine_credential
                .as_ref()
                .and(profile.machine_id.clone()),
            session_id: request.session_id,
            ..CloudRuntimeTokenRequestOptions::default()
        },
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
    Ok((cloud_profile_from_persisted(&profile), token))
}

fn resolve_relay_kernel_presence(
    remote_relay_inventory_projection: &RemoteRelayInventoryProjectionStore,
    request: &ResolveKernelClientConnectionRequest,
) -> Result<chariox_relay::protocol::RelayKernelPresence, DaemonError> {
    let kernel_ref = request.kernel_ref.trim();
    if kernel_ref.is_empty() || kernel_ref == "local" {
        return Err(DaemonError::LocalTransport {
            operation: "kernel client connection resolve",
            message: "remote kernel selection is empty".to_string(),
        });
    }
    let machine_ref = request
        .machine_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "local");
    let (_, kernels) = remote_relay_inventory_projection.snapshot();
    let mut matches: Vec<_> = kernels
        .into_iter()
        .filter(|kernel| {
            kernel.kernel_id == kernel_ref
                || kernel.relay_alias.as_deref() == Some(kernel_ref)
                || kernel.kernel_alias.as_deref() == Some(kernel_ref)
        })
        .filter(|kernel| {
            machine_ref.is_none_or(|machine_ref| {
                kernel.machine_id == machine_ref
                    || kernel.machine_alias.as_deref() == Some(machine_ref)
                    || kernel.relay_alias.as_deref() == Some(machine_ref)
                    || kernel.kernel_alias.as_deref() == Some(machine_ref)
            })
        })
        .collect();
    if matches.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "kernel client connection resolve",
            message: format!(
                "kernel `{kernel_ref}` is not present in the reachable relay inventory"
            ),
        });
    }
    if matches.len() > 1 {
        return Err(DaemonError::LocalTransport {
            operation: "kernel client connection resolve",
            message: format!("kernel `{kernel_ref}` is ambiguous in the reachable relay inventory"),
        });
    }
    Ok(matches.remove(0))
}

fn relay_kernel_target_alias(kernel: &chariox_relay::protocol::RelayKernelPresence) -> String {
    kernel
        .relay_alias
        .clone()
        .or_else(|| kernel.kernel_alias.clone())
        .unwrap_or_else(|| kernel.kernel_id.clone())
}
