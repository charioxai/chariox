mod enrollment;
use crate::app::DaemonApp;
use crate::error::DaemonError;

impl DaemonApp {
    pub(crate) fn persist_cloud_relay_token(
        &mut self,
        profile: crate::config::PersistedCloudRelayProfile,
        token: String,
    ) -> Result<(), DaemonError> {
        self.config.persist_cloud_relay_token(profile, token)?;
        self.config_projection.update(self.config.clone());
        Ok(())
    }

    pub(crate) fn mark_cloud_kernel_removal(
        &mut self,
        expected: &crate::config::PersistedCloudRelayProfile,
        removal: crate::config::CloudKernelRemoval,
    ) -> Result<(), DaemonError> {
        if self
            .config
            .persist_cloud_kernel_removal(expected, removal)?
        {
            self.config_projection.update(self.config.clone());
            let sessions = self.sessions.read().list_all_sessions();
            for session in sessions {
                if session.host_daemon_id() == self.config.daemon_id
                    && session.owner_user_id() == expected.user_id
                {
                    self.record_notice(
                        session.id(),
                        None,
                        self.attachments.list_session_attachment_ids(session.id()),
                        crate::runtime::cloud_relay_control::cloud_kernel_removal_message(removal),
                    );
                }
            }
        }
        Ok(())
    }

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
        if path.starts_with("providers.") {
            self.invalidate_provider_catalog_cache();
        }
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
        if path.starts_with("providers.") {
            self.invalidate_provider_catalog_cache();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_room_defaults_announce_config_changes() {
        let mut config = crate::config::DaemonConfig::for_tests();
        let root = crate::test_temp_dir::TestTempDir::new("new-room-defaults");
        config.user_config_path = root.join("config.toml");
        let mut app = DaemonApp::bootstrap(config).unwrap();
        for (path, value) in [
            ("providers.default", "claude"),
            ("providers.model", "claude-haiku-5-5"),
            ("providers.effort", "low"),
        ] {
            let before = app.provider_catalog_projection.change_sequence();
            app.set_user_config_value(path, value).unwrap();
            assert!(
                app.provider_catalog_projection.change_sequence() > before,
                "{path} must reach every client's launch defaults"
            );
            let before = app.provider_catalog_projection.change_sequence();
            app.unset_user_config_value(path).unwrap();
            assert!(
                app.provider_catalog_projection.change_sequence() > before,
                "unsetting {path} must reach every client too"
            );
        }
    }
}
