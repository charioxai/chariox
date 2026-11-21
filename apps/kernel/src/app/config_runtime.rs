mod enrollment;
use crate::app::DaemonApp;
use crate::error::DaemonError;

impl DaemonApp {
    pub(crate) fn configure_relay(
        &mut self,
        relay_url: Option<String>,
        relay_token: Option<String>,
    ) -> Result<(), DaemonError> {
        self.config.relay_url = relay_url
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.config.relay_token = relay_token
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.config.validate()?;
        self.config.persist_relay_config()?;
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    /// Renewal changes only the relay credentials of the enrollment that requested them.
    /// Re-enrolling Rooms here would make a heartbeat wait on unrelated durable work.
    pub(crate) fn install_cloud_relay_renewal(
        &mut self,
        previous: &crate::config::DaemonConfig,
        token: String,
        expires_at_ms: u64,
    ) -> Result<(), DaemonError> {
        let mut next = self.config.clone();
        let config_changed = next.relay_url != previous.relay_url
            || next.relay_token != previous.relay_token
            || next.relay_public_key != previous.relay_public_key
            || next.host_machine_id != previous.host_machine_id
            || next.cloud_relay != previous.cloud_relay;
        if config_changed {
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
        let profile = next
            .cloud_relay
            .as_mut()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "install renewed relay token",
                message: "Cloud enrollment is unavailable; link this kernel to Cloud again".into(),
            })?;
        profile.token_expires_at_ms = Some(expires_at_ms);
        next.relay_url = Some(profile.relay_url.clone());
        next.relay_token = Some(token);
        next.validate()?;
        next.persist_relay_config()?;
        self.config = next;
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    pub(crate) fn configure_managed_slice_relay(
        &mut self,
        relay_url: String,
        relay_token: String,
        recovery_token: String,
        owner_public_key: String,
    ) -> Result<(), DaemonError> {
        self.config.relay_url = Some(relay_url);
        self.config.relay_token = Some(relay_token);
        self.config.managed_slice_relay_recovery_token = Some(recovery_token);
        self.config.managed_slice_relay_owner_public_key = Some(owner_public_key);
        self.config.validate()?;
        self.config.persist_relay_config()?;
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    pub(crate) fn persist_cloud_relay_profile(
        &mut self,
        profile: Option<crate::config::PersistedCloudRelayProfile>,
    ) -> Result<(), DaemonError> {
        self.config.persist_cloud_relay_profile(profile)?;
        self.enroll_existing_local_state()?;
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    pub(crate) fn set_user_config_value(
        &mut self,
        path: impl AsRef<str>,
        value: impl Into<String>,
    ) -> Result<(), DaemonError> {
        let path = path.as_ref().trim().to_string();
        self.config.set_user_config_value(&path, value)?;
        crate::runtime::process_resources::configure(
            self.config.user_config.kernel.resource_protection,
        );
        if path == "history.operational.enabled" {
            self.operational_history
                .set_capture_enabled(self.config.user_config.history.operational.enabled);
        }
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    pub(crate) fn unset_user_config_value(
        &mut self,
        path: impl AsRef<str>,
    ) -> Result<(), DaemonError> {
        let path = path.as_ref().trim().to_string();
        self.config.unset_user_config_value(&path)?;
        crate::runtime::process_resources::configure(
            self.config.user_config.kernel.resource_protection,
        );
        if path == "history.operational.enabled" {
            self.operational_history
                .set_capture_enabled(self.config.user_config.history.operational.enabled);
        }
        self.config_projection.update(self.config.clone());
        Ok(())
    }
}
