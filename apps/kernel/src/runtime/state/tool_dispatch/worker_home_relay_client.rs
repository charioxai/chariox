use std::time::Duration;

use chariox_relay::protocol::ClientTarget;

use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime) async fn send_worker_home_runtime_request(
        &self,
        config: &crate::config::DaemonConfig,
        home_kernel_id: &str,
        request: RelayPeerRequest,
        response_timeout: Duration,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let target = ClientTarget {
            daemon_id: Some(home_kernel_id.to_string()),
            daemon_alias: None,
        };
        // MP-08/MP-10/MP-11: Computer uses the existing authenticated peer
        // connection. Every call still requires the typed HOME reply; no Room
        // binding or temporary socket creates desktop authority.
        if matches!(
            &request,
            RelayPeerRequest::SetLeasedComputerGrant { .. }
                | RelayPeerRequest::CheckLeasedComputerGrant { .. }
        ) {
            return crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                config,
                &self.owned.relay_state,
                target,
                request,
                response_timeout,
            )
            .await;
        }
        if let Some(owner_public_key) = managed_slice_home_owner_public_key(config, home_kernel_id)?
        {
            return crate::transport::relay_client::send_peer_request_to_known_kernel_via_relay_with_timeout(
                config,
                &self.owned.relay_state,
                target,
                &owner_public_key,
                request,
                response_timeout,
            )
            .await;
        }
        crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
            config,
            &self.owned.relay_state,
            target,
            request,
            response_timeout,
        )
        .await
    }
}

fn managed_slice_home_owner_public_key(
    config: &crate::config::DaemonConfig,
    home_kernel_id: &str,
) -> Result<Option<String>, DaemonError> {
    let identity = crate::runtime::router::managed_slice_relay_identity_from_environment(config);
    managed_slice_home_owner_public_key_for_identity(config, home_kernel_id, identity.as_ref())
}

fn managed_slice_home_owner_public_key_for_identity(
    config: &crate::config::DaemonConfig,
    home_kernel_id: &str,
    identity: Option<&crate::runtime::router::ManagedSliceRelayIdentity>,
) -> Result<Option<String>, DaemonError> {
    let Some(owner_public_key) = config.managed_slice_relay_owner_public_key.as_ref() else {
        return Ok(None);
    };
    let room_binding = config.room_environment_worker_binding.as_ref();
    let slice_matches = identity.is_some_and(|identity| identity.owner_kernel_id == home_kernel_id);
    let room_matches = room_binding.is_some_and(|binding| {
        binding.home_kernel_id == home_kernel_id && binding.home_public_key == *owner_public_key
    });
    // Standalone managed slices carry the same provisioner-owned identity and
    // owner pin as Room slices, without a Room session binding. When both exist
    // they must agree before any peer request enters transport.
    let binding_matches = match (identity, room_binding) {
        (Some(_), Some(_)) => slice_matches && room_matches,
        (Some(_), None) => slice_matches,
        (None, Some(_)) => room_matches,
        (None, None) => false,
    };
    if !binding_matches {
        return Err(DaemonError::LocalTransport {
            operation: "forward managed slice runtime tool",
            message: "managed slice owner identity does not match its provisioned binding"
                .to_string(),
        });
    }
    Ok(Some(owner_public_key.clone()))
}

#[cfg(test)]
#[path = "worker_home_relay_client_tests.rs"]
mod tests;
