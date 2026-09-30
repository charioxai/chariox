use std::future::Future;

use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::local::LogoutCloudRelayRequest;

pub(super) async fn request_cloud_logout<F, Fut>(
    profile: Option<&PersistedCloudRelayProfile>,
    request: &LogoutCloudRelayRequest,
    post: F,
) -> Result<(), DaemonError>
where
    F: FnOnce(String, serde_json::Value) -> Fut,
    Fut: Future<Output = Result<(), DaemonError>>,
{
    let requires_acknowledgement = request.revoke_client || request.revoke_machine;
    let Some(profile) = profile else {
        return if requires_acknowledgement {
            Err(logout_error("explicit Cloud revocation requires a linked profile"))
        } else {
            Ok(())
        };
    };
    if requires_acknowledgement {
        if missing(profile.cloud_session_token.as_deref()) {
            return Err(logout_error("Cloud revocation requires a session; run /relay cloud login first"));
        }
        if request.revoke_client && missing(profile.client_id.as_deref()) {
            return Err(logout_error("Cloud client revocation requires a linked client identity"));
        }
        if request.revoke_machine && missing(profile.machine_id.as_deref()) {
            return Err(logout_error("Cloud machine revocation requires a linked machine identity"));
        }
    }
    let acknowledged = post(
        profile.api_url.clone(),
        serde_json::json!({
            "sessionToken": profile.cloud_session_token,
            "accountId": profile.account_id,
            "clientId": profile.client_id,
            "machineId": profile.machine_id,
            "revokeClient": request.revoke_client,
            "revokeMachine": request.revoke_machine,
        }),
    ).await;
    if requires_acknowledgement { acknowledged } else { Ok(()) }
}

fn missing(value: Option<&str>) -> bool {
    value.is_none_or(|value| value.trim().is_empty())
}

fn logout_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "logout cloud relay",
        message: message.into(),
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            api_url: "https://cloud.example.invalid".into(),
            account_id: "account-fixture".into(),
            client_id: Some("client-fixture".into()),
            machine_id: Some("machine-fixture".into()),
            cloud_session_token: Some("synthetic-session-fixture".into()),
            ..Default::default()
        }
    }

    fn offline() -> DaemonError {
        DaemonError::LocalTransport {
            operation: "Cloud fixture",
            message: "acknowledgement unavailable".into(),
        }
    }

    #[tokio::test]
    async fn explicit_revocation_cannot_clear_the_profile_without_cloud_acknowledgement() {
        let profile = profile();
        for request in [
            LogoutCloudRelayRequest { revoke_client: true, revoke_machine: false },
            LogoutCloudRelayRequest { revoke_client: false, revoke_machine: true },
        ] {
            assert!(request_cloud_logout(Some(&profile), &request, |_, _| async { Err(offline()) })
                .await.is_err());
        }
    }

    #[tokio::test]
    async fn acknowledged_explicit_revocation_can_clear_the_bound_profile() {
        let profile = profile();
        let request = LogoutCloudRelayRequest { revoke_client: true, revoke_machine: true };
        request_cloud_logout(Some(&profile), &request, |api, body| async move {
            assert_eq!(api, "https://cloud.example.invalid");
            assert_eq!(body["accountId"], "account-fixture");
            assert_eq!(body["clientId"], "client-fixture");
            assert_eq!(body["machineId"], "machine-fixture");
            assert_eq!(body["revokeClient"], true);
            assert_eq!(body["revokeMachine"], true);
            Ok(())
        }).await.expect("acknowledged revocation can clear locally");
    }

    #[tokio::test]
    async fn plain_logout_can_clear_the_profile_offline() {
        request_cloud_logout(Some(&profile()), &LogoutCloudRelayRequest { revoke_client: false, revoke_machine: false },
            |_, _| async { Err(offline()) }).await.expect("ordinary logout must work offline");
    }

    #[tokio::test]
    async fn plain_logout_without_a_profile_does_not_require_cloud() {
        request_cloud_logout(None, &LogoutCloudRelayRequest { revoke_client: false, revoke_machine: false },
            |_, _| async { panic!("no Cloud request is needed"); #[allow(unreachable_code)] Ok(()) })
            .await.expect("ordinary logout is idempotent");
    }

    #[tokio::test]
    async fn explicit_revocation_without_a_profile_fails_before_cloud() {
        for request in [
            LogoutCloudRelayRequest { revoke_client: true, revoke_machine: false },
            LogoutCloudRelayRequest { revoke_client: false, revoke_machine: true },
        ] {
            assert!(request_cloud_logout(None, &request,
                |_, _| async { panic!("missing profile cannot be sent to Cloud"); #[allow(unreachable_code)] Ok(()) })
                .await.is_err());
        }
    }

    #[tokio::test]
    async fn explicit_revocation_requires_a_session_before_cloud() {
        for session in [None, Some(""), Some(" ")] {
            let mut profile = profile();
            profile.cloud_session_token = session.map(str::to_string);
            let request = LogoutCloudRelayRequest { revoke_client: false, revoke_machine: true };
            assert!(request_cloud_logout(Some(&profile), &request,
                |_, _| async { panic!("missing session cannot be sent to Cloud"); #[allow(unreachable_code)] Ok(()) })
                .await.is_err());
        }
    }

    #[tokio::test]
    async fn explicit_revocation_requires_each_requested_identity_before_cloud() {
        for identity in [None, Some(""), Some(" ")] {
            for revoke_machine in [false, true] {
                let mut profile = profile();
                if revoke_machine { profile.machine_id = identity.map(str::to_string); }
                else { profile.client_id = identity.map(str::to_string); }
                let request = LogoutCloudRelayRequest { revoke_client: !revoke_machine, revoke_machine };
                assert!(request_cloud_logout(Some(&profile), &request,
                    |_, _| async { panic!("missing requested identity cannot be sent to Cloud"); #[allow(unreachable_code)] Ok(()) })
                    .await.is_err());
            }
        }
    }

    #[tokio::test]
    async fn machine_only_revocation_does_not_require_a_client_identity() {
        let mut profile = profile();
        profile.client_id = None;
        let request = LogoutCloudRelayRequest { revoke_client: false, revoke_machine: true };
        request_cloud_logout(Some(&profile), &request, |_, _| async { Ok(()) })
            .await.expect("only the requested identity is required");
    }

}
