use crate::error::DaemonError;

use super::{agent_assessment_result::ProtectedFact, KernelRuntimeState};

const COMPUTER_SECRET_APPROVAL_TIMEOUT_SEC: u64 = 30;

impl KernelRuntimeState {
    pub(super) fn record_protected_fact(&self, room: &str, agent: &str, fact: ProtectedFact) {
        let facts = &self.owned.private_assessment_dispatch.protected_facts;
        match fact {
            // An unclassified failure never replaces a specific kernel-observed fact.
            ProtectedFact::Failed => facts.record_failure(room, agent),
            _ => facts.record(room, agent, fact),
        }
    }

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
            .await
            .inspect_err(|error| {
                self.record_protected_fact(session_id, agent_id, target_error_fact(error))
            })?;
        let target = match result {
            RoomBrowserControllerResult::ComputerSecretTarget { target } if target.valid() => {
                target
            }
            _ => {
                self.record_protected_fact(session_id, agent_id, ProtectedFact::FieldNotFound);
                return Err(computer_secret_approval_error(
                    "worker returned an invalid focused desktop target",
                ));
            }
        };
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
                "Allow `{credential_id}` to be typed into native window {}, focused control {} at {:?}? Confirm that this focused field masks secret input; approving an unmasked field can expose the credential. Chariox aborts if the observable focus or window changes uses no clipboard, and masks the inserted region before agent screen observations.",
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
        // The default-on-timeout choice resolves as an ordinary "deny" answer.
        let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let timeout_flag = timed_out.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(
                COMPUTER_SECRET_APPROVAL_TIMEOUT_SEC,
            ))
            .await;
            timeout_flag.store(true, std::sync::atomic::Ordering::SeqCst);
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
        let fact = if resolution.status == "timed_out"
            || timed_out.load(std::sync::atomic::Ordering::SeqCst)
        {
            ProtectedFact::ApprovalTimedOut
        } else {
            ProtectedFact::ApprovalDenied
        };
        self.record_protected_fact(session_id, agent_id, fact);
        Err(computer_secret_approval_error(
            "computer credential input was denied or timed out",
        ))
    }
}

/// Only the capture's own "no usable field" error proves the target is missing; a relay,
/// controller or helper failure says nothing about the field.
fn target_error_fact(error: &DaemonError) -> ProtectedFact {
    match error {
        DaemonError::LocalTransport { message, .. }
            if message == super::tool_dispatch::FOCUSED_TARGET_UNAVAILABLE =>
        {
            ProtectedFact::FieldNotFound
        }
        _ => ProtectedFact::Failed,
    }
}

fn computer_secret_approval_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "computer_secret_input.approval",
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_confirmed_missing_target_is_field_not_found() {
        let transport = |message: &str| DaemonError::LocalTransport {
            operation: "environment.action.execute",
            message: message.to_string(),
        };
        assert_eq!(
            target_error_fact(&transport(
                super::super::tool_dispatch::FOCUSED_TARGET_UNAVAILABLE
            )),
            ProtectedFact::FieldNotFound
        );
        for failure in [
            transport("relay delivery timed out"),
            transport("controller restarted"),
            transport("capture helper failed to run"),
        ] {
            assert_eq!(target_error_fact(&failure), ProtectedFact::Failed);
        }
    }
}
