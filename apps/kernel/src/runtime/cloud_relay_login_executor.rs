use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::local::{
    CloudRelayLoginPoll, CloudRelayLoginPollStatus, CloudRelayLoginStart, LocalDaemonResponse,
    LogoutCloudRelayRequest, PollCloudRelayLoginRequest, StartCloudRelayLoginRequest,
};
use crate::runtime::cloud_api_client::{
    cloud_profile_from_persisted, normalize_cloud_api_url, post_cloud_json,
    CloudDevicePollResponse, CloudDeviceStartResponse,
};
use crate::runtime::cloud_relay_profile_store::{clear_cloud_profile, persist_cloud_profile};
use crate::runtime::projection::DaemonConfigProjectionStore;

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
