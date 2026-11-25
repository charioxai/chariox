//! F4: Claude's folder-trust question on a native terminal becomes one interaction card for every
//! client of the Room; the kernel types the answer into Claude.
use super::*;
use crate::app::{ClaudeTrustDialog, ClaudeTrustKeys};
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};

/// Claude needs a moment to register a moved selector before Enter confirms it.
const SELECTION_SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

impl KernelRuntimeState {
    /// Raises or withdraws the trust card after a drain of `provider_run`'s terminal output.
    pub(super) async fn follow_claude_trust_dialog(
        &self,
        session_id: &str,
        provider_run: &crate::provider::RuntimeProviderRun,
        update: ClaudeTrustDialog,
    ) {
        let interaction_id = crate::app::claude_native_trust_interaction_id(provider_run);
        match update {
            ClaudeTrustDialog::AnsweredInTerminal => {
                // Answered through the card, it is already settled: nothing left to withdraw.
                let _ = self
                    .timeout_runtime_interaction(session_id, &interaction_id)
                    .await;
            }
            ClaudeTrustDialog::Opened { workspace } => {
                let Some(agent_id) = provider_run.agent_instance_id() else {
                    return;
                };
                let interaction = RuntimeInteraction::new(
                    interaction_id.clone(),
                    agent_id,
                    RuntimeInteractionKind::Permission,
                    RuntimeInteractionLevel::Warning,
                    Some("Trust this Claude Code workspace?".to_string()),
                    format!("{workspace}\nClaude is asking in its terminal, so the agent has not started."),
                    vec![
                        RuntimeInteractionChoice::new(
                            "allow_once",
                            "Trust workspace",
                            "allow",
                            Some(RuntimeInteractionChoiceStyle::Primary),
                        ),
                        RuntimeInteractionChoice::new(
                            "deny",
                            "Don't trust",
                            "deny",
                            Some(RuntimeInteractionChoiceStyle::Danger),
                        ),
                    ],
                    None,
                    None,
                    None,
                )
                .with_native_origin(Some(
                    crate::session::NativeInteractionOrigin::ProviderStartup {
                        provider_run_id: provider_run.id().to_string(),
                    },
                ));
                let receiver = match self
                    .create_runtime_interaction(
                        crate::runtime::state::CardKind::NativePermissionInput,
                        session_id,
                        interaction,
                    )
                    .await
                {
                    Ok(receiver) => receiver,
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "daemon.provider_output",
                            "Claude trust card could not be raised",
                            serde_json::json!({
                                "session_id": session_id,
                                "provider_run_id": provider_run.id(),
                                "error": error.to_string(),
                            }),
                        );
                        return;
                    }
                };
                let runtime = self.clone();
                let provider_run = provider_run.clone();
                tokio::spawn(async move {
                    let Ok(resolution) = receiver.await else {
                        return;
                    };
                    if resolution.status != "answered" {
                        return;
                    }
                    let trust = resolution.reply.as_deref() == Some("allow");
                    runtime.type_claude_trust_answer(&provider_run, trust).await;
                });
            }
        }
    }

    async fn type_claude_trust_answer(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        trust: bool,
    ) {
        let run_id = provider_run.id().to_string();
        let Some(ClaudeTrustKeys { first, then_enter }) =
            crate::app::claude_native_trust_keys(provider_run, trust)
        else {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "Claude trust answer found no selector to move",
                serde_json::json!({"provider_run_id": run_id}),
            );
            return;
        };
        let write = |bytes: Vec<u8>| {
            let run_id = run_id.clone();
            async move {
                self.with_app_side_effect(|app| {
                    app.write_provider_pty_input_for_runtime(&run_id, &bytes)
                })
                .await
            }
        };
        let mut result = write(first).await;
        if result.is_ok() && then_enter {
            tokio::time::sleep(SELECTION_SETTLE).await;
            result = write(b"\r".to_vec()).await;
        }
        if let Err(error) = result {
            crate::logging::warn_with_fields(
                "daemon.provider_output",
                "Claude trust answer could not be typed",
                serde_json::json!({"provider_run_id": run_id, "error": error.to_string()}),
            );
        }
    }
}
