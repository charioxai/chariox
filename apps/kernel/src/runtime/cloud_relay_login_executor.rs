use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::local::{
    CloudRelayLoginPoll, CloudRelayLoginPollStatus, CloudRelayLoginStart, LocalDaemonResponse,
    LogoutCloudRelayRequest, PollCloudRelayLoginRequest, StartCloudRelayLoginRequest,
};
use crate::runtime::cloud_api_client::{
    cloud_profile_from_persisted, normalize_cloud_api_url, post_cloud_acknowledged,
    post_cloud_device_poll, post_cloud_json, CloudDevicePollResponse, CloudDeviceStartResponse,
};
use crate::runtime::cloud_relay_logout::request_cloud_logout;
use crate::runtime::cloud_relay_profile_store::{clear_cloud_profile, persist_cloud_profile};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_start_cloud_relay_login_request(
    config_projection: &DaemonConfigProjectionStore,
    request: StartCloudRelayLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let api_url = normalize_cloud_api_url(&request.api_url)?;
    let config = config_projection.snapshot();
    if request
        .machine_id
        .as_deref()
        .is_some_and(|id| id != config.host_machine_id)
    {
        return Err(DaemonError::LocalTransport {
            operation: "start kernel Cloud enrollment",
            message: "machine identity differs from this kernel; use a separate root/profile"
                .into(),
        });
    }
    let response: CloudDeviceStartResponse = post_cloud_json(
        api_url.clone(),
        "/auth/device/start",
        kernel_device_enrollment_body(&config),
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
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: PollCloudRelayLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let api_url = normalize_cloud_api_url(&request.api_url)?;
    let supports_access_denied = request.supports_access_denied == Some(true);
    let response: CloudDevicePollResponse =
        post_cloud_device_poll(api_url.clone(), request.device_code, supports_access_denied)
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
        "access_denied" => CloudRelayLoginPoll {
            status: if supports_access_denied {
                CloudRelayLoginPollStatus::AccessDenied
            } else {
                CloudRelayLoginPollStatus::ExpiredToken
            },
            interval_seconds: None,
            expires_at: None,
            profile: None,
        },
        "approved" => {
            let config = config_projection.snapshot();
            let persisted = enrolled_profile(&config, api_url, response)?;
            persist_cloud_profile(runtime_state, persisted.clone()).await?;
            CloudRelayLoginPoll {
                status: CloudRelayLoginPollStatus::Approved,
                interval_seconds: None,
                expires_at: None,
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
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    request: LogoutCloudRelayRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = config_projection.snapshot().cloud_relay;
    let path = if profile
        .as_ref()
        .is_some_and(|p| p.kernel_credential.is_some())
    {
        "/kernels/unlink"
    } else {
        "/auth/logout"
    };
    request_cloud_logout(profile.as_ref(), &request, |api_url, body| {
        post_cloud_acknowledged(api_url, path, body)
    })
    .await?;
    runtime_state.configure_relay(None, None, true).await?;
    clear_cloud_profile(runtime_state).await?;
    Ok(LocalDaemonResponse::CloudRelayLoggedOut)
}

pub(crate) fn kernel_device_enrollment_body(
    config: &crate::config::DaemonConfig,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "enrollmentKind": "KERNEL", "kernelId": config.daemon_id, "machineId": config.host_machine_id,
        "publicKeyThumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
    });
    if let Some(name) = config.machine_name() {
        body["machineName"] = serde_json::json!(name);
    }
    body
}

#[cfg(test)]
mod enrollment_display_tests {
    use super::*;
    #[test]
    fn enrollment_reports_the_os_machine_name_and_no_kernel_name() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.machine_name_for_tests = Some("fixture-machine".into());
        let mut body = kernel_device_enrollment_body(&config);
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/cloud-contract/device-start-request.json"
        ))
        .unwrap();
        // The thumbprint is of a key made for the test; the file carries a stand-in of the same shape.
        assert_eq!(body["publicKeyThumbprint"].as_str().unwrap().len(), 64);
        body["publicKeyThumbprint"] = expected["publicKeyThumbprint"].clone();
        assert_eq!(
            body, expected,
            "the file is what Cloud's strict schema accepts"
        );
        assert_eq!(body["machineName"], "fixture-machine");
        assert!(body.get("machineAlias").is_none());
        assert!(body.get("kernelAlias").is_none());
        assert_eq!(body["kernelId"], config.daemon_id);
        assert_eq!(body["machineId"], config.host_machine_id);
    }
}

// MP-08 / MP-11: device flow and owner-managed tickets share identity/ownership admission.
pub(crate) fn enrolled_profile(
    config: &crate::config::DaemonConfig,
    api_url: String,
    response: CloudDevicePollResponse,
) -> Result<PersistedCloudRelayProfile, DaemonError> {
    if response.status != "approved" {
        return Err(DaemonError::LocalTransport {
            operation: "complete kernel Cloud enrollment",
            message: "kernel enrollment was not approved".into(),
        });
    }
    let profile = response
        .profile
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "poll cloud relay login",
            message: "cloud approval response did not include a profile".to_string(),
        })?;
    let kernel_credential = response
        .kernel_credential
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "poll cloud relay login",
            message: "cloud approval response did not include a kernel credential".to_string(),
        })?;
    if profile.kernel_id.as_deref() != Some(config.daemon_id.as_str())
        || profile.machine_id.as_deref() != Some(config.host_machine_id.as_str())
        || profile.public_key_thumbprint.as_deref()
            != Some(
                crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)
                    .as_str(),
            )
    {
        return Err(DaemonError::LocalTransport {
            operation: "complete kernel Cloud enrollment",
            message: "approval does not match this kernel identity and key".into(),
        });
    }
    if config
        .cloud_relay
        .as_ref()
        .is_some_and(|old| old.account_id != profile.account_id)
    {
        return Err(DaemonError::LocalTransport { operation: "complete kernel Cloud enrollment", message: "Cloud account conflicts with this root; unlink first or use a separate root/profile".into() });
    }
    Ok(PersistedCloudRelayProfile {
        api_url,
        email: profile.email,
        account_id: profile.account_id,
        user_id: profile.user_id,
        account_slug: profile.account_slug,
        realm_id: profile.realm_id,
        relay_url: profile.relay_url,
        issuer_id: profile.issuer_id,
        client_id: None,
        client_alias: None,
        machine_id: profile.machine_id,
        machine_credential: None,
        kernel_id: profile.kernel_id,
        kernel_credential: Some(kernel_credential),
        kernel_public_key_thumbprint: profile.public_key_thumbprint,
        cloud_session_token: None,
        cloud_session_expires_at_ms: None,
        token_expires_at_ms: None,
    })
}

#[cfg(test)]
mod shared_enrollment_tests {
    use super::*;
    #[test]
    fn byom_mp08_mp11_ticket_and_device_enrollment_share_key_machine_kernel_and_owner_checks() {
        let config = crate::config::DaemonConfig::new("new-kernel", "new-machine", "owner");
        let thumbprint =
            crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key);
        let response = serde_json::json!({"status":"approved","kernelCredential":"fixture-independent-credential","profile":{"email":"fixture@example.test","accountId":"owner","userId":"owner","accountSlug":"owner","realmId":"owner","relayUrl":"wss://relay.example.test","issuerId":"fixture","kernelId":config.daemon_id,"machineId":config.host_machine_id,"publicKeyThumbprint":thumbprint}});
        let admit = |value| {
            enrolled_profile(
                &config,
                "https://cloud.example.test".into(),
                serde_json::from_value(value).unwrap(),
            )
        };
        assert!(admit(response.clone()).is_ok());
        for field in ["kernelId", "machineId", "publicKeyThumbprint"] {
            let mut value = response.clone();
            value["profile"][field] = serde_json::json!("foreign");
            assert!(admit(value).is_err());
        }
        let mut value = response.clone();
        value["kernelCredential"] = serde_json::json!("");
        assert!(admit(value).is_err());
        let mut value = response;
        value["status"] = serde_json::json!("authorization_pending");
        assert!(admit(value).is_err());
    }
}
