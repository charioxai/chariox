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
