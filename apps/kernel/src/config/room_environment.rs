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

    pub(super) fn validate(&self, kernel_id: &str, machine_id: &str) -> Result<(), DaemonError> {
        let key = crate::transport::relay_crypto::decode_public_key(&self.home_public_key);
        // Protected boot has already verified the retained machine/kernel keys.
        // Its machine ID is not the legacy slice:<id> alias; the provisioner
        // supplies the slice record ID separately in both fresh and restored boots.
        // Unprotected workers keep the private slice:<id> alias, or a hosted
        // worker ref scoped to its authenticated parent Machine.
        let matches_slice = match self.provisioned_slice_id.as_deref() {
            Some(slice_id) => slice_id == self.slice_id,
            None => {
                machine_id == format!("slice:{}", self.slice_id)
                    || crate::slice::machine_scoped_slice_worker_ref(kernel_id, machine_id)
            }
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
                    "requires a home kernel, public key, Room, and matching provisioned slice ID, private slice alias, or machine-scoped slice identity",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bound_config() -> DaemonConfig {
        let mut config = DaemonConfig::for_tests();
        config.room_environment_worker_binding = Some(RoomEnvironmentWorkerBinding {
            provisioned_slice_id: None,
            home_kernel_id: "home-kernel".into(),
            home_public_key: config.relay_public_key.clone(),
            session_id: "room-a".into(),
            slice_id: "local-slice".into(),
        });
        config
    }

    fn hosted_config() -> DaemonConfig {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/slice-worker-identity.json"
        )))
        .unwrap();
        let case = &vectors["cases"][0];
        let mut config = bound_config();
        config.daemon_id = case["workerKernelRef"].as_str().unwrap().into();
        config.host_machine_id = case["machineId"].as_str().unwrap().into();
        config
    }

    #[test]
    fn room_worker_binding_accepts_hosted_canonical_parent_machine() {
        hosted_config()
            .validate()
            .expect("hosted worker uses its authenticated parent Machine");
    }

    #[test]
    fn room_worker_binding_rejects_foreign_machine_and_unqualified_kernel() {
        for (kernel, machine) in [
            (None, "foreign-machine"),
            (Some("slice:drill"), "machine-a"),
            (Some("ordinary-kernel"), "machine-a"),
        ] {
            let mut config = hosted_config();
            if let Some(kernel) = kernel {
                config.daemon_id = kernel.into();
            }
            config.host_machine_id = machine.into();
            assert!(matches!(
                config.validate(),
                Err(DaemonError::InvalidConfig {
                    field: "room_environment_worker_binding",
                    ..
                })
            ));
        }
    }

    #[test]
    fn room_worker_binding_preserves_private_synthetic_machine() {
        let mut config = bound_config();
        config.daemon_id = "private-worker".into();
        config.host_machine_id = "slice:local-slice".into();
        config
            .validate()
            .expect("private workers keep their synthetic Machine identity");
        config.host_machine_id = "slice:another-slice".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn room_worker_binding_keeps_exact_home_room_and_slice_authority() {
        let mut config = hosted_config();
        let binding = config.room_environment_worker_binding.as_ref().unwrap();
        assert!(binding.permits(
            "home-kernel",
            &config.relay_public_key,
            "room-a",
            "local-slice"
        ));
        assert!(!binding.permits(
            "other-home",
            &config.relay_public_key,
            "room-a",
            "local-slice"
        ));
        assert!(!binding.permits(
            "home-kernel",
            &config.relay_public_key,
            "room-b",
            "local-slice"
        ));
        assert!(!binding.permits(
            "home-kernel",
            &config.relay_public_key,
            "room-a",
            "other-slice"
        ));
        config
            .room_environment_worker_binding
            .as_mut()
            .unwrap()
            .home_public_key
            .clear();
        assert!(config.validate().is_err());
    }
}
