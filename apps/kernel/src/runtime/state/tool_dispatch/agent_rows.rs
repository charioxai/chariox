//! R3: a Room's agent list is one compact row per agent, in pages. The row names who the agent
//! is and where it stands; `get_session_agent` has the settings, liveness and placement.
use crate::runtime::projection::{agent_state_label, agent_turn_state, AgentRuntimeActivity};
use crate::transport::runtime_tools::AgentPageArgs;
use std::collections::BTreeMap;

const DEFAULT_ROWS: usize = 40;
const MAX_ROWS: usize = 200;

pub(super) struct AgentPage<'a> {
    pub agents: Vec<&'a crate::agent::AgentInstance>,
    /// Agents that match the filter, over every page.
    pub total: usize,
    pub next_cursor: Option<String>,
}

/// The page `args` asks for, in creation order. The cursor is the id of the last agent of the
/// previous page, so agents created between two reads move no row.
pub(super) fn page_of_agents<'a>(
    agents: &'a [crate::agent::AgentInstance],
    args: &AgentPageArgs,
) -> Result<AgentPage<'a>, String> {
    let needle = args
        .alias
        .as_deref()
        .map(|alias| alias.trim().trim_start_matches('@').to_lowercase())
        .filter(|alias| !alias.is_empty());
    let mut matching = agents
        .iter()
        .filter(|agent| {
            needle.as_ref().is_none_or(|needle| {
                agent
                    .alias()
                    .is_some_and(|alias| alias.to_lowercase().contains(needle))
            })
        })
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| {
        left.created_at_ms()
            .cmp(&right.created_at_ms())
            .then_with(|| left.id().cmp(right.id()))
    });
    let total = matching.len();
    let start = match args.cursor.as_deref() {
        None => 0,
        Some(cursor) => {
            matching
                .iter()
                .position(|agent| agent.id() == cursor)
                .ok_or_else(|| {
                    format!(
                    "cursor {cursor} names no agent of this list; repeat the call without a cursor"
                )
                })?
                + 1
        }
    };
    let limit = args.limit.unwrap_or(DEFAULT_ROWS).clamp(1, MAX_ROWS);
    let end = (start + limit).min(total);
    let next_cursor = (end < total).then(|| matching[end - 1].id().to_string());
    Ok(AgentPage {
        agents: matching[start..end].to_vec(),
        total,
        next_cursor,
    })
}

/// Who the agent is and where it stands, and nothing that grows with its history.
pub(super) fn compact_agent_row(
    session: &crate::session::RuntimeSession,
    agent: &crate::agent::AgentInstance,
    aliases: &BTreeMap<&str, &str>,
    activity: &BTreeMap<String, AgentRuntimeActivity>,
) -> serde_json::Value {
    let state = activity.get(agent.id());
    let mut row = serde_json::json!({
        "id": agent.id(),
        "alias": agent.alias(),
        "state": agent_state_label(agent, state),
        "turn": agent_turn_state(session.active_prompt_for_agent(agent.id()).is_some(), state),
        "provider": agent.provider(),
        "model": agent.model(),
    });
    if let Some(parent) = agent.parent_agent_id() {
        row["parent"] = serde_json::json!(aliases.get(parent).copied().unwrap_or(parent));
    }
    row
}

pub(super) fn alias_index(agents: &[crate::agent::AgentInstance]) -> BTreeMap<&str, &str> {
    agents
        .iter()
        .filter_map(|agent| Some((agent.id(), agent.alias()?)))
        .collect()
}
