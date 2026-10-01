//! Launch the provider run that reruns one failed turn on an agent substitute,
//! and retire that run once its turn is over so the next turn starts on the
//! agent's configured profile.

use crate::agent::{AgentInstance, AgentSubstituteProfile};
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::TurnSubstitute;
use crate::session::PromptQueueItem;

/// The substitute profile a provider launch for one agent uses while
/// `DaemonApp::launch_turn_substitute_run` holds the app.
pub(crate) struct TurnSubstituteLaunch {
    agent_id: String,
    turn: TurnSubstitute,
    profile: AgentSubstituteProfile,
}

impl DaemonApp {
    /// Launches the agent's provider on substitute `substitute_index` for the
    /// active `prompt`, through the same workflow-aware launch path as its
    /// first attempt. The stored agent profile never changes.
    pub(crate) fn launch_turn_substitute_run(
        &mut self,
        session_id: &str,
        agent_id: &str,
        prompt: &PromptQueueItem,
        substitute_index: usize,
    ) -> Result<String, DaemonError> {
        let profile = self
            .agents
            .get_agent(agent_id)?
            .substitutes()
            .get(substitute_index)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "rerun turn on substitute",
                message: format!(
                    "agent `{agent_id}` has no substitute {}",
                    substitute_index + 1
                ),
            })?;
        self.turn_substitute_launch = Some(TurnSubstituteLaunch {
            agent_id: agent_id.to_string(),
            turn: TurnSubstitute {
                prompt_id: prompt.id().to_string(),
                substitute_index,
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
        launched
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
        self.end_provider_run_for_workflow_context_flush(session_id, agent_id)
    }
}
