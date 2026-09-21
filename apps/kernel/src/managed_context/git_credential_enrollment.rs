use chariox_relay::auth::RelaySubjectKind;
use serde::Deserialize;

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::managed_bootstrap::{ConfirmedManagedKernelRegistration, ManagedKernelContextPlan};
use crate::managed_context::package::ManagedContextPlanBinding;
use crate::runtime::cloud_api_client::{cloud_error_is_retryable, post_cloud_json};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthorizeGitCredentialEnrollmentResponse {
    authorized: bool,
    context_plan: ManagedKernelContextPlan,
}

pub(crate) struct GitCredentialEnrollmentSource<'a> {
    pub(crate) subject: &'a str,
    pub(crate) subject_kind: RelaySubjectKind,
    pub(crate) kernel_id: &'a str,
    pub(crate) key_thumbprint: &'a str,
    pub(crate) owner_user_id: &'a str,
    pub(crate) realm_id: &'a str,
}

pub(crate) async fn authorize_git_credential_enrollment(
    config: &DaemonConfig,
    registration: &ConfirmedManagedKernelRegistration,
    source: GitCredentialEnrollmentSource<'_>,
    context_id: &str,
    plan_digest: &str,
) -> Result<ManagedContextPlanBinding, DaemonError> {
    let profile = config.cloud_relay.as_ref().ok_or_else(|| {
        enrollment_error("managed kernel Cloud relay profile is unavailable", false)
    })?;
    let machine_id = profile.machine_id.as_deref().ok_or_else(|| {
        enrollment_error(
            "managed kernel Cloud Machine identity is unavailable",
            false,
        )
    })?;
    let machine_credential = profile.machine_credential.as_deref().ok_or_else(|| {
        enrollment_error(
            "managed kernel Cloud Machine credential is unavailable",
            false,
        )
    })?;
    let launch_plan = registration.context_plan.as_ref().ok_or_else(|| {
        enrollment_error("managed kernel registration has no context plan", false)
    })?;
    if !launch_plan.is_empty()
        || registration.environment_id.trim().is_empty()
        || registration.machine_id != machine_id
        || registration.machine_id != config.host_machine_id
        || registration.kernel_id != config.daemon_id
        || profile.realm_id != source.realm_id
        || profile.user_id != source.owner_user_id
    {
        return Err(enrollment_error(
            "Git credential enrollment does not match an opted-out managed kernel",
            false,
        ));
    }
    let source_kind = match source.subject_kind {
        RelaySubjectKind::Kernel => "kernel",
        RelaySubjectKind::Machine => "machine",
        _ => {
            return Err(enrollment_error(
                "Git credential enrollment requires a kernel or machine source",
                false,
            ))
        }
    };
    let response: AuthorizeGitCredentialEnrollmentResponse = post_cloud_json(
        profile.api_url.clone(),
        "/v1/managed-kernels/git-credential-enrollment/authorize",
        serde_json::json!({
            "accountId": profile.account_id,
            "environmentId": registration.environment_id,
            "machineId": machine_id,
            "kernelId": registration.kernel_id,
            "machineCredential": machine_credential,
            "contextId": context_id,
            "planDigest": plan_digest,
            "sourceSubject": source.subject,
            "sourceSubjectKind": source_kind,
            "sourceKernelId": source.kernel_id,
            "sourceKeyThumbprint": source.key_thumbprint,
            "sourceOwnerUserId": source.owner_user_id,
            "sourceRealmId": source.realm_id,
        }),
    )
    .await
    .map_err(enrollment_cloud_error)?;
    validate_authorized_plan(response, source, context_id, plan_digest)
}

fn validate_authorized_plan(
    response: AuthorizeGitCredentialEnrollmentResponse,
    source: GitCredentialEnrollmentSource<'_>,
    context_id: &str,
    plan_digest: &str,
) -> Result<ManagedContextPlanBinding, DaemonError> {
    let plan = response.context_plan;
    let source_matches = plan.source_binding().is_some_and(|binding| {
        let subject_matches = match source.subject_kind {
            RelaySubjectKind::Kernel => source.subject == binding.kernel_id,
            RelaySubjectKind::Machine => source.subject == binding.machine_id,
            _ => false,
        };
        subject_matches
            && binding.kernel_id == source.kernel_id
            && binding.key_thumbprint == source.key_thumbprint
            && binding.relay_realm_id == source.realm_id
    });
    if !response.authorized
        || plan.validate().is_err()
        || !plan.is_git_credential_enrollment()
        || plan.context_id() != context_id
        || plan.package_binding().plan_digest != plan_digest
        || !source_matches
    {
        return Err(enrollment_error(
            "Cloud returned an invalid Git credential enrollment authorization",
            false,
        ));
    }
    let binding = plan.package_binding();
    crate::managed_context::scm::validate_selection(&binding.git_credentials)?;
    Ok(binding)
}

pub(crate) async fn complete_git_credential_enrollment(
    config: &DaemonConfig,
    registration: &ConfirmedManagedKernelRegistration,
    plan: &ManagedContextPlanBinding,
    context_manifest_digest: &str,
) -> Result<(), DaemonError> {
    let profile = config.cloud_relay.as_ref().ok_or_else(|| {
        enrollment_error("managed kernel Cloud relay profile is unavailable", false)
    })?;
    let machine_id = profile.machine_id.as_deref().ok_or_else(|| {
        enrollment_error(
            "managed kernel Cloud Machine identity is unavailable",
            false,
        )
    })?;
    let machine_credential = profile.machine_credential.as_deref().ok_or_else(|| {
        enrollment_error(
            "managed kernel Cloud Machine credential is unavailable",
            false,
        )
    })?;
    if !plan.is_git_credential_enrollment()
        || !registration
            .context_plan
            .as_ref()
            .is_some_and(ManagedKernelContextPlan::is_empty)
        || registration.machine_id != machine_id
        || registration.machine_id != config.host_machine_id
        || registration.kernel_id != config.daemon_id
    {
        return Err(enrollment_error(
            "Git credential enrollment completion binding is invalid",
            false,
        ));
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CompletionResponse {
        completed: bool,
        context_manifest_digest: String,
    }
    let response: CompletionResponse = post_cloud_json(
        profile.api_url.clone(),
        "/v1/managed-kernels/git-credential-enrollment/complete",
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
    .map_err(enrollment_cloud_error)?;
    if !response.completed || response.context_manifest_digest != context_manifest_digest {
        return Err(enrollment_error(
            "Cloud returned an invalid Git credential enrollment completion",
            false,
        ));
    }
    Ok(())
}

fn enrollment_cloud_error(error: DaemonError) -> DaemonError {
    let retryable = cloud_error_is_retryable(&error);
    enrollment_error(
        format!("Cloud could not authorize Git credential enrollment: {error}"),
        retryable,
    )
}

fn enrollment_error(message: impl Into<String>, retryable: bool) -> DaemonError {
    DaemonError::ManagedContext {
        code: if retryable {
            "managed_git_credential_enrollment_unavailable"
        } else {
            "managed_git_credential_enrollment_rejected"
        },
        operation: "enroll managed Git credential",
        message: message.into(),
        retryable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_authorization_accepts_only_the_exact_git_only_source_binding() {
        let thumbprint = "a".repeat(64);
        let plan = ManagedKernelContextPlan::git_credential_enrollment_for_tests(
            "managed_ctx_enroll",
            "realm-1",
            "source-kernel",
            &thumbprint,
        );
        let plan_digest = plan.package_binding().plan_digest;
        let valid = || GitCredentialEnrollmentSource {
            subject: "source-kernel",
            subject_kind: RelaySubjectKind::Kernel,
            kernel_id: "source-kernel",
            key_thumbprint: &thumbprint,
            owner_user_id: "user-1",
            realm_id: "realm-1",
        };
        let binding = validate_authorized_plan(
            AuthorizeGitCredentialEnrollmentResponse {
                authorized: true,
                context_plan: plan.clone(),
            },
            valid(),
            "managed_ctx_enroll",
            &plan_digest,
        )
        .expect("valid enrollment authorization");
        assert!(binding.is_git_credential_enrollment());

        for response in [
            AuthorizeGitCredentialEnrollmentResponse {
                authorized: false,
                context_plan: plan.clone(),
            },
            AuthorizeGitCredentialEnrollmentResponse {
                authorized: true,
                context_plan: ManagedKernelContextPlan::source_project_for_tests(
                    "managed_ctx_enroll",
                    "realm-1",
                    "source-kernel",
                    &thumbprint,
                    "project-1",
                ),
            },
        ] {
            assert!(validate_authorized_plan(
                response,
                valid(),
                "managed_ctx_enroll",
                &plan_digest,
            )
            .is_err());
        }
    }
}
