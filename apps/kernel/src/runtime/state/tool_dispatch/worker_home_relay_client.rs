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
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        let response_timeout = if matches!(&request, RelayPeerRequest::InvokeHomeExtensionTool { tool, .. } if Self::is_remote_app_view_tool(tool))
        {
            None
        } else {
            Some(response_timeout)
        };
        #[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
        let response_timeout = Some(response_timeout);
        send_worker_home_request(
            config,
            &self.owned.relay_state,
            home_kernel_id,
            request,
            response_timeout,
        )
        .await
    }
}

pub(in crate::runtime::state) async fn send_worker_home_request(
    config: &crate::config::DaemonConfig,
    relay_state: &Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    home_kernel_id: &str,
    request: RelayPeerRequest,
    response_timeout: Option<Duration>,
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
            relay_state,
            target,
            request,
            response_timeout,
        )
        .await;
    }
    if let Some(owner_public_key) = managed_slice_home_owner_public_key(config, home_kernel_id)? {
        return crate::transport::relay_client::send_peer_request_to_known_kernel_via_relay_with_timeout(
            config,
            relay_state,
            target,
            &owner_public_key,
            request,
            response_timeout,
        )
        .await;
    }
    crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
        config,
        relay_state,
        target,
        request,
        response_timeout,
    )
    .await
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
    if config.managed_slice_relay_owner_public_key.is_none() {
        return Ok(None);
    }
    // Standalone managed slices carry the same provisioner-owned identity and
    // owner pin as Room slices, without a Room session binding. When both exist
    // they must agree before any peer request enters transport.
    match crate::runtime::state::provisioned_slice_home(config, identity) {
        Some(home) if home.kernel_id == home_kernel_id => Ok(Some(home.public_key)),
        _ => Err(DaemonError::LocalTransport {
            operation: "forward managed slice runtime tool",
            message: "managed slice owner identity does not match its provisioned binding"
                .to_string(),
        }),
    }
}

#[cfg(test)]
#[path = "worker_home_relay_client_tests.rs"]
mod tests;
