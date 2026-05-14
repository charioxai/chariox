use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::app::DaemonApp;
use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::local::{
    AcceptCloudSessionInviteRequest, CloudRelayLoginPoll, CloudRelayLoginPollStatus,
    CloudRelayLoginStart, CloudRelayRuntimeToken, ConnectCloudRelayRequest,
    CreateCloudSessionInviteRequest, IssueCloudRelayClientTokenRequest,
    ListCloudCollaboratorsRequest, ListCloudSessionMembersRequest, LocalDaemonResponse,
    LogoutCloudRelayRequest, PairCloudRelayClientRequest, PairCloudRelayMachineRequest,
    PollCloudRelayLoginRequest, RevokeCloudSessionInviteRequest, ShowCloudSessionInviteRequest,
    StartCloudRelayLoginRequest,
};
use crate::runtime::cloud_api_client::{
    accept_cloud_session_invite, cloud_profile_from_persisted, create_cloud_session_invite,
    is_stale_cloud_link_error, issue_cloud_runtime_token, list_cloud_collaborators,
    list_cloud_session_members, normalize_cloud_api_url, post_cloud_json,
    revoke_cloud_session_invite, show_cloud_session_invite, CloudDevicePollResponse,
    CloudDeviceStartResponse, CloudPairingTokenResponse,
};
use crate::runtime::cloud_relay_control::{
    cloud_relay_profile_has_runtime_credentials, cloud_relay_runtime_token_is_fresh,
    cloud_runtime_token_subject, CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS,
};
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use crate::runtime::provider_catalog_control::provider_catalog_json_value;
use crate::runtime::remote_relay_inventory::projected_relay_status;
use crate::runtime::waiting_room_public_projection::infer_waiting_room_launch_target;
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
    app: &Arc<Mutex<DaemonApp>>,
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
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    let mut updated_profile = profile.clone();
    updated_profile.token_expires_at_ms = Some(now_ms + CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS);
    {
        let mut app = app.lock().await;
        app.configure_relay(Some(profile.relay_url), Some(issued.token))?;
        app.persist_cloud_relay_profile(Some(updated_profile))?;
        config_projection.update(app.config().clone());
    }
    Ok(())
}

pub(crate) async fn execute_start_cloud_relay_login_request(
    request: StartCloudRelayLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let api_url = normalize_cloud_api_url(&request.api_url)?;
    let response: CloudDeviceStartResponse = post_cloud_json(
        api_url.clone(),
        "/auth/device/start",
        serde_json::json!({
            "clientId": request.client_id,
            "clientAlias": request.client_alias,
            "machineId": request.machine_id,
            "machineAlias": request.machine_alias,
        }),
    )
    .await?;
    Ok(LocalDaemonResponse::CloudRelayLoginStarted {
        login: CloudRelayLoginStart {
            api_url,
            device_code: response.device_code,
            user_code: response.user_code,
            verification_url: response.verification_url,
            expires_at: response.expires_at,
            interval_seconds: response.interval_seconds,
        },
    })
}

pub(crate) async fn execute_poll_cloud_relay_login_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: PollCloudRelayLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let api_url = normalize_cloud_api_url(&request.api_url)?;
    let response: CloudDevicePollResponse = post_cloud_json(
        api_url.clone(),
        "/auth/device/poll",
        serde_json::json!({ "deviceCode": request.device_code }),
    )
    .await?;
    let result = match response.status.as_str() {
        "authorization_pending" => CloudRelayLoginPoll {
            status: CloudRelayLoginPollStatus::AuthorizationPending,
            interval_seconds: response.interval_seconds,
            expires_at: response.expires_at,
            profile: None,
        },
        "expired_token" => CloudRelayLoginPoll {
            status: CloudRelayLoginPollStatus::ExpiredToken,
            interval_seconds: None,
            expires_at: None,
            profile: None,
        },
        "approved" => {
            let profile = response
                .profile
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "poll cloud relay login",
                    message: "cloud approval response did not include a profile".to_string(),
                })?;
            let session_token =
                response
                    .cloud_session_token
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "poll cloud relay login",
                        message: "cloud approval response did not include a session token"
                            .to_string(),
                    })?;
            let persisted = PersistedCloudRelayProfile {
                api_url,
                email: profile.email,
                account_id: profile.account_id,
                user_id: profile.user_id,
                account_slug: profile.account_slug,
                realm_id: profile.realm_id,
                relay_url: profile.relay_url,
                issuer_id: profile.issuer_id,
                client_id: profile.client_id,
                client_alias: profile.client_alias,
                machine_id: profile.machine_id,
                machine_alias: profile.machine_alias,
                machine_credential: response.machine_credential,
                cloud_session_token: Some(session_token),
                cloud_session_expires_at_ms: None,
                token_expires_at_ms: None,
            };
            persist_cloud_profile(app, config_projection, persisted.clone()).await?;
            CloudRelayLoginPoll {
                status: CloudRelayLoginPollStatus::Approved,
                interval_seconds: None,
                expires_at: response.cloud_session_expires_at,
                profile: Some(cloud_profile_from_persisted(&persisted)),
            }
        }
        other => {
            return Err(DaemonError::LocalTransport {
                operation: "poll cloud relay login",
                message: format!("cloud returned unknown device login status `{other}`"),
            });
        }
    };
    Ok(LocalDaemonResponse::CloudRelayLoginPolled { result })
}

pub(crate) async fn execute_logout_cloud_relay_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: LogoutCloudRelayRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = config_projection.snapshot().cloud_relay;
    if let Some(profile) = profile.as_ref() {
        let _ = post_cloud_json::<serde_json::Value>(
            profile.api_url.clone(),
            "/auth/logout",
            serde_json::json!({
                "sessionToken": profile.cloud_session_token,
                "accountId": profile.account_id,
                "clientId": profile.client_id,
                "machineId": profile.machine_id,
                "revokeClient": request.revoke_client,
                "revokeMachine": request.revoke_machine,
            }),
        )
        .await;
    }
    clear_cloud_profile(app, config_projection).await?;
    Ok(LocalDaemonResponse::CloudRelayLoggedOut)
}

pub(crate) async fn execute_pair_cloud_relay_client_request(
    app: &Arc<Mutex<DaemonApp>>,
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
    let saved = persist_cloud_profile(app, config_projection, profile).await?;
    Ok(LocalDaemonResponse::CloudRelayClientPaired {
        profile: cloud_profile_from_persisted(&saved),
    })
}

pub(crate) async fn execute_pair_cloud_relay_machine_request(
    app: &Arc<Mutex<DaemonApp>>,
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
    let saved = persist_cloud_profile(app, config_projection, profile).await?;
    Ok(LocalDaemonResponse::CloudRelayMachinePaired {
        profile: cloud_profile_from_persisted(&saved),
    })
}

pub(crate) async fn execute_connect_cloud_relay_request(
    app: &Arc<Mutex<DaemonApp>>,
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
    let saved = persist_cloud_profile(app, config_projection, profile.clone()).await?;
    {
        let mut app = app.lock().await;
        app.configure_relay(Some(profile.relay_url.clone()), Some(issued.token.clone()))?;
        app.invalidate_provider_catalog_cache();
        config_projection.update(app.config().clone());
    }
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
    app: &Arc<Mutex<DaemonApp>>,
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
                clear_cloud_profile_if_stale(app, config_projection, &error).await?;
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
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
        profile.client_id = Some(request.client_id.clone());
        profile = persist_cloud_profile(app, config_projection, profile).await?;
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
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
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

pub(crate) async fn execute_create_cloud_session_invite_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: CreateCloudSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile_with_session(config_projection)?;
    let invite = match create_cloud_session_invite(&profile, request).await {
        Ok(invite) => invite,
        Err(error) => {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    Ok(LocalDaemonResponse::CloudSessionInviteCreated { invite })
}

pub(crate) async fn execute_show_cloud_session_invite_request(
    config_projection: &DaemonConfigProjectionStore,
    request: ShowCloudSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile(config_projection)?;
    let invite = show_cloud_session_invite(&profile, request).await?;
    Ok(LocalDaemonResponse::CloudSessionInviteShown { invite })
}

pub(crate) async fn execute_accept_cloud_session_invite_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: AcceptCloudSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile_with_session(config_projection)?;
    let acceptance = match accept_cloud_session_invite(&profile, request).await {
        Ok(acceptance) => acceptance,
        Err(error) => {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    Ok(LocalDaemonResponse::CloudSessionInviteAccepted { acceptance })
}

pub(crate) async fn execute_revoke_cloud_session_invite_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: RevokeCloudSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile_with_session(config_projection)?;
    let revoked = match revoke_cloud_session_invite(&profile, request).await {
        Ok(revoked) => revoked,
        Err(error) => {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    Ok(LocalDaemonResponse::CloudSessionInviteRevoked {
        invite_id: revoked.invite_id,
        status: revoked.status,
    })
}

pub(crate) async fn execute_list_cloud_session_members_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: ListCloudSessionMembersRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile_with_session(config_projection)?;
    let listed = match list_cloud_session_members(&profile, request).await {
        Ok(listed) => listed,
        Err(error) => {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    Ok(LocalDaemonResponse::CloudSessionMembersListed {
        session_id: listed.session_id,
        members: listed.members,
    })
}

pub(crate) async fn execute_list_cloud_collaborators_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    _request: ListCloudCollaboratorsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile_with_session(config_projection)?;
    let collaborators = match list_cloud_collaborators(&profile).await {
        Ok(collaborators) => collaborators,
        Err(error) => {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    Ok(LocalDaemonResponse::CloudCollaboratorsListed { collaborators })
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

fn required_cloud_relay_profile(
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

fn required_cloud_relay_profile_with_session(
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

async fn persist_cloud_profile(
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

async fn clear_cloud_profile(
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
