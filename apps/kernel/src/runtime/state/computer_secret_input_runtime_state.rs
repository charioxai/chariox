use crate::error::DaemonError;

use super::KernelRuntimeState;

impl KernelRuntimeState {
    pub(crate) fn computer_screen_capture_guard(
        &self,
    ) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, DaemonError> {
        self.owned
            .computer_input_executions
            .capture_guard()
            .map_err(|message| DaemonError::LocalTransport {
                operation: "environment.computer.capture",
                message: message.to_string(),
            })
    }

    pub(super) async fn computer_secret_input_target(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::transport::room_browser_controller::RoomComputerSecretTarget, DaemonError>
    {
        use crate::transport::room_browser_controller::{
            RoomBrowserControllerCommand, RoomBrowserControllerResult,
        };
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.into(),
                agent_id: agent_id.into(),
            });
        }
        self.recover_active_room_for_computer_input(session_id)
            .await?;
        let result = self
            .room_browser_controller_command(
                session_id,
                RoomBrowserControllerCommand::ComputerSecretTarget,
            )
            .await?;
        let RoomBrowserControllerResult::ComputerSecretTarget { target } = result else {
            return Err(computer_secret_target_error(
                "worker returned an invalid focused desktop target",
            ));
        };
        if !target.valid() {
            return Err(computer_secret_target_error(
                "worker returned an invalid focused desktop target",
            ));
        }
        Ok(target)
    }
}

fn computer_secret_target_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "computer_secret_input.target",
        message: message.to_string(),
    }
}
