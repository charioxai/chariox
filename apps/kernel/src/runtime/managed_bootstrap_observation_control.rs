use chrono::Utc;

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, ManagedEnvironmentPreReimageObservationAcknowledgement,
    ObserveManagedEnvironmentPreReimageRequest,
};
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;

pub(crate) async fn execute_managed_bootstrap_observation_request(
    config: DaemonConfig,
    registration: Option<ConfirmedManagedKernelRegistration>,
    caller_user_id: &str,
    request: ObserveManagedEnvironmentPreReimageRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    authorize_observation_caller(&config, caller_user_id)?;
    let registration = registration
        .ok_or_else(|| observation_error("kernel is not a confirmed managed environment"))?;
    let acknowledgement = tokio::task::spawn_blocking(move || {
        crate::managed_bootstrap::observe_managed_environment_pre_reimage(
            &config,
            &registration,
            &request.environment_id,
            request.expected_generation,
            Utc::now(),
        )
    })
    .await
    .map_err(|error| observation_error(format!("observation task failed: {error}")))??;
    Ok(LocalDaemonResponse::ManagedEnvironmentPreReimageObserved {
        acknowledgement: ManagedEnvironmentPreReimageObservationAcknowledgement {
            environment_id: acknowledgement.environment_id,
            generation: acknowledgement.generation,
            observed_at: acknowledgement.observed_at,
        },
    })
}

fn authorize_observation_caller(
    config: &DaemonConfig,
    caller_user_id: &str,
) -> Result<(), DaemonError> {
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| observation_error("kernel is not connected to Chariox Cloud"))?;
    if caller_user_id != profile.user_id {
        return Err(observation_error(
            "managed pre-reimage observation belongs to another Cloud user",
        ));
    }
    if profile.machine_credential.as_deref().is_none_or(str::is_empty) {
        return Err(observation_error(
            "managed pre-reimage observation requires a machine credential",
        ));
    }
    Ok(())
}

fn observation_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "observe managed environment before reimage",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PersistedCloudRelayProfile;

    #[test]
    fn pre_reimage_observation_requires_the_installed_cloud_owner_and_machine_credential() {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            user_id: "owner-1".to_string(),
            machine_credential: Some(format!("mcred_{}", "a".repeat(40))),
            ..PersistedCloudRelayProfile::default()
        });
        authorize_observation_caller(&config, "owner-1").expect("managed Cloud owner");
        assert!(authorize_observation_caller(&config, "owner-2").is_err());
        config
            .cloud_relay
            .as_mut()
            .expect("profile")
            .machine_credential = None;
        assert!(authorize_observation_caller(&config, "owner-1").is_err());
    }
}
