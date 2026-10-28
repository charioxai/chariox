//! MP-08 / MP-09 / MP-10 / MP-11: use the same committed activity as the clients.
use super::*;
use crate::durable_state::agent_lifecycle::outside::{RunState, SettledTurn};
use crate::runtime::projection::{AgentRuntimeActivity, AgentRuntimeStatus};
use std::collections::BTreeSet;

/// The agent's last settled turn and how it ended.
fn settled_turn(activity: &AgentRuntimeActivity) -> Option<SettledTurn> {
    let turn = activity.last_completed_turn.as_ref()?;
    Some(SettledTurn {
        prompt_id: turn.prompt_id.clone(),
        turn_id: turn.turn_id.clone(),
        outcome: turn.outcome.clone()?,
    })
}

impl KernelRuntimeOwnedState {
    pub(in crate::runtime::state) fn outside_run_state_snapshot(
        &self,
        targets: &[EventTarget],
    ) -> Option<Vec<RunState>> {
        let kernel = self.config_projection.snapshot().daemon_id;
        let sessions: BTreeSet<_> = targets
            .iter()
            .filter(|target| target.kernel == kernel)
            .map(|target| target.session_id.as_str())
            .collect();
        let active_turns = self.active_turns.snapshot();
        let mut states = Vec::new();
        for id in sessions {
            // An ended/deleted session has already published its terminal event.
            let Ok(mut session) = self.session_store.try_get_session_for_projection(id)? else {
                continue;
            };
            session.set_agents(self.agent_store.try_get_session_agents_for_projection(id)?);
            // Settlement detail changes prompt_status, but not its normalized run state.
            // Avoid taking the prompt-activity lock inside its own mutation boundary.
            let mut activity = crate::runtime::projection::agent_activity_for_session_projection(
                &session,
                |agent| {
                    self.provider_run_projection
                        .get_for_agent(id, agent)
                        .or_else(|| {
                            self.provider_store
                                .try_get_agent_run_for_projection(id, agent)
                        })
                },
                &Default::default(),
                &active_turns,
                None,
                |agent| {
                    self.completed_git_turn_snapshots
                        .latest_projection_for_agent(id, agent)
                },
            );
            self.session_projection
                .project_external_observed_activity(id, &mut activity);
            for (agent_id, value) in activity {
                if !targets.iter().any(|target| {
                    target.kernel == kernel
                        && target.session_id == id
                        && target
                            .agent_id
                            .as_ref()
                            .is_none_or(|agent| agent == &agent_id)
                }) {
                    continue;
                }
                let turn = (value.status != AgentRuntimeStatus::Working)
                    .then(|| settled_turn(&value))
                    .flatten();
                let state = match value.status {
                    AgentRuntimeStatus::Idle => EventRunState::Idle,
                    AgentRuntimeStatus::Working => EventRunState::Working,
                    AgentRuntimeStatus::Error => EventRunState::Error,
                };
                states.push(RunState {
                    session_id: id.into(),
                    agent_id,
                    state,
                    turn,
                });
            }
        }
        Some(states)
    }

    pub(in crate::runtime::state) fn publish_outside_runtime_activity(
        &self,
    ) -> Result<(), DaemonError> {
        let targets: Vec<_> = self
            .durable_state_store
            .outside_parts()?
            .into_iter()
            .filter(|part| part.ended.is_none())
            .flat_map(|part| part.targets)
            .collect();
        if targets.is_empty() {
            return Ok(());
        }
        // Some callers still hold a store's mutation guard. Their normal projection
        // publication retries this capture after those guards have been released.
        let Some(states) = self.outside_run_state_snapshot(&targets) else {
            return Ok(());
        };
        if states.is_empty() {
            return Ok(());
        }
        self.durable_state_store
            .outside_mutate(Mutation::RunStates { states })
    }
}
