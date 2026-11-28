//! Launch the provider run that reruns one failed turn on an agent substitute,
//! and retire that run once its turn is over so the next turn starts on the
//! agent's configured profile.

use crate::agent::{AgentInstance, AgentSubstituteProfile};
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::TurnSubstitute;
use crate::session::PromptQueueItem;

#[cfg(test)]
#[path = "turn_substitute/admission_tests.rs"]
mod admission_tests;

/// The substitute profile a provider launch for one agent uses while
/// `DaemonApp::launch_turn_substitute_run` holds the app.
pub(crate) struct TurnSubstituteLaunch {
    agent_id: String,
    turn: TurnSubstitute,
    profile: AgentSubstituteProfile,
}

impl DaemonApp {
    /// Launches the agent's provider for the active `prompt` on the last of
    /// the substitutes `tried` in this turn, through the same workflow-aware
    /// launch path as its first attempt. The stored agent profile never
    /// changes.
    pub(crate) fn launch_turn_substitute_run(
        &mut self,
        session_id: &str,
        agent_id: &str,
        prompt: &PromptQueueItem,
        tried: Vec<AgentSubstituteProfile>,
    ) -> Result<String, DaemonError> {
        let profile = tried
            .last()
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "rerun turn on substitute",
                message: format!("no substitute was chosen for agent `{agent_id}`"),
            })?;
        self.turn_substitute_launch = Some(TurnSubstituteLaunch {
            agent_id: agent_id.to_string(),
            turn: TurnSubstitute {
                prompt_id: prompt.id().to_string(),
                tried,
            },
            profile,
        });
        let launched = if prompt.workflow_run_id().is_some() {
            crate::app::workflow_runtime::ensure_workflow_provider_run_for_prompt_from_runtime(
                self, session_id, agent_id, prompt,
            )
        } else {
            self.ensure_prompt_provider_run_for_agent(session_id, agent_id)
        };
        self.turn_substitute_launch = None;
        let provider_run_id = launched?;
        // The workflow node keeps its worktree claim; it now belongs to the
        // substitute run that settles the node.
        if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        {
            self.release_workflow_node_workspace_claim(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            );
            self.acquire_workflow_node_workspace_claim(
                session_id,
                &provider_run_id,
                agent_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
        }
        Ok(provider_run_id)
    }

    /// The same admission as the configured account's own check, except that a
    /// refusal a configured substitute can answer admits the turn: its launch
    /// runs on that substitute.
    pub(crate) fn require_agent_admitted(
        &self,
        owner_user_id: &str,
        agent: &AgentInstance,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        match self.provider_account_profiles.require_agent_authenticated(
            owner_user_id,
            agent,
            operation,
        ) {
            Err(refusal)
                if crate::substitute_admission::substitute_can_answer(&refusal)
                    && agent.remote_execution().is_none()
                    && crate::substitute_admission::first_usable_substitute(
                        &self.provider_account_profiles,
                        &self.config,
                        agent,
                        &[],
                    )?
                    .index
                    .is_some() =>
            {
                Ok(())
            }
            admitted => admitted,
        }
    }

    /// Runs `launch` for the turn `turn_prompt_id` (the agent's active turn
    /// when absent). When the agent's configured account is refused for a
    /// reason a substitute can answer, the launch uses the first substitute
    /// that can run, exactly as a rerun after a failed attempt does.
    pub(crate) fn with_admission_substitute<T>(
        &mut self,
        session_id: &str,
        agent_id: &str,
        turn_prompt_id: Option<&str>,
        launch: impl FnOnce(&mut Self) -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let engaged = self.engage_admission_substitute(session_id, agent_id, turn_prompt_id)?;
        let result = launch(self);
        if engaged {
            self.turn_substitute_launch = None;
        }
        result
    }

    fn engage_admission_substitute(
        &mut self,
        session_id: &str,
        agent_id: &str,
        turn_prompt_id: Option<&str>,
    ) -> Result<bool, DaemonError> {
        if self
            .turn_substitute_launch
            .as_ref()
            .is_some_and(|launch| launch.agent_id == agent_id)
        {
            return Ok(false);
        }
        let agent = self.agents.get_agent(agent_id)?;
        if agent.substitutes().is_empty()
            || agent.remote_execution().is_some()
            || self
                .reusable_leased_claude_run_for_agent(session_id, &agent)?
                .is_some()
        {
            return Ok(false);
        }
        let owner = self.provider_account_authority_owner_for_agent(&agent)?;
        let refusal = match self.provider_account_profiles.require_agent_authenticated(
            &owner,
            &agent,
            "admit turn",
        ) {
            Ok(()) => return Ok(false),
            Err(refusal) => refusal,
        };
        if !crate::substitute_admission::substitute_can_answer(&refusal) {
            return Ok(false);
        }
        let turn_prompt_id = match turn_prompt_id {
            Some(id) => id.to_string(),
            None => match self.prompt_owner_active_prompt_for_agent(session_id, agent_id)? {
                Some(prompt) if !prompt.is_external() => prompt.id().to_string(),
                _ => return Ok(false),
            },
        };
        // A live process of the configured profile that holds no turn is idle:
        // retire it so the substitute's process serves this turn.
        if let Some(run) = self.providers.get_run_for_agent(session_id, agent_id) {
            if run.state() != crate::provider::ProviderRunState::Ended
                && run.turn_substitute().is_none()
                && self.provider_run_has_active_prompt(session_id, &run)?
            {
                return Ok(false);
            }
        }
        let crate::substitute_admission::SubstituteChoice { index, skipped } =
            crate::substitute_admission::first_usable_substitute(
                &self.provider_account_profiles,
                &self.config,
                &agent,
                &[],
            )?;
        for (skipped_index, reason) in skipped {
            self.record_notice(
                session_id,
                None,
                self.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Skipping substitute {} for agent `{agent_id}`: {reason}.",
                    skipped_index + 1
                ),
            );
        }
        let Some(index) = index else {
            return Ok(false);
        };
        let profile = agent.substitutes()[index].clone();
        if self
            .providers
            .get_run_for_agent(session_id, agent_id)
            .is_some_and(|run| run.turn_substitute().is_none())
        {
            self.end_agent_provider_run(session_id, agent_id)?;
        }
        let label = if profile.model.trim().is_empty() || profile.model == "default" {
            profile.provider.clone()
        } else {
            profile.model.clone()
        };
        self.record_notice(
            session_id,
            None,
            self.attachments().list_session_attachment_ids(session_id),
            format!(
                "This turn runs on {label} because {}.",
                refusal.to_string().trim_end_matches('.')
            ),
        );
        self.turn_substitute_launch = Some(TurnSubstituteLaunch {
            agent_id: agent_id.to_string(),
            turn: TurnSubstitute {
                prompt_id: turn_prompt_id,
                tried: vec![profile.clone()],
            },
            profile,
        });
        Ok(true)
    }

    /// The profile a provider launch for `agent` uses, with the turn it reruns
    /// when that launch is a substitute's.
    pub(crate) fn agent_launch_profile(
        &self,
        agent: AgentInstance,
    ) -> (AgentInstance, Option<TurnSubstitute>) {
        match self
            .turn_substitute_launch
            .as_ref()
            .filter(|launch| launch.agent_id == agent.id())
        {
            Some(launch) => (
                agent.with_substitute_profile(&launch.profile),
                Some(launch.turn.clone()),
            ),
            None => (agent, None),
        }
    }

    /// A substitute run serves only the turn it reruns. Retire it before any
    /// other turn could reuse it as the agent's live provider run.
    pub(crate) fn retire_finished_turn_substitute_run(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(turn) = self
            .providers
            .get_run_for_agent(session_id, agent_id)
            .and_then(|run| run.turn_substitute().cloned())
        else {
            return Ok(());
        };
        if self
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .is_some_and(|active| active.id() == turn.prompt_id)
        {
            return Ok(());
        }
        self.end_agent_provider_run(session_id, agent_id)
    }
}
