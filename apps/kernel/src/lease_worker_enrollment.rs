//! Context import on a lease worker uses its configured home identity and owner enrollment.
use crate::config::{DaemonConfig, KernelRuntimeRole, LeaseWorkerHomeCaller};
use crate::error::DaemonError;

pub(crate) fn enrolled_lease_worker_home_caller(
    config: &DaemonConfig,
) -> Result<Option<LeaseWorkerHomeCaller>, DaemonError> {
    if config.kernel_runtime_role != KernelRuntimeRole::RemoteLeaseWorker {
        return Ok(None);
    }
    let denied = || DaemonError::KernelRuntimeRoleDenied {
        role: config.kernel_runtime_role.as_str(),
        operation: "managed_context.import",
    };
    let home = config
        .lease_worker_home_caller
        .as_ref()
        .ok_or_else(denied)?;
    let profile = config.cloud_relay.as_ref().ok_or_else(denied)?;
    if !config.accept_remote_leases
        || config.remote_lease_capacity != Some(1)
        || home.realm_id != profile.realm_id
        || home.user_id != profile.user_id
        || profile.machine_id.as_deref() != Some(config.host_machine_id.as_str())
        || profile
            .machine_credential
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Err(denied());
    }
    Ok(Some(home.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PersistedCloudRelayProfile;

    #[test]
    fn context_import_requires_the_configured_home_and_owner_enrollment() {
        let mut config = DaemonConfig::for_tests();
        assert!(enrolled_lease_worker_home_caller(&config)
            .unwrap()
            .is_none());
        config.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
        assert!(enrolled_lease_worker_home_caller(&config).is_err());
        config.accept_remote_leases = true;
        config.remote_lease_capacity = Some(1);
        let home = LeaseWorkerHomeCaller {
            kernel_id: "home-kernel".into(),
            realm_id: "owner-realm".into(),
            user_id: "owner-user".into(),
            relay_public_key: "home-key".into(),
        };
        config.lease_worker_home_caller = Some(home.clone());
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            realm_id: home.realm_id.clone(),
            user_id: home.user_id.clone(),
            machine_id: Some(config.host_machine_id.clone()),
            machine_credential: Some("synthetic-enrollment-credential".into()),
            ..PersistedCloudRelayProfile::default()
        });
        assert_eq!(
            enrolled_lease_worker_home_caller(&config).unwrap(),
            Some(home)
        );
        config.cloud_relay.as_mut().unwrap().user_id = "another-user".into();
        assert!(enrolled_lease_worker_home_caller(&config).is_err());
        config.cloud_relay.as_mut().unwrap().user_id = "owner-user".into();
        config.cloud_relay.as_mut().unwrap().realm_id = "another-realm".into();
        assert!(enrolled_lease_worker_home_caller(&config).is_err());
        config.cloud_relay.as_mut().unwrap().realm_id = "owner-realm".into();
        config.remote_lease_capacity = Some(2);
        assert!(enrolled_lease_worker_home_caller(&config).is_err());
    }
}
