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
        if path == "history.operational.enabled" {
            self.operational_history
                .set_capture_enabled(self.config.user_config.history.operational.enabled);
        }
        self.config_projection.update(self.config.clone());
        Ok(())
    }
}
