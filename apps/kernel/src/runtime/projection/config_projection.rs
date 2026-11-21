use crate::config::{DaemonConfig, EventGeneratorManagementTarget};
use crate::error::DaemonError;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

#[derive(Clone)]
pub(crate) struct DaemonConfigProjectionStore {
    config: Arc<StdMutex<ProjectedConfig>>,
}

struct ProjectedConfig {
    base: DaemonConfig,
    renewal: Option<CloudRelayRenewal>,
}

struct CloudRelayRenewal {
    token: String,
    expires_at_ms: u64,
}

impl ProjectedConfig {
    fn snapshot(&self) -> DaemonConfig {
        let mut config = self.base.clone();
        if let (Some(renewal), Some(profile)) = (&self.renewal, config.cloud_relay.as_mut()) {
            config.relay_url = Some(profile.relay_url.clone());
            config.relay_token = Some(renewal.token.clone());
            profile.token_expires_at_ms = Some(renewal.expires_at_ms);
        }
        config
    }
}

fn same_relay_configuration(left: &DaemonConfig, right: &DaemonConfig) -> bool {
    left.daemon_id == right.daemon_id
        && left.host_machine_id == right.host_machine_id
        && left.relay_public_key == right.relay_public_key
        && left.relay_url == right.relay_url
        && left.relay_token == right.relay_token
        && left.cloud_relay == right.cloud_relay
}

impl DaemonConfigProjectionStore {
    pub(crate) fn new(config: DaemonConfig) -> Self {
        Self {
            config: Arc::new(StdMutex::new(ProjectedConfig {
                base: config,
                renewal: None,
            })),
        }
    }

    pub(crate) fn snapshot(&self) -> DaemonConfig {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .snapshot()
    }

    pub(crate) fn set_runtime_mcp_port(&self, port: u16) {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .base
            .runtime_mcp_port = port;
    }

    pub(crate) fn local_owner_user_id(&self) -> String {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .base
            .cloud_relay
            .as_ref()
            .map(|profile| profile.user_id.clone())
            .unwrap_or_else(|| crate::session::DEFAULT_LOCAL_USER_ID.into())
    }

    /// Pre-enrollment rooms keep their member identity after the owner enrolls.
    pub(crate) fn room_member_user_id_in(&self, room_owner: &str, user_id: &str) -> String {
        room_member_user_id_in(
            &self
                .config
                .lock()
                .expect("daemon config projection lock")
                .base,
            room_owner,
            user_id,
        )
    }

    pub(crate) fn workflow_invoker_matches(
        &self,
        kernel_id: &str,
        room_owner: &str,
        user_id: &str,
        invoker: &str,
    ) -> bool {
        let config = self.config.lock().expect("daemon config projection lock");
        let config = &config.base;
        config.daemon_id == kernel_id
            && room_member_user_id_in(config, room_owner, user_id)
                == room_member_user_id_in(config, room_owner, invoker)
    }

    /// The enrolled owner, or the `local` identity this owner's pre-enrollment rooms keep: the
    /// terminal reaches those under `local` for membership.
    pub(crate) fn is_kernel_owner(&self, kernel_id: &str, user_id: &str) -> bool {
        let config = self.config.lock().expect("daemon config projection lock");
        let config = &config.base;
        config.daemon_id == kernel_id
            && (user_id == crate::session::DEFAULT_LOCAL_USER_ID
                || config
                    .cloud_relay
                    .as_ref()
                    .is_some_and(|profile| profile.user_id == user_id))
    }

    pub(crate) fn update(&self, config: DaemonConfig) {
        let mut projected = self.config.lock().expect("daemon config projection lock");
        if !same_relay_configuration(&projected.base, &config) {
            projected.renewal = None;
        }
        projected.base = config;
    }

    /// An issued token is replaceable on restart with the durable enrollment.
    /// Apply it directly so neither the app lock nor its disk writes gate reauthentication.
    pub(crate) fn install_cloud_relay_renewal(
        &self,
        previous: &DaemonConfig,
        token: String,
        expires_at_ms: u64,
    ) -> Result<(), DaemonError> {
        let mut projected = self.config.lock().expect("daemon config projection lock");
        if projected.base.cloud_relay.is_none()
            || !same_relay_configuration(&projected.snapshot(), previous)
        {
            return Err(DaemonError::LocalTransport {
                operation: "install renewed relay token",
                message: "relay configuration changed during renewal; retry with the current configuration".into(),
            });
        }
        if expires_at_ms <= crate::session::unix_epoch_ms() {
            return Err(DaemonError::LocalTransport {
                operation: "install renewed relay token",
                message: "renewed relay token expired before installation; request another token"
                    .into(),
            });
        }
        projected.renewal = Some(CloudRelayRenewal {
            token,
            expires_at_ms,
        });
        Ok(())
    }

    /// Remember the management URL the registry gave for a generator, keeping any target the
    /// operator configured.
    pub(crate) fn merge_event_generator_management_target(
        &self,
        generator_id: &str,
        resolved: EventGeneratorManagementTarget,
    ) {
        self.config
            .lock()
            .expect("daemon config projection lock should not be poisoned")
            .base
            .event_generator_management_targets
            .entry(generator_id.to_string())
            .or_insert(resolved);
    }
}

fn room_member_user_id_in(config: &DaemonConfig, room_owner: &str, user_id: &str) -> String {
    if room_owner == crate::session::DEFAULT_LOCAL_USER_ID
        && !user_id.is_empty()
        && config
            .cloud_relay
            .as_ref()
            .is_some_and(|profile| profile.user_id == user_id)
    {
        crate::session::DEFAULT_LOCAL_USER_ID.into()
    } else {
        user_id.into()
    }
}
