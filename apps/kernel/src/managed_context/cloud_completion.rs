use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::managed_context::package::ManagedContextPlanBinding;
use crate::managed_context::transfer::MAX_IMPORT_RECEIPT_BYTES;
use crate::runtime::cloud_api_client::{cloud_error_is_retryable, post_cloud_json};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompleteManagedContextResponse {
    ready: bool,
    observed_state: String,
    context_manifest_digest: String,
}

pub(crate) fn context_manifest_digest(receipt_json: &str) -> Result<String, DaemonError> {
    if receipt_json.is_empty() || receipt_json.len() > MAX_IMPORT_RECEIPT_BYTES {
        return Err(completion_error(
            "managed context receipt is empty or exceeds the configured limit",
            false,
        ));
    }
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(receipt_json.as_bytes())
    ))
}

pub(crate) async fn complete_managed_context_import(
    config: &DaemonConfig,
    registration: &ConfirmedManagedKernelRegistration,
    plan: &ManagedContextPlanBinding,
    context_manifest_digest: &str,
) -> Result<(), DaemonError> {
    validate_managed_context_completion_binding(config, registration, plan)?;
    let profile = config.cloud_relay.as_ref().ok_or_else(|| {
        completion_error("managed kernel Cloud relay profile is unavailable", false)
    })?;
    let machine_id = profile.machine_id.as_deref().ok_or_else(|| {
        completion_error(
            "managed kernel Cloud Machine identity is unavailable",
            false,
        )
    })?;
    let machine_credential = profile.machine_credential.as_deref().ok_or_else(|| {
        completion_error(
            "managed kernel Cloud Machine credential is unavailable",
            false,
        )
    })?;
    let response: CompleteManagedContextResponse = post_cloud_json(
        profile.api_url.clone(),
        "/v1/managed-kernels/context/complete",
        serde_json::json!({
            "accountId": profile.account_id,
            "environmentId": registration.environment_id,
            "machineId": machine_id,
            "kernelId": registration.kernel_id,
            "machineCredential": machine_credential,
            "contextId": plan.context_id,
            "planDigest": plan.plan_digest,
            "contextManifestDigest": context_manifest_digest,
        }),
    )
    .await
    .map_err(cloud_completion_error)?;
    if !response.ready
        || response.observed_state != "ready"
        || response.context_manifest_digest != context_manifest_digest
    {
        return Err(completion_error(
            "Cloud returned an invalid managed context completion result",
            false,
        ));
    }
    Ok(())
}

pub(crate) fn validate_managed_context_completion_binding(
    config: &DaemonConfig,
    registration: &ConfirmedManagedKernelRegistration,
    plan: &ManagedContextPlanBinding,
) -> Result<(), DaemonError> {
    let profile = config.cloud_relay.as_ref().ok_or_else(|| {
        completion_error("managed kernel Cloud relay profile is unavailable", false)
    })?;
    let machine_id = profile.machine_id.as_deref().ok_or_else(|| {
        completion_error(
            "managed kernel Cloud Machine identity is unavailable",
            false,
        )
    })?;
    if profile
        .machine_credential
        .as_deref()
        .is_none_or(str::is_empty)
    {
        return Err(completion_error(
            "managed kernel Cloud Machine credential is unavailable",
            false,
        ));
    }
    if registration.environment_id.trim().is_empty()
        || registration.machine_id != machine_id
        || registration.machine_id != config.host_machine_id
        || registration.kernel_id != config.daemon_id
        || registration
            .context_plan
            .as_ref()
            .map(|context_plan| context_plan.package_binding())
            .as_ref()
            != Some(plan)
    {
        return Err(completion_error(
            "managed context completion does not match the confirmed kernel registration",
            false,
        ));
    }
    Ok(())
}

fn cloud_completion_error(error: DaemonError) -> DaemonError {
    let retryable = cloud_error_is_retryable(&error);
    completion_error(
        format!("Cloud could not confirm the managed context import: {error}"),
        retryable,
    )
}

fn completion_error(message: impl Into<String>, retryable: bool) -> DaemonError {
    DaemonError::ManagedContext {
        code: if retryable {
            "managed_context_cloud_completion_unavailable"
        } else {
            "managed_context_cloud_completion_rejected"
        },
        operation: "complete managed context in Cloud",
        message: message.into(),
        retryable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_manifest_digest_is_prefixed_and_stable() {
        assert_eq!(
            context_manifest_digest("{\"schemaVersion\":1}").expect("manifest digest"),
            "sha256:0e9561cfb83d50990a103b3896fe249a11fe27fa28985448187f93ec12116d72"
        );
        for error in [
            context_manifest_digest("").expect_err("empty receipt"),
            context_manifest_digest(&"x".repeat(MAX_IMPORT_RECEIPT_BYTES + 1))
                .expect_err("oversized receipt"),
        ] {
            assert!(matches!(
                error,
                DaemonError::ManagedContext {
                    code: "managed_context_cloud_completion_rejected",
                    retryable: false,
                    ..
                }
            ));
        }
    }

    #[test]
    fn cloud_completion_preserves_terminal_and_transient_failures() {
        let transient = cloud_completion_error(DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 503: cloud_api_code=dependency_unavailable"
                .to_string(),
        });
        assert!(matches!(
            transient,
            DaemonError::ManagedContext {
                code: "managed_context_cloud_completion_unavailable",
                retryable: true,
                ..
            }
        ));
        let terminal = cloud_completion_error(DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 409: cloud_api_code=identity_conflict"
                .to_string(),
        });
        assert!(matches!(
            terminal,
            DaemonError::ManagedContext {
                code: "managed_context_cloud_completion_rejected",
                retryable: false,
                ..
            }
        ));
    }
}
