//! Session and agent runtime projection invariant checks.

use std::collections::{BTreeMap, BTreeSet};

use crate::agent::AgentInstance;
use crate::app::ActiveTurnState;
use crate::provider::{ProviderRunState, RuntimeProviderRun};
use crate::session::{PromptQueueItem, RuntimeSession};

use super::super::{
    AgentRuntimeProjectionStore, ProjectionInvariantHealthSnapshot, ProjectionInvariantMismatch,
};

pub(super) fn snapshot(
    sessions: Vec<RuntimeSession>,
    agent_runtime: &AgentRuntimeProjectionStore,
    canonical_agents: &[AgentInstance],
    active_turns: &BTreeMap<String, ActiveTurnState>,
    provider_runs: &[RuntimeProviderRun],
) -> ProjectionInvariantHealthSnapshot {
    let mut agent_projections = agent_runtime
        .list()
        .into_iter()
        .map(|projection| (projection.agent_id.clone(), projection))
        .collect::<BTreeMap<_, _>>();
    let canonical_agent_ids = canonical_agents
        .iter()
        .map(|agent| agent.id().to_string())
        .collect::<BTreeSet<_>>();
    let session_ids = sessions
        .iter()
        .map(|session| session.id().to_string())
        .collect::<BTreeSet<_>>();
    let provider_runs_by_id = provider_runs
        .iter()
        .map(|run| (run.id().to_string(), run))
        .collect::<BTreeMap<_, _>>();
    let mut projected_session_agents = BTreeMap::new();
    let mut checked_agents = 0;
    let mut mismatches = Vec::new();

    for session in &sessions {
        if let Some(focused_agent_id) = session.focused_agent_id() {
            if !session
                .agents()
                .iter()
                .any(|agent| agent.id() == focused_agent_id)
            {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "stale_focused_agent".to_string(),
                    session_id: session.id().to_string(),
                    agent_id: Some(focused_agent_id.to_string()),
                    details: "focused agent is not present in the session agent list".to_string(),
                });
            }
        }

        let mut expected_prompt_states = BTreeMap::new();
        let mut session_agent_ids = BTreeSet::new();
        for agent in session.agents() {
            session_agent_ids.insert(agent.id().to_string());
            projected_session_agents.insert(agent.id().to_string(), session.id().to_string());
            if !canonical_agent_ids.is_empty() && !canonical_agent_ids.contains(agent.id()) {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "session_agent_missing_record".to_string(),
                    session_id: session.id().to_string(),
                    agent_id: Some(agent.id().to_string()),
                    details: "session projection contains an agent missing from the canonical agent store".to_string(),
                });
            }
            let prompt_state = session.prompt_states().get(agent.id());
            expected_prompt_states.insert(
                agent.id().to_string(),
                (
                    prompt_state.and_then(|state| state.active_prompt().cloned()),
                    prompt_state.and_then(|state| state.queued_prompts().front().cloned()),
                    prompt_state
                        .map(|state| state.queued_prompts().len())
                        .unwrap_or(0),
                ),
            );
        }
        for agent_id in session.prompt_states().keys() {
            if !session_agent_ids.contains(agent_id) {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "prompt_state_without_session_agent".to_string(),
                    session_id: session.id().to_string(),
                    agent_id: Some(agent_id.clone()),
                    details: "session prompt state has no matching session agent".to_string(),
                });
            }
        }
        for (agent_id, prompt_state) in session.prompt_states() {
            if let Some(active_prompt) = prompt_state.active_prompt() {
                check_prompt_targets_prompt_state_agent(
                    &mut mismatches,
                    session.id(),
                    agent_id,
                    "active",
                    active_prompt,
                );
            }
            for queued_prompt in prompt_state.queued_prompts() {
                check_prompt_targets_prompt_state_agent(
                    &mut mismatches,
                    session.id(),
                    agent_id,
                    "queued",
                    queued_prompt,
                );
            }
        }

        for (agent_id, (active_prompt, next_queued_prompt, queued_prompt_count)) in
            expected_prompt_states
        {
            checked_agents += 1;
            let Some(projection) = agent_projections.remove(&agent_id) else {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "missing_agent_runtime_projection".to_string(),
                    session_id: session.id().to_string(),
                    agent_id: Some(agent_id),
                    details: "session projection has no matching agent runtime projection"
                        .to_string(),
                });
                continue;
            };
            if projection.session_id != session.id() {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "agent_runtime_session_mismatch".to_string(),
                    session_id: session.id().to_string(),
                    agent_id: Some(agent_id.clone()),
                    details: format!(
                        "agent runtime projection points at session {}",
                        projection.session_id
                    ),
                });
            }
            if projection.active_prompt != active_prompt {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "active_prompt_mismatch".to_string(),
                    session_id: session.id().to_string(),
                    agent_id: Some(agent_id.clone()),
                    details: format!(
                        "session active {}, agent runtime active {}",
                        describe_projected_prompt(&active_prompt),
                        describe_projected_prompt(&projection.active_prompt)
                    ),
                });
            }
            if projection.next_queued_prompt != next_queued_prompt {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "queue_front_mismatch".to_string(),
                    session_id: session.id().to_string(),
                    agent_id: Some(agent_id.clone()),
                    details: format!(
                        "session queue front {}, agent runtime queue front {}",
                        describe_projected_prompt(&next_queued_prompt),
                        describe_projected_prompt(&projection.next_queued_prompt)
                    ),
                });
            }
            if projection.queued_prompt_count != queued_prompt_count {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "queued_prompt_count_mismatch".to_string(),
                    session_id: session.id().to_string(),
                    agent_id: Some(agent_id),
                    details: format!(
                        "session queued count {}, agent runtime queued count {}",
                        queued_prompt_count, projection.queued_prompt_count
                    ),
                });
            }
        }
    }

    for projection in agent_projections.into_values() {
        mismatches.push(ProjectionInvariantMismatch {
            kind: "orphaned_agent_runtime_projection".to_string(),
            session_id: projection.session_id.clone(),
            agent_id: Some(projection.agent_id.clone()),
            details: "agent runtime projection has no matching projected session agent".to_string(),
        });
    }

    let mut active_turn_provider_runs_by_agent = BTreeMap::<(String, String), Vec<String>>::new();
    for (provider_run_id, active_turn) in active_turns {
        active_turn_provider_runs_by_agent
            .entry((active_turn.session_id.clone(), active_turn.agent_id.clone()))
            .or_default()
            .push(provider_run_id.clone());
    }
    for ((session_id, agent_id), provider_run_ids) in active_turn_provider_runs_by_agent {
        if provider_run_ids.len() > 1 {
            mismatches.push(ProjectionInvariantMismatch {
                kind: "duplicate_active_turns_for_agent".to_string(),
                session_id,
                agent_id: Some(agent_id),
                details: format!(
                    "agent has multiple active turns for provider runs {}",
                    provider_run_ids.join(", ")
                ),
            });
        }
    }

    for (provider_run_id, active_turn) in active_turns {
        if provider_run_id != &active_turn.provider_run_id {
            mismatches.push(ProjectionInvariantMismatch {
                kind: "active_turn_provider_run_key_mismatch".to_string(),
                session_id: active_turn.session_id.clone(),
                agent_id: Some(active_turn.agent_id.clone()),
                details: format!(
                    "active turn stored under provider run {provider_run_id} but points at provider run {}",
                    active_turn.provider_run_id
                ),
            });
        }
        match provider_runs_by_id.get(&active_turn.provider_run_id) {
            None => mismatches.push(ProjectionInvariantMismatch {
                kind: "active_turn_missing_provider_run".to_string(),
                session_id: active_turn.session_id.clone(),
                agent_id: Some(active_turn.agent_id.clone()),
                details: format!(
                    "active turn points at missing provider run {}",
                    active_turn.provider_run_id
                ),
            }),
            Some(run) => {
                if run.state() == ProviderRunState::Ended {
                    mismatches.push(ProjectionInvariantMismatch {
                        kind: "active_turn_ended_provider_run".to_string(),
                        session_id: active_turn.session_id.clone(),
                        agent_id: Some(active_turn.agent_id.clone()),
                        details: format!(
                            "active turn points at ended provider run {}",
                            active_turn.provider_run_id
                        ),
                    });
                }
                if run.session_id() != active_turn.session_id {
                    mismatches.push(ProjectionInvariantMismatch {
                        kind: "active_turn_provider_run_session_mismatch".to_string(),
                        session_id: active_turn.session_id.clone(),
                        agent_id: Some(active_turn.agent_id.clone()),
                        details: format!(
                            "active turn provider run {} points at session {}",
                            active_turn.provider_run_id,
                            run.session_id()
                        ),
                    });
                }
                if run.agent_instance_id() != Some(active_turn.agent_id.as_str()) {
                    mismatches.push(ProjectionInvariantMismatch {
                        kind: "active_turn_provider_run_agent_mismatch".to_string(),
                        session_id: active_turn.session_id.clone(),
                        agent_id: Some(active_turn.agent_id.clone()),
                        details: format!(
                            "active turn provider run {} points at agent {}",
                            active_turn.provider_run_id,
                            run.agent_instance_id().unwrap_or("-")
                        ),
                    });
                }
            }
        }
        let Some(session) = sessions
            .iter()
            .find(|session| session.id() == active_turn.session_id)
        else {
            mismatches.push(ProjectionInvariantMismatch {
                kind: "active_turn_missing_session".to_string(),
                session_id: active_turn.session_id.clone(),
                agent_id: Some(active_turn.agent_id.clone()),
                details: format!(
                    "active turn for provider run {} points at a missing projected session",
                    active_turn.provider_run_id
                ),
            });
            continue;
        };
        if !session
            .agents()
            .iter()
            .any(|agent| agent.id() == active_turn.agent_id)
        {
            mismatches.push(ProjectionInvariantMismatch {
                kind: "active_turn_missing_session_agent".to_string(),
                session_id: active_turn.session_id.clone(),
                agent_id: Some(active_turn.agent_id.clone()),
                details: format!(
                    "active turn for provider run {} points at an agent missing from the projected session",
                    active_turn.provider_run_id
                ),
            });
        }
        if let Some(active_prompt) = session
            .prompt_states()
            .get(&active_turn.agent_id)
            .and_then(|state| state.active_prompt())
        {
            if !prompt_matches_active_turn(active_prompt, &active_turn.prompt_id) {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "active_turn_active_prompt_mismatch".to_string(),
                    session_id: active_turn.session_id.clone(),
                    agent_id: Some(active_turn.agent_id.clone()),
                    details: format!(
                        "active turn prompt {} does not match active prompt {}",
                        active_turn.prompt_id,
                        describe_prompt_with_pending_id(active_prompt)
                    ),
                });
            }
            if active_turn.prompt_origin != Some(active_prompt.prompt_origin())
                && (active_turn.prompt_origin.is_some() || active_prompt.is_external())
            {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "active_turn_prompt_origin_mismatch".to_string(),
                    session_id: active_turn.session_id.clone(),
                    agent_id: Some(active_turn.agent_id.clone()),
                    details: format!(
                        "active turn prompt origin {} does not match active prompt origin {}",
                        describe_prompt_origin(active_turn.prompt_origin),
                        describe_prompt_origin(Some(active_prompt.prompt_origin()))
                    ),
                });
            }
            if active_turn.source_attachment_id.as_deref()
                != Some(active_prompt.source_attachment_id())
            {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "active_turn_source_attachment_mismatch".to_string(),
                    session_id: active_turn.session_id.clone(),
                    agent_id: Some(active_turn.agent_id.clone()),
                    details: format!(
                        "active turn source attachment {} does not match active prompt source attachment {}",
                        describe_source_attachment(active_turn.source_attachment_id.as_deref()),
                        active_prompt.source_attachment_id()
                    ),
                });
            }
            let active_turn_external_observed_id = active_turn.external_observed_id.as_ref();
            let active_prompt_external_observed_id = active_prompt.external_observed_id();
            if active_turn_external_observed_id != active_prompt_external_observed_id.as_ref()
                && (active_turn_external_observed_id.is_some()
                    || active_prompt_external_observed_id.is_some())
            {
                mismatches.push(ProjectionInvariantMismatch {
                    kind: "active_turn_external_identity_mismatch".to_string(),
                    session_id: active_turn.session_id.clone(),
                    agent_id: Some(active_turn.agent_id.clone()),
                    details: format!(
                        "active turn external identity {} does not match active prompt external identity {}",
                        describe_external_observed_id(active_turn_external_observed_id),
                        describe_external_observed_id(active_prompt_external_observed_id.as_ref())
                    ),
                });
            }
        }
    }

    for agent in canonical_agents {
        if !session_ids.contains(agent.session_id()) {
            mismatches.push(ProjectionInvariantMismatch {
                kind: "agent_record_missing_projected_session".to_string(),
                session_id: agent.session_id().to_string(),
                agent_id: Some(agent.id().to_string()),
                details:
                    "canonical agent record points at a session missing from the session projection"
                        .to_string(),
            });
            continue;
        }
        if projected_session_agents.get(agent.id()).map(String::as_str) != Some(agent.session_id())
        {
            mismatches.push(ProjectionInvariantMismatch {
                kind: "agent_record_not_in_session_projection".to_string(),
                session_id: agent.session_id().to_string(),
                agent_id: Some(agent.id().to_string()),
                details:
                    "canonical agent record is not present in its projected session agent list"
                        .to_string(),
            });
        }
    }

    ProjectionInvariantHealthSnapshot {
        checked_sessions: sessions.len(),
        checked_agents,
        mismatches,
    }
}

fn describe_projected_prompt(prompt: &Option<PromptQueueItem>) -> String {
    prompt
        .as_ref()
        .map(|prompt| prompt.id().to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn check_prompt_targets_prompt_state_agent(
    mismatches: &mut Vec<ProjectionInvariantMismatch>,
    session_id: &str,
    prompt_state_agent_id: &str,
    prompt_slot: &str,
    prompt: &PromptQueueItem,
) {
    if prompt.target_agent_id() == prompt_state_agent_id {
        return;
    }
    mismatches.push(ProjectionInvariantMismatch {
        kind: "prompt_state_prompt_target_mismatch".to_string(),
        session_id: session_id.to_string(),
        agent_id: Some(prompt_state_agent_id.to_string()),
        details: format!(
            "{prompt_slot} prompt {} targets agent {}",
            prompt.id(),
            prompt.target_agent_id()
        ),
    });
}

fn prompt_matches_active_turn(prompt: &PromptQueueItem, active_turn_prompt_id: &str) -> bool {
    prompt.id() == active_turn_prompt_id
        || prompt.pending_prompt_id() == Some(active_turn_prompt_id)
}

fn describe_prompt_with_pending_id(prompt: &PromptQueueItem) -> String {
    match prompt.pending_prompt_id() {
        Some(pending_prompt_id) => {
            format!("{} (pending {})", prompt.id(), pending_prompt_id)
        }
        None => prompt.id().to_string(),
    }
}

fn describe_prompt_origin(prompt_origin: Option<crate::session::PromptOrigin>) -> &'static str {
    match prompt_origin {
        None => "none",
        Some(prompt_origin) => prompt_origin_label(prompt_origin),
    }
}

fn describe_source_attachment(source_attachment_id: Option<&str>) -> &str {
    source_attachment_id.unwrap_or("none")
}

fn prompt_origin_label(prompt_origin: crate::session::PromptOrigin) -> &'static str {
    match prompt_origin {
        crate::session::PromptOrigin::Arroba => "arroba",
        crate::session::PromptOrigin::External => "external",
    }
}

fn describe_external_observed_id(
    observed_id: Option<&crate::history::ExternalProviderObservedId>,
) -> String {
    observed_id
        .map(|observed_id| {
            format!(
                "{}:{}:{}",
                observed_id.provider, observed_id.provider_session_id, observed_id.provider_turn_id
            )
        })
        .unwrap_or_else(|| "none".to_string())
}

#[cfg(test)]
mod tests;
