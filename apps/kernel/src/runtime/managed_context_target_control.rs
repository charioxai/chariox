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
        let workspace_path = crate::managed_context::empty::ensure_empty_managed_context_workspace(
            &config,
            &binding.context_id,
        )?;
        return Ok(LocalDaemonResponse::ManagedContextLaunchTarget {
            target: crate::local::ManagedContextLaunchTarget {
                environment_id: registration.environment_id,
                kernel_id: registration.kernel_id,
                context_id: binding.context_id,
                plan_digest: binding.plan_digest,
                development: crate::local::ManagedContextDevelopmentLaunchTarget::Empty {
                    workspace_path: workspace_path.to_string_lossy().into_owned(),
                },
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
    use crate::managed_bootstrap::ManagedKernelContextPlan;

    struct EmptyLaunchTargetTestCleanup {
        root: std::path::PathBuf,
        repository_root_env: &'static str,
        previous_repository_root: Option<std::ffi::OsString>,
    }

    impl Drop for EmptyLaunchTargetTestCleanup {
        fn drop(&mut self) {
            match self.previous_repository_root.take() {
                Some(value) => std::env::set_var(self.repository_root_env, value),
                None => std::env::remove_var(self.repository_root_env),
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

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

    #[test]
    fn empty_launch_target_uses_managed_workspace_without_publication_control_state() {
        let _lock = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-empty-launch-target-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let state_root = root.join("state");
        let managed_workspace_root = root.join("user-workspaces");
        std::fs::create_dir_all(&state_root).expect("create state root");
        std::fs::create_dir_all(&managed_workspace_root).expect("create managed workspace root");
        let repository_root_env = crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV;
        let _cleanup = EmptyLaunchTargetTestCleanup {
            root: root.clone(),
            repository_root_env,
            previous_repository_root: std::env::var_os(repository_root_env),
        };
        std::env::set_var(repository_root_env, &managed_workspace_root);

        let mut config = DaemonConfig::for_tests();
        config.daemon_id = "kernel-empty".to_string();
        config.user_config.state.path = Some(state_root.join("state.db").display().to_string());
        config.publication_control_state_root = None;
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            user_id: "cloud-user-1".to_string(),
            ..PersistedCloudRelayProfile::default()
        });
        let registration = ConfirmedManagedKernelRegistration {
            environment_id: "environment-empty".to_string(),
            machine_id: "machine-empty".to_string(),
            kernel_id: "kernel-empty".to_string(),
            context_plan: Some(ManagedKernelContextPlan::empty_for_tests("context-empty")),
        };
        let store = ManagedContextTransferStore::open(root.join("managed-context-transfers"))
            .expect("open transfer store");
        let plan = registration
            .context_plan
            .as_ref()
            .expect("context plan")
            .package_binding();
        let request = || {
            LocalDaemonRequest::GetManagedContextLaunchTarget(
                crate::local::GetManagedContextLaunchTargetRequest {
                    context_id: plan.context_id.clone(),
                    plan_digest: plan.plan_digest.clone(),
                },
            )
        };
        let response = execute_managed_context_target_request(
            config.clone(),
            Some(registration.clone()),
            store.clone(),
            "cloud-user-1",
            request(),
        )
        .expect("empty launch target");
        let LocalDaemonResponse::ManagedContextLaunchTarget { target } = response else {
            panic!("expected launch target response")
        };
        let crate::local::ManagedContextDevelopmentLaunchTarget::Empty { workspace_path } =
            target.development
        else {
            panic!("expected empty development target")
        };
        let workspace = std::path::PathBuf::from(workspace_path);
        let canonical_workspace_root =
            std::fs::canonicalize(&managed_workspace_root).expect("canonical workspace root");
        let canonical_state_root =
            std::fs::canonicalize(&state_root).expect("canonical state root");
        assert!(workspace.is_dir());
        assert!(
            workspace.starts_with(&canonical_workspace_root),
            "empty launch workspace must use the managed repository root"
        );
        assert!(
            !workspace.starts_with(&canonical_state_root),
            "empty launch workspace must remain outside protected state"
        );

        let replay = execute_managed_context_target_request(
            config,
            Some(registration),
            store,
            "cloud-user-1",
            request(),
        )
        .expect("replayed empty launch target");
        let LocalDaemonResponse::ManagedContextLaunchTarget { target } = replay else {
            panic!("expected replayed launch target response")
        };
        assert_eq!(
            target.development,
            crate::local::ManagedContextDevelopmentLaunchTarget::Empty {
                workspace_path: workspace.to_string_lossy().into_owned(),
            }
        );
    }
}
