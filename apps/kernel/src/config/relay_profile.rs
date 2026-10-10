use super::{
    persisted_daemon::{load_persisted_daemon_config, persist_daemon_config},
    DaemonConfig, PersistedCloudRelayProfile,
};
use crate::error::DaemonError;

impl DaemonConfig {
    pub fn persist_relay_config(&self) -> Result<(), DaemonError> {
        if self.cloud_relay.is_some() || self.kernel_cloud_state_exists() {
            return self.persist_kernel_cloud_state();
        }
        let mut persisted = load_persisted_daemon_config();
        persisted.relay_url = self.relay_url.clone();
        persisted.relay_token = self.relay_token.clone();
        persisted.managed_slice_relay_recovery_token =
            self.managed_slice_relay_recovery_token.clone();
        persisted.managed_slice_relay_owner_public_key =
            self.managed_slice_relay_owner_public_key.clone();
        persist_daemon_config(&persisted, "persist relay config")
    }

    pub fn persist_cloud_relay_profile(
        &mut self,
        profile: Option<PersistedCloudRelayProfile>,
    ) -> Result<(), DaemonError> {
        self.cloud_relay = profile.map(PersistedCloudRelayProfile::canonicalized);
        self.persist_kernel_cloud_state()?;
        // Keep a predecessor machine credential only as a pinned migration source
        // for existing sibling kernels; never retain the human Cloud session.
        let mut persisted = load_persisted_daemon_config();
        if let Some(legacy) = persisted.cloud_relay.as_mut() {
            legacy.cloud_session_token = None;
            legacy.cloud_session_expires_at_ms = None;
            legacy.client_id = None;
            legacy.kernel_credential = None;
            legacy.kernel_id = None;
        }
        persisted.relay_token = None;
        persist_daemon_config(&persisted, "retire shared Cloud session")
    }
}
