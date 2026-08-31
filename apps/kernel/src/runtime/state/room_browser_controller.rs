use crate::runtime::browser_controller_process::BrowserControllerProcessStore;
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand as Command, RoomBrowserControllerResult as Response,
};

use super::*;

impl KernelRuntimeState {
    pub(crate) fn browser_controller_enabled_for_room(&self, session_id: &str) -> bool {
        self.owned
            .slice_store
            .environment_slice(session_id)
            .is_some()
            || self.browser_controller_process_enabled()
            || self
                .owned
                .config_projection
                .snapshot()
                .room_environment_worker_binding
                .is_some()
    }

    pub(super) async fn room_browser_controller_command(
        &self,
        session_id: &str,
        command: Command,
    ) -> Result<Response, DaemonError> {
        let Some(slice) = self.owned.slice_store.environment_slice(session_id) else {
            if self
                .owned
                .config_projection
                .snapshot()
                .room_environment_worker_binding
                .is_some()
            {
                return Err(controller_route_error(
                    "browser_controller_scope_denied: provisioned slice controller requires the home Room relay path",
                ));
            }
            return execute_local(
                self.owned.browser_controller_processes.clone(),
                session_id,
                command,
            )
            .await;
        };
        let _guard = self.owned.slice_store.guard_environment_use(
            &slice.id,
            Some(session_id),
            "browser_controller.route",
        )?;
        let config = self.owned.config_projection.snapshot();
        let config = config.slice_relay_override(&slice).unwrap_or(config);
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &config,
            ClientTarget { daemon_id: slice.worker_kernel_id.clone(),
                daemon_alias: slice.worker_kernel_id.is_none().then(|| slice.worker_kernel_ref.clone()) },
            RelayPeerRequest::RoomBrowserController {
                session_id: session_id.to_string(), slice_id: slice.id.clone(), command,
            },
            Duration::from_secs(15),
        ).await?;
        match response {
            RelayPeerResponse::RoomBrowserController {
                session_id: returned_room,
                slice_id,
                result,
            } if returned_room == session_id && slice_id == slice.id => Ok(result),
            _ => Err(controller_route_error(
                "worker returned a mismatched controller response",
            )),
        }
    }

    pub(crate) async fn execute_bound_room_browser_controller(
        &self,
        authenticated_kernel_id: &str,
        authenticated_public_key: &str,
        session_id: &str,
        slice_id: &str,
        command: Command,
    ) -> Result<Response, DaemonError> {
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
            return Err(controller_route_error("browser_controller_scope_denied: peer or Room does not match the provisioned slice binding"));
        }
        if !self.browser_controller_process_enabled() {
            return Err(controller_route_error(
                "browser_controller_unavailable: slice has no configured controller",
            ));
        }
        execute_local(
            self.owned.browser_controller_processes.clone(),
            session_id,
            command,
        )
        .await
    }
}

async fn execute_local(
    processes: BrowserControllerProcessStore,
    session_id: &str,
    command: Command,
) -> Result<Response, DaemonError> {
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || match command {
        Command::Acquire => processes
            .acquire(&session_id)
            .map(|snapshot| Response::Process { snapshot }),
        Command::Release => processes
            .release(&session_id)
            .map(|snapshot| Response::Process { snapshot }),
        Command::Reconcile { viewport } => processes
            .reconcile_browser(&session_id, &viewport)
            .map(|reconciliation| Response::Reconciled { reconciliation }),
        Command::Snapshot {
            target_id,
            document_id,
        } => processes
            .capture_browser_snapshot(&session_id, &target_id, &document_id)
            .map(|snapshot| Response::Snapshot { snapshot }),
    })
    .await
    .map_err(|error| controller_route_error(&error.to_string()))?
    .map_err(|message| controller_route_error(&message))
}

pub(super) fn controller_route_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "browser_controller.route",
        message: message.to_string(),
    }
}
