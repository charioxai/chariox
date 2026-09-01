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
        // Keep the relay client's large future off callers' async stacks. Local
        // controller operations stay allocation-free; only the remote boundary
        // owns this boxed transport future.
        Box::pin(self.route_room_browser_controller_command(session_id, slice, command)).await
    }

    async fn route_room_browser_controller_command(
        &self,
        session_id: &str,
        slice: crate::slice::SliceRecord,
        command: Command,
    ) -> Result<Response, DaemonError> {
        // The original action retains its operation guard until terminal proof.
        // Cancellation must not wait for that very action to release the guard.
        let _guard = if matches!(&command, Command::CancelAction { .. }) {
            None
        } else {
            Some(self.owned.slice_store.guard_environment_use(
                &slice.id,
                Some(session_id),
                "browser_controller.route",
            )?)
        };
        let config = self.owned.config_projection.snapshot();
        let config = config.slice_relay_override(&slice).unwrap_or(config);
        let target = ClientTarget {
            daemon_id: slice.worker_kernel_id.clone(),
            daemon_alias: slice
                .worker_kernel_id
                .is_none()
                .then(|| slice.worker_kernel_ref.clone()),
        };
        let recovery = match &command {
            Command::Action {
                execution_id,
                target_id,
                document_id,
                node_ref,
                action,
                timeout_ms,
            } => Some(Command::RecoverAction {
                execution_id: execution_id.clone(),
                target_id: target_id.clone(),
                document_id: document_id.clone(),
                node_ref: node_ref.clone(),
                action: action.clone(),
                timeout_ms: *timeout_ms,
            }),
            _ => None,
        };
        let request = |command| RelayPeerRequest::RoomBrowserController {
            session_id: session_id.to_string(),
            slice_id: slice.id.clone(),
            command,
        };
        let first = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &config,
            target.clone(),
            request(command.clone()),
            Duration::from_secs(15),
        ).await;
        let response = match first {
            Ok(response) => response,
            Err(first_error) if recovery.is_some() => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    &config,
                    target,
                    request(recovery.expect("action recovery command")),
                    Duration::from_secs(15),
                ).await.map_err(|retry_error| controller_route_error(&format!(
                    "browser action result remained unavailable after non-mutating receipt recovery: {retry_error}; initial delivery error: {first_error}"
                )))?
            }
            Err(error) => return Err(error),
        };
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
        Command::CancelAction { execution_id } => Ok(Response::CancellationRequested {
            accepted: processes.cancel_browser_action(&session_id, &execution_id),
        }),
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
        Command::Navigate {
            target_id,
            document_id,
            url,
        } => processes
            .navigate_browser(&session_id, &target_id, &document_id, url.as_str())
            .map(|result| Response::Navigation { result }),
        Command::Wait {
            target_id,
            document_id,
            wait,
            timeout_ms,
        } => processes
            .wait_for_browser(&session_id, &target_id, &document_id, &wait, timeout_ms)
            .map(|result| Response::Wait { result }),
        Command::Dialog {
            target_id,
            document_id,
            action,
        } => processes
            .handle_browser_dialog(&session_id, &target_id, &document_id, &action)
            .map(|result| Response::Dialog { result }),
        Command::ConfigureDownloads {
            target_id,
            document_id,
        } => processes
            .configure_browser_downloads(&session_id, &target_id, &document_id)
            .map(|result| Response::Downloads { result }),
        Command::Upload {
            target_id,
            document_id,
            node_ref,
            files,
        } => processes
            .upload_browser_files(&session_id, &target_id, &document_id, &node_ref, &files)
            .map(|result| Response::Upload { result }),
        Command::Permission {
            target_id,
            document_id,
            permission,
            setting,
        } => processes
            .set_browser_permission(&session_id, &target_id, &document_id, permission, setting)
            .map(|result| Response::Permission { result }),
        Command::PollEvents {
            browser_generation,
            cursor,
            limit,
        } => processes
            .poll_browser_events(&session_id, browser_generation, cursor, limit)
            .map(|batch| Response::Events { batch }),
        Command::Action {
            execution_id,
            target_id,
            document_id,
            node_ref,
            action,
            timeout_ms,
        } => processes.perform_cancellable_browser_action(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &node_ref,
            &action,
            timeout_ms,
        ),
        Command::RecoverAction {
            execution_id,
            target_id,
            document_id,
            node_ref,
            action,
            timeout_ms,
        } => processes.recover_cancellable_browser_action(
            &session_id,
            &execution_id,
            &target_id,
            &document_id,
            &node_ref,
            &action,
            timeout_ms,
        ),
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
