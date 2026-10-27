use crate::error::DaemonError;
use crate::session::EnvironmentLifecycle;

use super::KernelRuntimeState;

impl KernelRuntimeState {
    /// MP-08/MP-10/MP-11: recover an already running Room through the same
    /// controller and headed-slice health checks used by Browser admission.
    /// Input must never implicitly start a stopped or failed Room.
    pub(super) async fn recover_active_room_for_computer_input(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "environment.computer.recover",
                message: format!("{}: {error:?}", error.code()),
            })?;
        if matches!(
            environment.lifecycle,
            EnvironmentLifecycle::Starting | EnvironmentLifecycle::Degraded
        ) && self.browser_controller_enabled_for_room(session_id)
        {
            self.finish_room_environment_controller_start(
                session_id,
                "environment.computer.recover",
            )
            .await?;
        }
        Ok(())
    }
}
