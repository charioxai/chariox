use crate::error::DaemonError;

use super::KernelRuntimeState;

const COMPUTER_SECRET_APPROVAL_TIMEOUT_SEC: u64 = 30;

impl KernelRuntimeState {
    pub(super) async fn ensure_computer_secret_input_approved(
        &self,
        session_id: &str,
        agent_id: &str,
        credential_id: &str,
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
                "Allow `{credential_id}` to be typed into the currently focused desktop control? Continue only if that control masks secret input. Chariox will preserve the current desktop focus and will not use the clipboard."
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
