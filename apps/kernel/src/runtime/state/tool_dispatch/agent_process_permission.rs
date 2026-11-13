//! MP-11 F10: explicit one-command approval when provider policy is mutable.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, AgentWake};
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};

impl KernelRuntimeState {
    /// Consent for a kernel-run command of an agent on this kernel. A person may answer
    /// "Allow for this agent": the agent's later commands and jobs then run without a card until
    /// its permissions change, its room ends or a parent revokes the grant.
    pub(in crate::runtime::state) async fn approve_agent_process(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        wake: &AgentWake,
        cwd: &Path,
        prompt: &str,
    ) -> Result<(), DaemonError> {
        self.approve_process(run, wake, cwd, prompt, true).await
    }

    /// Consent for a command a leased worker runs for its home: the grant would live on the
    /// worker, out of reach of the parent that must be able to see and revoke it, so every
    /// command asks.
    pub(in crate::runtime::state) async fn approve_leased_agent_process(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        wake: &AgentWake,
        cwd: &Path,
        prompt: &str,
    ) -> Result<(), DaemonError> {
        self.approve_process(run, wake, cwd, prompt, false).await
    }

    pub(crate) fn holds_watched_process_grant(&self, agent_id: &str) -> bool {
        self.watched_process_grants.is_allowed(agent_id)
    }

    /// Whether the agent held the grant: its next watched command asks again.
    pub(crate) fn revoke_watched_process_grant(&self, agent_id: &str) -> bool {
        self.watched_process_grants.forget(agent_id)
    }

    async fn approve_process(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        wake: &AgentWake,
        cwd: &Path,
        prompt: &str,
        grantable: bool,
    ) -> Result<(), DaemonError> {
        // Claude's Bash sandbox applies even to print-mode bypass runs. Native
        // TUIs can change their permission mode after launch. Neither seam
        // supplies a current sandbox grant for a kernel-owned executable, so
        // require kernel process consent there, including initial Yolo.
        if run.provider() != "claude" && run.client_interface().is_chariox() {
            return Ok(());
        }
        if !(grantable && self.watched_process_grants.is_allowed(&wake.agent_id)) {
            self.ask_process_consent(run, wake, cwd, grantable).await?;
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

    async fn ask_process_consent(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        wake: &AgentWake,
        cwd: &Path,
        grantable: bool,
    ) -> Result<(), DaemonError> {
        let mut choices = vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )];
        if grantable {
            choices.push(RuntimeInteractionChoice::new(
                "allow_agent",
                "Allow for this agent",
                "allow_agent",
                Some(RuntimeInteractionChoiceStyle::Secondary),
            ));
        }
        choices.push(RuntimeInteractionChoice::new(
            "deny",
            "Deny",
            "deny",
            Some(RuntimeInteractionChoiceStyle::Danger),
        ));
        let scope = if grantable {
            "Allow once permits this command; Allow for this agent permits every watched command and job of this agent until its permissions change or a parent revokes it."
        } else {
            "Approval permits this command once."
        };
        let interaction = RuntimeInteraction::new(
            format!("watched-process-{}", wake.id), wake.agent_id.clone(),
            RuntimeInteractionKind::Permission, RuntimeInteractionLevel::Warning,
            Some("Run watched command outside the provider sandbox?".into()),
            format!("Command: {}\nDirectory: {}\nThe kernel runs this command with its own process policy. Provider-native permission and Bash sandbox settings do not apply. {scope}", serde_json::to_string(&wake.command).map_err(|e| ledger::error(e.to_string()))?, super::super::agent_process_output::sanitize(cwd.to_string_lossy().as_bytes())),
            choices, None, Some(300), Some("deny".into()),
        ).with_native_origin(self.owned.capture_native_interaction_origin(run.session_id(), &wake.agent_id, run.id()));
        let answer =
            crate::runtime::native_interaction_bridge::request_provider_native_interaction(
                self,
                run.session_id(),
                interaction,
                "agent_watched_process_permission",
            )
            .await?;
        match answer.choice_id.as_deref().filter(|_| answer.status == "answered") {
            Some("allow_once") => Ok(()),
            Some("allow_agent") if grantable => {
                self.watched_process_grants.allow(&wake.agent_id);
                Ok(())
            }
            _ => Err(ledger::error("watched process requires current explicit user approval; use your provider-native shell instead")),
        }
    }
}
