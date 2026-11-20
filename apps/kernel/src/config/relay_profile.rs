use super::{
    persisted_daemon::update_persisted_daemon_config, DaemonConfig, PersistedCloudRelayProfile,
};
use crate::error::DaemonError;

impl DaemonConfig {
    pub(crate) fn persist_cloud_relay_token(
        &mut self,
        profile: PersistedCloudRelayProfile,
        token: String,
    ) -> Result<(), DaemonError> {
        let mut updated = self.clone();
        updated.relay_url = Some(profile.relay_url.clone());
        updated.relay_token = Some(token);
        updated.cloud_relay = Some(profile);
        updated.validate()?;
        updated.persist_kernel_cloud_state()?;
        *self = updated;
        Ok(())
    }

    pub(crate) fn persist_unlinked_cloud_kernel(
        &mut self,
        expected: &PersistedCloudRelayProfile,
    ) -> Result<bool, DaemonError> {
        let Some(profile) = self.cloud_relay.as_ref() else {
            return Ok(false);
        };
        if profile.kernel_unlinked
            || expected.kernel_credential.is_none()
            || profile.kernel_credential != expected.kernel_credential
            || profile.account_id != expected.account_id
            || profile.realm_id != expected.realm_id
            || profile.kernel_id != expected.kernel_id
        {
            return Ok(false);
        }
        let mut updated = self.clone();
        if crate::runtime::cloud_relay_control::cloud_relay_transport_selected(self) {
            updated.relay_url = None;
            updated.relay_token = None;
        }
        updated.cloud_relay = Some(PersistedCloudRelayProfile {
            kernel_unlinked: true,
            kernel_credential: None,
            machine_credential: None,
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
            ..profile.clone()
        });
        updated.persist_kernel_cloud_state()?;
        *self = updated;
        Ok(true)
    }

    pub fn persist_relay_config(&self) -> Result<(), DaemonError> {
        if self.cloud_relay.is_some() {
            return self.persist_kernel_cloud_state();
        }
        if self.kernel_cloud_state_exists() {
            // An explicit relay override suppresses the in-memory Cloud profile.
            // Transport configuration is not an enrollment unlink operation.
            let mut retained = self.clone();
            retained.try_load_kernel_cloud_state()?;
            retained.relay_url = self.relay_url.clone();
            retained.relay_token = self.relay_token.clone();
            return retained.persist_kernel_cloud_state();
        }
        update_persisted_daemon_config("persist relay config", |persisted| {
            persisted.relay_url = self.relay_url.clone();
            persisted.relay_token = self.relay_token.clone();
            persisted.managed_slice_relay_recovery_token =
                self.managed_slice_relay_recovery_token.clone();
            persisted.managed_slice_relay_owner_public_key =
                self.managed_slice_relay_owner_public_key.clone();
            Ok(())
        })
    }

    pub fn persist_cloud_relay_profile(
        &mut self,
        profile: Option<PersistedCloudRelayProfile>,
    ) -> Result<(), DaemonError> {
        self.cloud_relay = profile.map(PersistedCloudRelayProfile::canonicalized);
        self.persist_kernel_cloud_state()?;
        // Keep a predecessor machine credential only as a pinned migration source
        // for existing sibling kernels; never retain the human Cloud session.
        update_persisted_daemon_config("retire shared Cloud session", |persisted| {
            if let Some(legacy) = persisted.cloud_relay.as_mut() {
                legacy.cloud_session_token = None;
                legacy.cloud_session_expires_at_ms = None;
                legacy.client_id = None;
                legacy.kernel_credential = None;
                legacy.kernel_id = None;
            }
            persisted.relay_token = None;
            Ok(())
        })
    }
}
