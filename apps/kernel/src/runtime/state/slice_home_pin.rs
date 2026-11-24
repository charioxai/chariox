//! The home a slice is provisioned to. A slice has no Cloud profile, so the same-owner channel
//! stays closed to it; the provisioner-owned home pin (kernel id and key) and the home's own slice
//! records stand in for the owner check on the machine files between the two.
use super::*;
use crate::runtime::router::ManagedSliceRelayIdentity;
use chariox_relay::protocol::RelayCallerIdentity;

pub(in crate::runtime) struct ProvisionedHome {
    pub(in crate::runtime) kernel_id: String,
    pub(in crate::runtime) public_key: String,
}

/// The pinned home of a managed slice, from its provisioned owner key and the owner identity it
/// was started with or its Room binding. When both exist they must agree.
pub(in crate::runtime) fn provisioned_slice_home(
    config: &crate::config::DaemonConfig,
    identity: Option<&ManagedSliceRelayIdentity>,
) -> Option<ProvisionedHome> {
    let public_key = config.managed_slice_relay_owner_public_key.as_ref()?;
    let binding = config.room_environment_worker_binding.as_ref();
    let kernel_id = match (identity, binding) {
        (Some(identity), Some(binding))
            if identity.owner_kernel_id == binding.home_kernel_id
                && binding.home_public_key == *public_key =>
        {
            &identity.owner_kernel_id
        }
        (Some(identity), None) => &identity.owner_kernel_id,
        (None, Some(binding)) if binding.home_public_key == *public_key => &binding.home_kernel_id,
        _ => return None,
    };
    Some(ProvisionedHome {
        kernel_id: kernel_id.clone(),
        public_key: public_key.clone(),
    })
}

impl KernelRuntimeState {
    /// The home this kernel is provisioned to, when it is a managed slice.
    pub(in crate::runtime) fn slice_home(&self) -> Option<ProvisionedHome> {
        let config = self.owned.config_projection.snapshot();
        let identity =
            crate::runtime::router::managed_slice_relay_identity_from_environment(&config);
        provisioned_slice_home(&config, identity.as_ref())
    }

    /// The owner of a request for the files of this machine. A slice serves only its pinned home.
    /// A home serves a kernel of its own Cloud owner and the slices it provisioned.
    pub(crate) fn machine_file_peer_user_id(
        &self,
        identity: &RelayCallerIdentity,
        sender: &str,
        requester_public_key: &str,
    ) -> Option<String> {
        if let Some(home) = self.slice_home() {
            return (home.kernel_id == sender && home.public_key == requester_public_key)
                .then(|| identity.user_id.clone())
                .flatten();
        }
        if self.is_own_startup_file_probe(sender, requester_public_key) {
            return self.authenticated_owner_of(identity);
        }
        self.same_owner_peer_user_id(identity)
            .or_else(|| self.provisioned_slice_user_id(identity, sender))
    }

    /// A leased worker is no home kernel, so no kernel of the same owner reads its files. The
    /// worker's own managed startup check does: it reads a probe file of this machine through
    /// the relay, from this kernel's own id and key.
    fn is_own_startup_file_probe(&self, sender: &str, requester_public_key: &str) -> bool {
        let config = self.owned.config_projection.snapshot();
        config.kernel_runtime_role == crate::config::KernelRuntimeRole::RemoteLeaseWorker
            && sender == config.daemon_id
            && requester_public_key == config.relay_public_key
    }

    fn provisioned_slice_user_id(
        &self,
        identity: &RelayCallerIdentity,
        sender: &str,
    ) -> Option<String> {
        self.ensure_notification_home_role().ok()?;
        let config = self.owned.config_projection.snapshot();
        let profile = crate::runtime::cloud_relay_control::active_cloud_profile(&config)?;
        if identity.realm_id != profile.realm_id {
            return None;
        }
        let slice = self
            .owned
            .slice_store
            .resolve_by_worker_kernel_ref(sender)?;
        (slice.owner_kernel_id == config.daemon_id).then(|| profile.user_id.clone())
    }
}

#[cfg(test)]
mod tests {
    // The slice pulls what it copies from its home. It has no Cloud profile to discover the home
    // with, so the home it is provisioned to is the one kernel it can name as a source.
    #[tokio::test]
    async fn a_slice_names_its_pinned_home_as_a_transfer_source_without_cloud_discovery() {
        let mut worker = crate::config::DaemonConfig::for_tests();
        let mut home = crate::config::DaemonConfig::for_tests();
        home.daemon_id = "home-kernel".into();
        worker.cloud_relay = None;
        worker.managed_slice_relay_owner_public_key = Some(home.relay_public_key.clone());
        worker.room_environment_worker_binding =
            Some(crate::config::RoomEnvironmentWorkerBinding {
                home_kernel_id: home.daemon_id.clone(),
                home_public_key: home.relay_public_key.clone(),
                session_id: "home-session".into(),
                slice_id: "owned-slice".into(),
                provisioned_slice_id: Some("owned-slice".into()),
            });
        let app = crate::DaemonApp::bootstrap(worker).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        let runtime = router.runtime_state();
        // The transfer's source resolves to the pinned home and links to it...
        assert_eq!(
            runtime
                .resolve_peer_kernel(&home.daemon_id)
                .await
                .expect("the pinned home is a source the slice can reach"),
            Some(home.daemon_id.clone())
        );
        runtime
            .peer_files(&home.daemon_id, &home.daemon_id)
            .expect("the slice pulls from its pinned home");
        // ...and any other kernel still needs the Cloud discovery a slice does not have.
        assert!(runtime
            .resolve_peer_kernel("some-other-kernel")
            .await
            .is_err());
    }
}
