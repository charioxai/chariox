use super::{
    persisted_daemon::{load_persisted_daemon_config, persist_daemon_config},
    DaemonConfig, PersistedCloudRelayProfile,
};
use crate::error::DaemonError;

impl DaemonConfig {
    pub fn persist_relay_config(&self) -> Result<(), DaemonError> {
        let mut persisted = load_persisted_daemon_config();
        persisted.relay_url = self.relay_url.clone();
        persisted.relay_token = self.relay_token.clone();
        // A local/self-hosted relay override changes the active transport, not
        // the user's Cloud sign-in. Keep the durable Cloud profile so browser
        // relay credentials can still renew after their short-lived token
        // expires. Explicit sign-out uses `persist_cloud_relay_profile(None)`.
        if self.cloud_relay.is_some() {
            persisted.cloud_relay = self.cloud_relay.clone();
        }
        persist_daemon_config(&persisted, "persist relay config")
    }

    pub fn persist_cloud_relay_profile(
        &mut self,
        profile: Option<PersistedCloudRelayProfile>,
    ) -> Result<(), DaemonError> {
        self.cloud_relay = profile.map(PersistedCloudRelayProfile::canonicalized);
        let mut persisted = load_persisted_daemon_config();
        persisted.relay_url = self.relay_url.clone();
        persisted.relay_token = self.relay_token.clone();
        persisted.cloud_relay = self.cloud_relay.clone();
        persist_daemon_config(&persisted, "persist cloud relay profile")
    }
}
