use std::collections::BTreeSet;
use std::sync::Arc;

use super::{RuntimeSession, SessionCollaborationAgentCounts};
use crate::session::CollaborationLevel;

pub(super) fn redacted_for_user(mut session: RuntimeSession, user_id: &str) -> RuntimeSession {
    // A4: sub-agents are hidden before anything is counted or redacted (redaction clears the edge).
    Arc::make_mut(&mut session.agents).retain(|agent| agent.is_room_agent());
    session.agents = Arc::new(
        Arc::unwrap_or_clone(session.agents)
            .into_iter()
            .map(crate::agent::AgentInstance::without_inactive_usage_limit_stop)
            .collect(),
    );
    // MP-08 / MP-10 / MP-11: private runs need participant ownership, too.
    session.workflow_runs = session
        .visible_workflow_runs_for_user(user_id, None)
        .unwrap_or_default();
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
        Arc::make_mut(&mut session.agents).retain(|agent| {
            collaboration_level.can_view_agent_trace() || agent.owner_user_id() == user_id
        });
        for agent in Arc::make_mut(&mut session.agents) {
            agent.set_visible_in_freeform(visible_agent_ids.contains(agent.id()));
        }
    } else {
        session.agents = Arc::new(Arc::unwrap_or_clone(session.agents)
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
            .collect());
    }
    if user_id != session.owner_user_id() {
        for agent in Arc::make_mut(&mut session.agents) {
            if agent.parent_room_id().is_some() {
                agent.set_parent_agent(None);
            }
        }
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
        .prompt_runtime
        .hide_internal_prompts(session.focused_agent_id.as_deref());
    session
        .metaagent_tasks
        .retain(|task| visible_agent_ids.contains(task.metaagent_id()));
    // MP-08/MP-09/MP-10/MP-11: lifecycle reasons and process argv follow trace visibility.
    Arc::make_mut(&mut session.agent_tasks)
        .retain(|task| visible_agent_ids.contains(&task.agent_id));
    Arc::make_mut(&mut session.agent_wakes)
        .retain(|wake| visible_agent_ids.contains(&wake.agent_id));
    // A queued message carries the start of the sender's text for the recipient agent.
    Arc::make_mut(&mut session.queued_agent_messages)
        .retain(|message| visible_agent_ids.contains(&message.agent_id));
    // A trigger and its unconfirmed firings carry the owner's prompt text.
    session
        .agent_triggers_mut()
        .retain(|trigger| visible_agent_ids.contains(trigger.agent_id()));
    session
        .trigger_deliveries
        .retain(|delivery| visible_agent_ids.contains(delivery.agent_id()));
    // A04: window, run and requester details belong to the window's owner.
    session
        .sudo_windows
        .retain(|window| window.owner_user_id == user_id);
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
    session.agent_output_read_state = Arc::default();
    session
}
