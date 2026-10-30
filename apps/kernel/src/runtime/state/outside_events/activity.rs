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
    /// Run states of the followed targets for a new subscription. Blocking store reads: the
    /// caller holds only the managed-activity boundary, which is taken before any store.
    pub(in crate::runtime::state) fn outside_run_state_baseline(
        &self,
        targets: &[EventTarget],
    ) -> Vec<RunState> {
        let kernel = self.config_projection.snapshot().daemon_id;
        let sessions = local_sessions(targets, &kernel)
            .filter_map(|id| {
                // An ended/deleted session has already published its terminal event.
                let mut session = self.session_store.get_session(id).ok()?;
                session.set_agents(self.agent_store.get_session_agents(id));
                Some(session)
            })
            .collect();
        self.outside_run_states(&kernel, targets, sessions)
    }

    /// Queues the followed run states of `session` (of every followed session when `None`)
    /// that changed since the writer last got them. Publishers may hold a store guard, so the
    /// stores are only tried; a busy store leaves the states for the next publication.
    pub(in crate::runtime::state) fn publish_outside_run_states(&self, session: Option<&str>) {
        let queued = self
            .durable_state_store
            .enqueue_outside_run_states(session, |targets| {
                let kernel = self.config_projection.snapshot().daemon_id;
                let mut sessions = Vec::new();
                for id in local_sessions(targets, &kernel) {
                    let Ok(mut session) = self.session_store.try_get_session_for_projection(id)?
                    else {
                        continue;
                    };
                    session.set_agents(self.agent_store.try_get_session_agents_for_projection(id)?);
                    sessions.push(session);
                }
                Some(self.outside_run_states(&kernel, targets, sessions))
            });
        if let Err(error) = queued {
            crate::logging::error_with_fields(
                "outside.events",
                "agent run-state transition could not be queued",
                serde_json::json!({"error":error.to_string()}),
            );
        }
    }

    fn outside_run_states(
        &self,
        kernel: &str,
        targets: &[EventTarget],
        sessions: Vec<crate::session::RuntimeSession>,
    ) -> Vec<RunState> {
        let active_turns = self.active_turns.snapshot();
        let mut states = Vec::new();
        for session in sessions {
            let id = session.id();
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
                let turn = settled_turn(&value);
                // The turn that just settled keeps its activity until the settlement
                // finishes. That is not a successor starting, so its end is published
                // once, when the activity clears.
                if value.status == AgentRuntimeStatus::Working
                    && turn
                        .as_ref()
                        .zip(value.active_turn.as_ref())
                        .is_some_and(|(settled, active)| settled.prompt_id == active.prompt_id)
                {
                    continue;
                }
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
        states
    }
}

fn local_sessions<'a>(
    targets: &'a [EventTarget],
    kernel: &'a str,
) -> impl Iterator<Item = &'a str> + 'a {
    targets
        .iter()
        .filter(move |target| target.kernel == kernel)
        .map(|target| target.session_id.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
}
