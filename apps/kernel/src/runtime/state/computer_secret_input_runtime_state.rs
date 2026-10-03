use crate::error::DaemonError;

use super::KernelRuntimeState;

const COMPUTER_SECRET_APPROVAL_TIMEOUT_SEC: u64 = 30;

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
            return Err(computer_secret_approval_error(
                "worker returned an invalid focused desktop target",
            ));
        };
        if !target.valid() {
            return Err(computer_secret_approval_error(
                "worker returned an invalid focused desktop target",
            ));
        }
        Ok(target)
    }

    pub(super) async fn ensure_computer_secret_input_approved(
        &self,
        session_id: &str,
        agent_id: &str,
        credential_id: &str,
        target: &crate::transport::room_browser_controller::RoomComputerSecretTarget,
    ) -> Result<(), DaemonError> {
        let interaction = crate::session::RuntimeInteraction::new(
            format!(
                "computer-secret-input-{agent_id}-{}",
                crate::session::unix_epoch_ms()
            ),
            agent_id,
            crate::session::RuntimeInteractionKind::Permission,
            crate::session::RuntimeInteractionLevel::Critical,
            Some("Computer credential input".to_string()),
            format!(
                "Allow `{credential_id}` to be typed into native window {}, focused control {} at {:?}? Confirm that this focused field masks secret input; approving an unmasked field can expose the credential. Chariox aborts if the observable focus or window changes and withholds agent screen captures while typing, without using the clipboard.",
                target.active_window, target.focus_window, target.geometry
            ),
            vec![
                crate::session::RuntimeInteractionChoice::new(
                    "allow",
                    "Type credential",
                    "allow",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
                ),
                crate::session::RuntimeInteractionChoice::new(
                    "deny",
                    "Cancel",
                    "deny",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            Some(COMPUTER_SECRET_APPROVAL_TIMEOUT_SEC),
            Some("deny".to_string()),
        );
        let interaction_id = interaction.id().to_string();
        let resolution_rx = self
            .create_runtime_interaction(session_id, interaction)
            .await?;
        let state = self.clone();
        let timeout_session_id = session_id.to_string();
        let timeout_interaction_id = interaction_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(
                COMPUTER_SECRET_APPROVAL_TIMEOUT_SEC,
            ))
            .await;
            let _ = state
                .timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id)
                .await;
        });
        let resolution = resolution_rx.await.map_err(|error| {
            computer_secret_approval_error(&format!(
                "approval interaction dropped before resolution: {error}"
            ))
        })?;
        if resolution.choice_id.as_deref() == Some("allow") {
            return Ok(());
        }
        Err(computer_secret_approval_error(
            "computer credential input was denied or timed out",
        ))
    }
}

fn computer_secret_approval_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "computer_secret_input.approval",
        message: message.to_string(),
    }
}
