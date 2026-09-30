use std::collections::BTreeSet;

use super::{RuntimeSession, SessionCollaborationAgentCounts};
use crate::session::CollaborationLevel;

pub(super) fn redacted_for_user(mut session: RuntimeSession, user_id: &str) -> RuntimeSession {
    let collaboration_level = session
        .collaboration_level_for_user(user_id)
        .unwrap_or(CollaborationLevel::Private);
    let total_agent_count = session.agents.len();
    let owned_agent_count = session
        .agents
        .iter()
        .filter(|agent| agent.owner_user_id() == user_id)
        .count();
    let collaborator_count = session
        .members
        .iter()
        .filter(|member| member.user_id() != user_id)
        .count();
    session.collaboration_agent_counts = Some(SessionCollaborationAgentCounts {
        owned_agent_count,
        other_user_agent_count: total_agent_count.saturating_sub(owned_agent_count),
        total_agent_count,
        collaborator_count,
    });
    let has_unowned_agents = session
        .agents
        .iter()
        .any(|agent| agent.owner_user_id() != user_id);
    let owned_agent_ids = session
        .agents
        .iter()
        .filter(|agent| agent.owner_user_id() == user_id)
        .map(|agent| agent.id().to_string())
        .collect::<BTreeSet<_>>();
    let visible_agent_ids = if collaboration_level.can_view_agent_trace() {
        session
            .agents
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<BTreeSet<_>>()
    } else {
        owned_agent_ids.clone()
    };
    if collaboration_level.can_view_agent_parameters() {
        session.agents.retain(|agent| {
            collaboration_level.can_view_agent_trace() || agent.owner_user_id() == user_id
        });
        for agent in &mut session.agents {
            agent.set_visible_in_freeform(visible_agent_ids.contains(agent.id()));
        }
    } else {
        session.agents = session
            .agents
            .into_iter()
            .map(|agent| {
                if agent.owner_user_id() == user_id {
                    let mut agent = agent;
                    agent.set_visible_in_freeform(true);
                    agent
                } else {
                    let mut agent = agent.redacted_parameters();
                    agent.set_visible_in_freeform(visible_agent_ids.contains(agent.id()));
                    agent
                }
            })
            .collect();
    }
    if session
        .focused_agent_id
        .as_ref()
        .is_some_and(|agent_id| !visible_agent_ids.contains(agent_id))
    {
        session.focused_agent_id = None;
    }
    if has_unowned_agents && !collaboration_level.can_view_agent_trace() {
        session.active_provider_run_id = None;
    }
    session
        .prompt_runtime
        .retain_agent_ids(&visible_agent_ids, session.focused_agent_id.as_deref());
    session
        .metaagent_tasks
        .retain(|task| visible_agent_ids.contains(task.metaagent_id()));
    if !collaboration_level.can_view_agent_trace() {
        session.workflows = session
            .workflows
            .into_iter()
            .map(|workflow| workflow.redacted_for_user(user_id))
            .collect();
    }
    session
        .workflow_publication_state
        .workflow_publications
        .retain(|publication| publication.created_by_user_id() == user_id);
    session
        .workflow_publication_state
        .workflow_publication_snapshots
        .clear();
    session.agent_output_read_state.clear();
    session
}
