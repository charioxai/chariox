//! MP-11 F10: explicit one-command approval when provider policy is mutable.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, AgentWake};
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};

impl KernelRuntimeState {
    pub(super) async fn approve_agent_process(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        wake: &AgentWake,
        cwd: &Path,
        prompt: &str,
    ) -> Result<(), DaemonError> {
        // Claude's Bash sandbox applies even to print-mode bypass runs. Native
        // TUIs can change their permission mode after launch. Neither seam
        // supplies a current sandbox grant for a kernel-owned executable, so
        // always require explicit approval there, including initial Yolo.
        if run.provider() != "claude" && run.client_interface().is_chariox() {
            return Ok(());
        }
        let interaction = RuntimeInteraction::new(
            format!("watched-process-{}", wake.id), wake.agent_id.clone(),
            RuntimeInteractionKind::Permission, RuntimeInteractionLevel::Warning,
            Some("Run watched command outside the provider sandbox?".into()),
            format!("Command: {}\nDirectory: {}\nThe kernel runs this command with its own process policy. Provider-native permission and Bash sandbox settings do not apply. Approval permits this command once.", serde_json::to_string(&wake.command).map_err(|e| ledger::error(e.to_string()))?, super::super::agent_process_output::sanitize(cwd.to_string_lossy().as_bytes())),
            vec![RuntimeInteractionChoice::new("allow_once", "Allow once", "allow", Some(RuntimeInteractionChoiceStyle::Primary)), RuntimeInteractionChoice::new("deny", "Deny", "deny", Some(RuntimeInteractionChoiceStyle::Danger))],
            None, Some(300), Some("deny".into()),
        ).with_native_origin(self.owned.capture_native_interaction_origin(run.session_id(), &wake.agent_id, run.id()));
        let answer =
            crate::runtime::native_interaction_bridge::request_provider_native_interaction(
                self,
                run.session_id(),
                interaction,
                "agent_watched_process_permission",
            )
            .await?;
        if answer.status != "answered" || answer.choice_id.as_deref() != Some("allow_once") {
            return Err(ledger::error("watched process requires current explicit user approval; use your provider-native shell instead"));
        }
        self.authorize_room_provider_epoch(Some(&wake.agent_id), Some(run.id()))?;
        let session = self.owned.session_store.get_session(run.session_id())?;
        let current = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &wake.agent_id);
        if !current.is_some_and(|p| {
            p.id() == prompt && p.status() == crate::session::PromptStatus::Running
        }) {
            return Err(ledger::error(
                "watched process approval belongs to an ended turn",
            ));
        }
        Ok(())
    }
}
