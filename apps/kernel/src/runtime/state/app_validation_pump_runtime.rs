//! Shows each pending App validation as a trusted kernel approval (outside App
//! content and the agent-operated Environment) and records the human decision.
//! An App can only request; neither App code, an App view click nor an agent
//! reply can answer a kernel-operation approval.
use super::KernelRuntimeState;
use crate::durable_state::app_validations::{ValidationCommand, ValidationOperation};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};

const PAGE: usize = 16;

impl KernelRuntimeState {
    pub(crate) fn schedule_app_validation_pump(&self) {
        let now_ms = crate::session::unix_epoch_ms();
        if self
            .owned
            .durable_state_store
            .require_writer_healthy()
            .is_err()
        {
            return;
        }
        let Some(pass) = self.app_control().validation_pump().try_begin(now_ms) else {
            return;
        };
        let runtime = self.clone();
        tokio::spawn(async move {
            let _pass = pass;
            runtime.app_validation_pass(now_ms).await;
        });
    }

    async fn app_validation_pass(&self, now_ms: u64) {
        let store = self.owned.durable_state_store.clone();
        let Ok(Ok(pending)) = tokio::task::spawn_blocking(move || {
            let _ = store.app_validation(ValidationCommand::Expire { now_ms });
            store.pending_app_validations(PAGE)
        })
        .await
        else {
            return;
        };
        for operation in pending {
            if !self
                .app_control()
                .begin_validation_prompt(&operation.operation_id)
            {
                continue;
            }
            let Some(session) = self.validation_session(&operation.owner) else {
                // No session to show it in yet; it stays pending until one exists.
                self.app_control()
                    .end_validation_prompt(&operation.operation_id);
                continue;
            };
            let interaction = validation_interaction(&operation);
            let receiver = match self
                .create_kernel_operation_interaction(&session, &operation.owner, interaction)
                .await
            {
                Ok(receiver) => receiver,
                Err(_) => {
                    self.app_control()
                        .end_validation_prompt(&operation.operation_id);
                    continue;
                }
            };
            let runtime = self.clone();
            tokio::spawn(async move {
                let decision = receiver.await.ok().and_then(|resolution| {
                    match resolution.choice_id.as_deref() {
                        Some("approve") if resolution.status == "answered" => Some(true),
                        Some("deny") if resolution.status == "answered" => Some(false),
                        _ => None,
                    }
                });
                if let Some(approved) = decision {
                    let store = runtime.owned.durable_state_store.clone();
                    let operation_id = operation.operation_id.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        store.app_validation(ValidationCommand::Decide {
                            operation_id,
                            approved,
                            now_ms: crate::session::unix_epoch_ms(),
                        })
                    })
                    .await;
                }
                // Undecided (timed out or abandoned): shown again on a later pass
                // while the operation is still pending.
                runtime
                    .app_control()
                    .end_validation_prompt(&operation.operation_id);
            });
        }
    }

    /// The owner's most recently used session hosts the approval, so it
    /// appears on the terminals the person is using.
    fn validation_session(&self, owner: &str) -> Option<String> {
        self.owned
            .session_store
            .list_sessions()
            .into_iter()
            .filter(|session| session.has_member(owner))
            .max_by_key(|session| {
                session
                    .last_prompt_sent_at_ms()
                    .unwrap_or(session.created_at_ms())
            })
            .map(|session| session.id().to_owned())
    }
}

fn validation_interaction(operation: &ValidationOperation) -> RuntimeInteraction {
    RuntimeInteraction::for_kernel_operation(
        format!("app_validation_{}", operation.operation_id),
        format!("validation:{}", operation.operation_id),
        "Approve App action",
        format!(
            "An App asks to perform a protected action.\n\nInstallation: {}\nAction: {}\nParameters: {}\n\nApprove only if you expect this exact action with these exact parameters.",
            operation.installation, operation.action, operation.parameters
        ),
        vec![
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            RuntimeInteractionChoice::new("approve", "Approve", "allow", None),
        ],
    )
}
