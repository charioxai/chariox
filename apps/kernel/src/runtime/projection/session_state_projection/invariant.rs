//! Session and agent runtime projection invariant checks.

use std::collections::BTreeMap;

use crate::session::{PromptQueueItem, RuntimeSession};

use super::super::{
    AgentRuntimeProjectionStore, ProjectionInvariantHealthSnapshot, ProjectionInvariantMismatch,
};

pub(super) fn snapshot(
    sessions: Vec<RuntimeSession>,
    agent_runtime: &AgentRuntimeProjectionStore,
) -> ProjectionInvariantHealthSnapshot {
    let mut agent_projections = agent_runtime
        .list()
        .into_iter()
        .map(|projection| (projection.agent_id.clone(), projection))
        .collect::<BTreeMap<_, _>>();
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
        for agent in session.agents() {
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
        for (agent_id, prompt_state) in session.prompt_states() {
            expected_prompt_states
                .entry(agent_id.clone())
                .or_insert_with(|| {
                    (
                        prompt_state.active_prompt().cloned(),
                        prompt_state.queued_prompts().front().cloned(),
                        prompt_state.queued_prompts().len(),
                    )
                });
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
        let clean_snapshot = session_store.invariant_snapshot(&agent_store);
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

        let drift_snapshot = session_store.invariant_snapshot(&agent_store);
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

        let snapshot = session_store.invariant_snapshot(&agent_store);

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
}
