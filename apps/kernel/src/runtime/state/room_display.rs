use super::*;
use crate::local::GetSliceDisplayEndpointRequest;
use crate::runtime::command::{KernelCaller, KernelCallerKind};
use crate::slice::{SliceDisplayEndpoint, SliceDisplayEndpointKind, SliceStatus};

impl KernelRuntimeState {
    pub(crate) async fn open_room_selkies_display(
        &self,
        caller: &KernelCaller,
        request: GetSliceDisplayEndpointRequest,
    ) -> Result<SliceDisplayEndpoint, DaemonError> {
        let session_id = required_request_field(request.session_id, "session_id")?;
        let attachment_id = required_request_field(request.attachment_id, "attachment_id")?;
        let viewer_public_key =
            required_request_field(request.viewer_public_key, "viewer_public_key")?;
        crate::transport::relay_crypto::decode_public_key(&viewer_public_key)
            .map_err(|_| display_error("viewer public key is invalid"))?;
        match caller.caller_kind {
            KernelCallerKind::LocalClient => {}
            KernelCallerKind::RemoteClient => {
                let expected = caller.public_key_thumbprint.as_deref().ok_or_else(|| {
                    display_error("remote viewer admission requires a key-bound relay identity")
                })?;
                let actual =
                    crate::runtime::terminal_pairings::public_key_thumbprint(&viewer_public_key);
                if expected != actual {
                    return Err(display_error(
                        "viewer key does not match the authenticated relay client",
                    ));
                }
            }
            KernelCallerKind::RemoteKernel
            | KernelCallerKind::HostedService
            | KernelCallerKind::Metaagent => {
                return Err(display_error("caller cannot open a Room display"));
            }
        }
        self.ensure_attachment_in_session(&session_id, &attachment_id)
            .await?;
        let caller_user_id = caller
            .user_id
            .as_deref()
            .unwrap_or(crate::session::DEFAULT_LOCAL_USER_ID);
        if self.attachment_owner_user_id(&attachment_id).await? != caller_user_id {
            return Err(DaemonError::SessionAccessDenied {
                session_id,
                user_id: caller_user_id.to_string(),
            });
        }
        let binding = self
            .room_environment_slice(&session_id)?
            .ok_or_else(|| display_error("Room has no bound Environment slice"))?;
        let slice = self.resolve_slice(&request.slice_ref)?;
        if binding.slice_id != slice.id {
            return Err(display_error(
                "requested slice is not the Room Environment slice",
            ));
        }
        if slice.status != SliceStatus::Running
            || slice
                .display_endpoint
                .as_ref()
                .is_none_or(|endpoint| endpoint.kind != SliceDisplayEndpointKind::Selkies)
        {
            return Err(display_error(
                "Room Environment Selkies display is not running",
            ));
        }
        let _guard = self.owned.slice_store.guard_environment_use(
            &slice.id,
            Some(&session_id),
            "environment.display.open",
        )?;
        let config = self.owned.config_projection.snapshot();
        let config = config.slice_relay_override(&slice).unwrap_or(config);
        let target = ClientTarget {
            daemon_id: slice.worker_kernel_id.clone(),
            daemon_alias: slice
                .worker_kernel_id
                .is_none()
                .then(|| slice.worker_kernel_ref.clone()),
        };
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &config,
            target,
            RelayPeerRequest::OpenRoomDisplay {
                session_id: session_id.clone(),
                slice_id: slice.id.clone(),
                viewer_public_key,
            },
            Duration::from_secs(15),
        )
        .await?;
        match response {
            RelayPeerResponse::RoomDisplayOpened {
                session_id: returned_session,
                slice_id,
                endpoint,
            } if returned_session == session_id && slice_id == slice.id => Ok(endpoint),
            _ => Err(display_error(
                "worker returned a mismatched Room display response",
            )),
        }
    }

    pub(crate) async fn execute_bound_room_display_open(
        &self,
        authenticated_kernel_id: &str,
        authenticated_public_key: &str,
        session_id: &str,
        slice_id: &str,
        viewer_public_key: String,
    ) -> Result<SliceDisplayEndpoint, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let permitted = config
            .room_environment_worker_binding
            .as_ref()
            .is_some_and(|binding| {
                binding.permits(
                    authenticated_kernel_id,
                    authenticated_public_key,
                    session_id,
                    slice_id,
                )
            });
        if !permitted {
            return Err(display_error(
                "Room display peer or binding scope was denied",
            ));
        }
        crate::runtime::slice_command_executor::register_room_selkies_display_endpoint(
            Arc::clone(&self.owned.relay_state),
            config.relay_url,
            slice_id,
            viewer_public_key,
            config.relay_public_key,
        )
        .await
    }
}

fn required_request_field(value: Option<String>, field: &str) -> Result<String, DaemonError> {
    value
        .filter(|value| !value.is_empty() && value.trim() == value)
        .ok_or_else(|| display_error(&format!("Room Selkies display requires {field}")))
}

fn display_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "open Room Selkies display",
        message: message.to_string(),
    }
}
