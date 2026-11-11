//! A kernel enrolled under one machine id that now runs on a computer deriving another
//! (the id comes from the OS since Landing 8) asks Cloud to move its own enrollment.
use serde::Deserialize;

use crate::error::DaemonError;
use crate::runtime::cloud_api_client::{cloud_error_is_retryable, post_cloud_json};
use crate::runtime::cloud_relay_profile_store::persist_cloud_profile;
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RebindResponse {
    machine_id: String,
}

/// Moves the enrollment to this computer's machine id before any token is requested.
/// An unanswered request is retried by the caller's next tick; a refusal is final: the
/// kernel drops its Cloud link and the owner links it again.
pub(crate) async fn rebind_machine_if_moved(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
) -> Result<(), DaemonError> {
    let config = config_projection.snapshot();
    let Some(mut profile) = config.cloud_relay.clone() else {
        return Ok(());
    };
    let (Some(credential), Some(enrolled_machine)) = (
        profile.kernel_credential.clone(),
        profile.machine_id.as_deref(),
    ) else {
        return Ok(());
    };
    if enrolled_machine == config.host_machine_id {
        return Ok(());
    }
    let requested = post_cloud_json::<RebindResponse>(
        profile.api_url.clone(),
        "/kernels/rebind-machine",
        serde_json::json!({
            "accountId": profile.account_id,
            "realmId": profile.realm_id,
            "kernelId": config.daemon_id,
            "machineId": config.host_machine_id,
            "kernelCredential": credential,
            "publicKeyThumbprint":
                crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
        }),
    )
    .await;
    let refusal = match requested {
        Ok(response) if response.machine_id == config.host_machine_id => {
            profile.machine_id = Some(config.host_machine_id);
            persist_cloud_profile(runtime_state, profile).await?;
            return Ok(());
        }
        Ok(_) => "Cloud moved the kernel to a different machine id".to_string(),
        Err(error) if cloud_error_is_retryable(&error) => return Err(error),
        Err(error) => error.to_string(),
    };
    runtime_state
        .configure_relay_with_cloud_profile(None, None, None, false)
        .await?;
    Err(DaemonError::LocalTransport {
        operation: "rebind kernel machine",
        message: format!(
            "Cloud refused to move this kernel to this computer's machine id ({refusal}); link this kernel to Cloud again with /cloud link"
        ),
    })
}
