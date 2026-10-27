//! The hosted agent gateway is listed in the Cloud clients list so its owner can revoke it there.

use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;

use super::post_cloud_json;
use super::terminal_client::kernel_client_subject;

/// Registers (or brings back) the client row named by the gateway's relay subject.
/// Cloud accepts it only from this kernel's own credential.
pub(crate) async fn register_cloud_gateway_client(
    profile: &PersistedCloudRelayProfile,
    requested_client_id: &str,
    public_key_thumbprint: &str,
    alias: &str,
) -> Result<(), DaemonError> {
    let (Some(kernel_id), Some(credential)) = (
        profile.kernel_id.as_deref().filter(|id| !id.is_empty()),
        profile
            .kernel_credential
            .as_deref()
            .filter(|c| !c.is_empty()),
    ) else {
        return Err(DaemonError::LocalTransport {
            operation: "register agent gateway",
            message:
                "the agent gateway needs this kernel's own Cloud enrollment; run /relay cloud login"
                    .into(),
        });
    };
    post_cloud_json::<serde_json::Value>(
        profile.api_url.clone(),
        "/kernels/gateway-client",
        serde_json::json!({
            "accountId": profile.account_id,
            "realmId": profile.realm_id,
            "kernelId": kernel_id,
            "kernelCredential": credential,
            "clientId": kernel_client_subject(kernel_id, requested_client_id)?,
            "publicKeyThumbprint": public_key_thumbprint,
            "alias": alias,
        }),
    )
    .await
    .map(|_| ())
}
