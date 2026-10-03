//! Account-wide client/machine pairing admission through the Cloud session.

use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;

use super::{post_cloud_json_authenticated, CloudPairingTokenResponse};

pub(crate) async fn request_account_pairing_token(
    profile: &PersistedCloudRelayProfile,
    subject_kind: &'static str,
) -> Result<CloudPairingTokenResponse, DaemonError> {
    let session_token = profile
        .cloud_session_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "pair cloud relay identity",
            message:
                "cloud session required for account-wide pairing; run /relay cloud login first"
                    .into(),
        })?;
    post_cloud_json_authenticated(
        profile.api_url.clone(),
        "/pairing-tokens".into(),
        session_token.to_string(),
        serde_json::json!({
            "accountId": profile.account_id,
            "createdByUserId": profile.user_id,
            "subjectKind": subject_kind,
        }),
    )
    .await
}

#[cfg(test)]
#[path = "pairing_tests.rs"]
mod tests;
