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
        persisted.cloud_relay = self.cloud_relay.clone();
        persist_daemon_config(&persisted, "persist relay config")
    }

    pub fn persist_cloud_relay_profile(
        &mut self,
        profile: Option<PersistedCloudRelayProfile>,
    ) -> Result<(), DaemonError> {
        self.cloud_relay = profile;
        self.persist_relay_config()
    }
}
