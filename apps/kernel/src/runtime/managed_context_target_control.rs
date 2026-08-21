use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::managed_context::transfer::ManagedContextTransferStore;

pub(crate) fn execute_managed_context_target_request(
    config: DaemonConfig,
    registration: Option<ConfirmedManagedKernelRegistration>,
    store: ManagedContextTransferStore,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    authorize_managed_kernel_owner(&config, caller_user_id)?;
    let registration = registration.ok_or_else(|| {
        target_error(
            "managed context launch target is available only on a confirmed managed kernel",
        )
    })?;
    let LocalDaemonRequest::GetManagedContextLaunchTarget(request) = request else {
        return Err(DaemonError::LocalTransport {
            operation: "managed context target control",
            message: "unsupported request".to_string(),
        });
    };
    let plan = registration
        .context_plan
        .ok_or_else(|| target_error("managed kernel has no confirmed context plan"))?;
    let binding = plan.package_binding();
    if request.context_id != binding.context_id || request.plan_digest != binding.plan_digest {
        return Err(target_error(
            "managed context launch target request does not match the confirmed plan",
        ));
    }
    if matches!(
        binding.development,
        crate::managed_context::package::ManagedContextDevelopmentSelection::Empty
    ) {
        return Ok(LocalDaemonResponse::ManagedContextLaunchTarget {
            target: crate::local::ManagedContextLaunchTarget {
                environment_id: registration.environment_id,
                kernel_id: registration.kernel_id,
                context_id: binding.context_id,
                plan_digest: binding.plan_digest,
                development: crate::local::ManagedContextDevelopmentLaunchTarget::Empty,
            },
        });
    }
    let target = store.launch_target(&request.context_id, &request.plan_digest)?;
    if target.environment_id != registration.environment_id
        || target.kernel_id != registration.kernel_id
    {
        return Err(target_error(
            "managed context launch target does not match the confirmed kernel",
        ));
    }
    Ok(LocalDaemonResponse::ManagedContextLaunchTarget { target })
}

fn authorize_managed_kernel_owner(
    config: &DaemonConfig,
    caller_user_id: &str,
) -> Result<(), DaemonError> {
    let owner_user_id = config
        .cloud_relay
        .as_ref()
        .map(|profile| profile.user_id.as_str())
        .filter(|user_id| !user_id.is_empty())
        .ok_or_else(|| target_error("managed kernel has no Cloud owner binding"))?;
    if caller_user_id != owner_user_id {
        return Err(DaemonError::ManagedContext {
            code: "unauthorized",
            operation: "get managed context launch target",
            message: "managed context launch target belongs to another Cloud user".to_string(),
            retryable: false,
        });
    }
    Ok(())
}

fn target_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_launch_target_unavailable",
        operation: "get managed context launch target",
        message: message.into(),
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PersistedCloudRelayProfile;

    #[test]
    fn launch_target_requires_exact_cloud_owner() {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            user_id: "cloud-user-1".to_string(),
            ..PersistedCloudRelayProfile::default()
        });

        authorize_managed_kernel_owner(&config, "cloud-user-1").expect("Cloud owner");
        assert!(authorize_managed_kernel_owner(&config, "cloud-user-2").is_err());
        assert!(
            authorize_managed_kernel_owner(&config, crate::session::DEFAULT_LOCAL_USER_ID).is_err()
        );
    }
}
