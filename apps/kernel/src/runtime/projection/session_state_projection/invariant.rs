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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::runtime::projection::test_support::{launch_dev_stub_provider, submit_prompt};
    use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn projection_invariant_health_reports_agent_runtime_drift() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-projection-invariant",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_dev_stub_provider(&mut app, &session_id, &agent_id);
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "active prompt",
        );
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "queued prompt",
        );

        let session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("session snapshot should load");
        let session_store = SessionStateProjectionStore::default();
        let agent_store = AgentRuntimeProjectionStore::default();
        session_store.update(session.clone());
        agent_store.update_session(&session);
        let active_turns = BTreeMap::new();
        let provider_runs = Vec::new();
        let clean_snapshot =
            session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
        assert_eq!(clean_snapshot.checked_sessions, 1);
        assert_eq!(clean_snapshot.checked_agents, 1);
        assert!(clean_snapshot.mismatches.is_empty());

        let projection = agent_store
            .get(&agent_id)
            .expect("agent projection should exist before drift injection");
        agent_store.update_agent_prompt_state(
            &session_id,
            &agent_id,
            projection.active_prompt,
            None,
            0,
        );

        let drift_snapshot =
            session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
        assert!(drift_snapshot
            .mismatches
            .iter()
            .any(|mismatch| mismatch.kind == "queue_front_mismatch"));
        assert!(drift_snapshot
            .mismatches
            .iter()
            .any(|mismatch| mismatch.kind == "queued_prompt_count_mismatch"));
    }

    #[test]
    fn projection_invariant_health_reports_stale_focused_agent() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let mut session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(session.id())
            .expect("session snapshot should load");
        session.set_focused_agent(Some("missing-agent".to_string()));

        let session_store = SessionStateProjectionStore::default();
        let agent_store = AgentRuntimeProjectionStore::default();
        session_store.update(session.clone());
        agent_store.update_session(&session);

        let active_turns = BTreeMap::new();
        let provider_runs = Vec::new();
        let snapshot =
            session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);

        assert_eq!(snapshot.checked_sessions, 1);
        assert_eq!(snapshot.checked_agents, 1);
        assert!(snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "stale_focused_agent"
                && mismatch.session_id == session.id()
                && mismatch.agent_id.as_deref() == Some("missing-agent")
                && mismatch.details == "focused agent is not present in the session agent list"
        }));
        assert!(!snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "missing_agent_runtime_projection"
                && mismatch.agent_id.as_deref() == Some(agent.id())
        }));
    }

    #[test]
    fn projection_invariant_health_reports_prompt_state_without_session_agent() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-projection-invariant-prompt-state-only",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_dev_stub_provider(&mut app, &session_id, &agent_id);
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "active prompt",
        );

        let mut session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("session snapshot should load");
        assert!(
            session.prompt_states().contains_key(&agent_id),
            "fixture should retain prompt state before removing projected agents"
        );
        session.set_agents(Vec::new());

        let session_store = SessionStateProjectionStore::default();
        let agent_store = AgentRuntimeProjectionStore::default();
        session_store.update(session.clone());
        agent_store.update_session(&session);

        let active_turns = BTreeMap::new();
        let provider_runs = Vec::new();
        let snapshot =
            session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);

        assert_eq!(snapshot.checked_sessions, 1);
        assert_eq!(snapshot.checked_agents, 0);
        assert!(agent_store.get(&agent_id).is_none());
        assert!(snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "prompt_state_without_session_agent"
                && mismatch.session_id == session_id
                && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
                && mismatch.details == "session prompt state has no matching session agent"
        }));
        assert!(!snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "orphaned_agent_runtime_projection"
                && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
        }));
    }

    #[test]
    fn projection_invariant_health_reports_active_turn_membership_drift() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let valid_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
        let wrong_agent_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
        let mut ended_run = launch_dev_stub_provider(&mut app, session.id(), agent.id());
        ended_run.mark_ended();
        let session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(session.id())
            .expect("session snapshot should load");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let provider_runs = vec![
            valid_run.clone(),
            wrong_agent_run.clone(),
            ended_run.clone(),
        ];

        let session_store = SessionStateProjectionStore::default();
        let agent_store = AgentRuntimeProjectionStore::default();
        session_store.update(session.clone());
        agent_store.update_session(&session);
        let mut active_turns = BTreeMap::new();
        active_turns.insert(
            valid_run.id().to_string(),
            crate::app::ActiveTurnState::new(
                session_id.clone(),
                agent_id.clone(),
                "prompt-valid".to_string(),
                valid_run.id().to_string(),
            ),
        );

        let clean_snapshot =
            session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);
        assert!(
            clean_snapshot.mismatches.is_empty(),
            "{:?}",
            clean_snapshot.mismatches
        );

        active_turns.insert(
            "run-missing-session".to_string(),
            crate::app::ActiveTurnState::new(
                "missing-session".to_string(),
                agent_id.clone(),
                "prompt-missing-session".to_string(),
                "run-missing-session".to_string(),
            ),
        );
        active_turns.insert(
            wrong_agent_run.id().to_string(),
            crate::app::ActiveTurnState::new(
                session_id.clone(),
                "missing-agent".to_string(),
                "prompt-missing-agent".to_string(),
                wrong_agent_run.id().to_string(),
            ),
        );
        active_turns.insert(
            ended_run.id().to_string(),
            crate::app::ActiveTurnState::new(
                session_id.clone(),
                agent_id.clone(),
                "prompt-ended-run".to_string(),
                ended_run.id().to_string(),
            ),
        );

        let drift_snapshot =
            session_store.invariant_snapshot(&agent_store, &[], &active_turns, &provider_runs);

        assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "active_turn_missing_provider_run"
                && mismatch.session_id == "missing-session"
                && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
                && mismatch.details.contains("run-missing-session")
        }));
        assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "active_turn_missing_session"
                && mismatch.session_id == "missing-session"
                && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
                && mismatch.details.contains("run-missing-session")
        }));
        assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "active_turn_missing_session_agent"
                && mismatch.session_id == session_id
                && mismatch.agent_id.as_deref() == Some("missing-agent")
                && mismatch.details.contains(wrong_agent_run.id())
        }));
        assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "active_turn_provider_run_agent_mismatch"
                && mismatch.session_id == session_id
                && mismatch.agent_id.as_deref() == Some("missing-agent")
                && mismatch.details.contains(wrong_agent_run.id())
        }));
        assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "active_turn_ended_provider_run"
                && mismatch.session_id == session_id
                && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
                && mismatch.details.contains(ended_run.id())
        }));
        assert!(drift_snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "duplicate_active_turns_for_agent"
                && mismatch.session_id == session_id
                && mismatch.agent_id.as_deref() == Some(agent_id.as_str())
                && mismatch.details.contains(valid_run.id())
                && mismatch.details.contains(ended_run.id())
        }));
    }

    #[test]
    fn projection_invariant_health_reports_canonical_agent_membership_drift() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (first_session, first_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("first session should be created");
        let (second_session, _second_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-2", "worktree-2"))
            .expect("second session should be created");
        let second_session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(second_session.id())
            .expect("second session snapshot should load");
        let ghost_agent = crate::agent::AgentInstance::new(
            "ghost-agent",
            "ghost-agent",
            second_session.id(),
            None,
            "dev-stub",
            Some("default".to_string()),
            None,
            Some(second_session.worktree_id().to_string()),
            crate::agent::GridPosition::new(0, 0, 1, 1),
        );
        let session_store = SessionStateProjectionStore::default();
        let agent_store = AgentRuntimeProjectionStore::default();
        session_store.update(second_session.clone());
        agent_store.update_session(&second_session);

        let active_turns = BTreeMap::new();
        let provider_runs = Vec::new();
        let snapshot = session_store.invariant_snapshot(
            &agent_store,
            &[first_agent.clone(), ghost_agent.clone()],
            &active_turns,
            &provider_runs,
        );

        assert!(snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "agent_record_missing_projected_session"
                && mismatch.session_id == first_session.id()
                && mismatch.agent_id.as_deref() == Some(first_agent.id())
        }));
        assert!(snapshot.mismatches.iter().any(|mismatch| {
            mismatch.kind == "agent_record_not_in_session_projection"
                && mismatch.session_id == second_session.id()
                && mismatch.agent_id.as_deref() == Some(ghost_agent.id())
        }));
    }
}
