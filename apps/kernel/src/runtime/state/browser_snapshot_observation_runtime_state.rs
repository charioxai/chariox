//! MP-08 / MP-10 / MP-11: fresh browser observations preserve document-bound action authority.
use crate::error::DaemonError;
use crate::session::EnvironmentLifecycle;
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand, RoomBrowserControllerResult,
};

use super::browser_controller_runtime_state::environment_runtime_error;
use super::room_browser_controller::controller_route_error;
use super::KernelRuntimeState;

impl KernelRuntimeState {
    pub(crate) async fn capture_browser_environment_snapshot(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<
        crate::runtime::browser_controller_snapshot::RoomBrowserStructuredSnapshot,
        DaemonError,
    > {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.snapshot", error))?;
        if matches!(
            environment.lifecycle,
            EnvironmentLifecycle::Stopped
                | EnvironmentLifecycle::Stopping
                | EnvironmentLifecycle::Failed
        ) {
            return Err(controller_route_error(
                "browser_unavailable: explicit Room start required",
            ));
        }
        // MP-08 / MP-11: reject an unknown tab before probing the controller.
        self.room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.snapshot", error))?;
        // Native address-bar navigation changes the document outside kernel actions.
        // Observe it through the existing bound-worker route before minting fresh
        // references. Old action references retain their document revision fence.
        let environment = self
            .reconcile_browser_controller_environment(session_id)
            .await?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.snapshot", error))?;
        let RoomBrowserControllerResult::Snapshot {
            snapshot: Some(controller_snapshot),
        } = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::Snapshot {
                    target_id: binding.runtime_target_id.clone(),
                    document_id: binding.document_id.clone(),
                },
            )
            .await?
        else {
            return Err(controller_route_error(
                "browser controller did not return a snapshot",
            ));
        };
        controller_snapshot
            .validate(&binding.runtime_target_id, &binding.document_id)
            .map_err(|message| controller_route_error(&message))?;
        let references = self
            .register_room_environment_element_references(
                session_id,
                tab_id,
                environment.runtime_generation,
                binding.document_revision,
                controller_snapshot.controller_node_refs(),
            )
            .map_err(|error| environment_runtime_error("browser_controller.snapshot", error))?;
        controller_snapshot
            .into_room_snapshot(
                session_id.to_string(),
                environment.environment_id,
                environment.runtime_generation,
                tab_id.to_string(),
                binding.document_revision,
                &references,
            )
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.snapshot",
                message,
            })
    }
}
