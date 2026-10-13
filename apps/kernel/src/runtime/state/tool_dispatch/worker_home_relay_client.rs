use std::time::Duration;

use chariox_relay::protocol::ClientTarget;

use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn send_worker_home_runtime_request(
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
        crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            config,
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
    if config.managed_slice_relay_owner_public_key.is_none() {
        return Ok(None);
    }
    crate::transport::relay_client::managed_slice_home_public_key(config, home_kernel_id)
        .map(Some)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "forward managed slice runtime tool",
            message:
                "managed slice owner identity does not match the provisioner-owned Room binding"
                    .to_string(),
        })
}
