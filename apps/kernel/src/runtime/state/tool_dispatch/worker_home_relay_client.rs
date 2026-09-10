use std::time::Duration;

use chariox_relay::protocol::ClientTarget;

use super::*;

impl KernelRuntimeState {
    pub(super) async fn send_worker_home_runtime_request(
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
    let Some(owner_public_key) = config.managed_slice_relay_owner_public_key.as_ref() else {
        return Ok(None);
    };
    let binding = config
        .room_environment_worker_binding
        .as_ref()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "forward managed slice runtime tool",
            message: "managed slice worker has no provisioner-owned Room binding".to_string(),
        })?;
    if binding.home_kernel_id != home_kernel_id || binding.home_public_key != *owner_public_key {
        return Err(DaemonError::LocalTransport {
            operation: "forward managed slice runtime tool",
            message: "managed slice owner identity does not match the Room binding".to_string(),
        });
    }
    Ok(Some(owner_public_key.clone()))
}
