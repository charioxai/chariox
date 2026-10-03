use super::DaemonConfig;
use crate::error::DaemonError;

/// Provisioner-owned identity, never learned from the first relay request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomEnvironmentWorkerBinding {
    pub home_kernel_id: String,
    pub home_public_key: String,
    pub session_id: String,
    pub slice_id: String,
    // Boot configuration only; never serialized or learned from relay callers.
    pub(crate) provisioned_slice_id: Option<String>,
}

impl RoomEnvironmentWorkerBinding {
    pub(super) fn from_environment() -> Option<Self> {
        let home = std::env::var("CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID").ok();
        let key = std::env::var("CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY").ok();
        let session = std::env::var("CHARIOX_ROOM_ENVIRONMENT_SESSION_ID").ok();
        let slice = std::env::var("CHARIOX_ROOM_ENVIRONMENT_SLICE_ID").ok();
        if [&home, &key, &session, &slice]
            .iter()
            .all(|value| value.is_none())
        {
            return None;
        }
        // Preserve incomplete configuration so validation rejects it at boot.
        Some(Self {
            home_kernel_id: home.unwrap_or_default(),
            home_public_key: key.unwrap_or_default(),
            session_id: session.unwrap_or_default(),
            slice_id: slice.unwrap_or_default(),
            provisioned_slice_id: super::identity::protected_slice_identity_required()
                .then(|| std::env::var("CHARIOX_SLICE_ID").unwrap_or_default()),
        })
    }

    pub(super) fn validate(&self, machine_id: &str) -> Result<(), DaemonError> {
        let key = crate::transport::relay_crypto::decode_public_key(&self.home_public_key);
        // Protected boot has already verified the retained machine/kernel keys.
        // Its machine ID is not the legacy slice:<id> alias; the provisioner
        // supplies the slice record ID separately in both fresh and restored boots.
        let matches_slice = match self.provisioned_slice_id.as_deref() {
            Some(slice_id) => slice_id == self.slice_id,
            None => machine_id == format!("slice:{}", self.slice_id),
        };
        if [&self.home_kernel_id, &self.session_id, &self.slice_id]
            .iter()
            .any(|value| value.is_empty() || value.trim() != value.as_str())
            || !matches!(key, Ok(ref public_key)
                if crate::transport::relay_crypto::encode_public_key(public_key) == self.home_public_key)
            || !matches_slice
        {
            return Err(DaemonError::InvalidConfig {
                field: "room_environment_worker_binding",
                message:
                    "requires a home kernel, public key, Room, and matching provisioned slice ID or legacy machine alias",
            });
        }
        Ok(())
    }

    pub(crate) fn permits(
        &self,
        kernel_id: &str,
        public_key: &str,
        session_id: &str,
        slice_id: &str,
    ) -> bool {
        self.home_kernel_id == kernel_id
            && self.home_public_key == public_key
            && self.session_id == session_id
            && self.slice_id == slice_id
    }
}

impl DaemonConfig {
    /// The same recorded relay selection is used by agents and Room controllers.
    pub(crate) fn slice_relay_override(&self, slice: &crate::slice::SliceRecord) -> Option<Self> {
        let mut config = self.clone();
        if let Some(endpoint) = slice.relay_endpoint.as_ref() {
            if !endpoint.private && self.relay_url_uses_cloud_profile(&endpoint.url) {
                return None;
            }
            config.relay_url = Some(endpoint.url.clone());
            if endpoint.private {
                config.relay_token = Some(crate::slice::local_docker_private_relay_token(slice));
            }
        } else {
            let relay = crate::slice::local_docker_private_relay(slice);
            config.relay_url = Some(relay.relay_url);
            config.relay_token = Some(relay.relay_token);
        }
        config.cloud_relay = None;
        Some(config)
    }
}
