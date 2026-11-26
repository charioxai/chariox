use super::list_page::{self, CreationKey, LIST_ROWS};
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, AgentTaskExecution};
use crate::transport::runtime_tools::RuntimeToolResult;
use serde_json::{json, Value};

/// A page holds at most two parts (agents, tasks) and stays under 16 KiB in the agent's context.
const PART_BYTES: usize = 7 * 1024;
/// A task's obligations come alone in their page.
const OBLIGATIONS_BYTES: usize = 14 * 1024;
/// The longest waiting-on detail a listing row carries.
const WAITING_ON_CHARS: usize = 80;

impl KernelRuntimeState {
    pub(super) fn handle_list_session_agents_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        requester: &crate::agent::AgentInstance,
        args: &Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let agent_cursor = CreationKey::cursor(args, "agents_cursor")?;
        let task_cursor = list_page::cursor_number(args, "tasks_cursor")?;
        let obligations_cursor = list_page::cursor_number(args, "obligations_cursor")?;
        let include_history = list_page::include_history(args)?;
        let store = &self.owned.durable_state_store;
        if let Some(task_id) = args.get("task_id") {
            let task_id = task_id
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ledger::error("task_id must identify a task in this Room"))?;
            if agent_cursor.is_some() || task_cursor.is_some() {
                return Err(ledger::error(
                    "task_id cannot be combined with agents_cursor or tasks_cursor",
                ));
            }
            let task = store
                .agent_task(task_id)?
                .filter(|task| task.room_id == session.id())
                .ok_or_else(|| ledger::error("task_id must identify a task in this Room"))?;
            let (obligations, next_obligations_cursor) =
                list_page::page_with_budget(
                    task.obligations.into_iter().enumerate().rev().filter_map(
                        |(index, obligation)| {
                            let position = index + 1;
                            (obligations_cursor.is_none_or(|before| position < before as usize)
                                && (include_history || obligation.requires_resolution()))
                            .then(|| (position, json!(obligation)))
                        },
                    ),
                    OBLIGATIONS_BYTES,
                )?;
            return Ok(RuntimeToolResult {
                ok: true,
                payload: json!({"task_id": task_id, "obligations": obligations,
                                "next_obligations_cursor": next_obligations_cursor}),
            });
        }
        if obligations_cursor.is_some() {
            return Err(ledger::error("obligations_cursor requires task_id"));
        }

        let (agents, next_agents_cursor) = if agent_cursor.is_some() || task_cursor.is_none() {
            let mut agents = self.session_agents(session.id());
            agents.sort_unstable_by(|left, right| {
                (right.created_at_ms(), right.id()).cmp(&(left.created_at_ms(), left.id()))
            });
            let activity = self.agent_activity_for_session(session);
            let mut failure = None;
            let page = list_page::page_with_budget(
                agents.iter().filter_map(|agent| {
                    let key = CreationKey(agent.created_at_ms(), agent.id().into());
                    if agent_cursor.as_ref().is_some_and(|before| &key >= before) {
                        return None;
                    }
                    match self.session_agent_description(
                        session,
                        requester,
                        agent,
                        activity.get(agent.id()),
                    ) {
                        Ok(description) => Some((key, agent_listing_row(&description))),
                        Err(error) => {
                            failure.get_or_insert(error);
                            None
                        }
                    }
                }),
                PART_BYTES,
            );
            if let Some(error) = failure {
                return Err(error);
            }
            page?
        } else {
            (Vec::new(), None)
        };
        let (tasks, next_tasks_cursor) = if task_cursor.is_some() || agent_cursor.is_none() {
            let tasks = store.room_agent_task_page(
                session.id(),
                task_cursor,
                include_history,
                LIST_ROWS + 1,
            )?;
            list_page::page_with_budget(
                tasks
                    .into_iter()
                    .map(|(key, task)| (key, task_listing_row(task, include_history, PART_BYTES))),
                PART_BYTES,
            )?
        } else {
            (Vec::new(), None)
        };
        Ok(RuntimeToolResult {
            ok: true,
            payload: json!({
                "session_id": session.id(), "focused_agent_id": session.focused_agent_id(),
                "agents": agents, "next_agents_cursor": next_agents_cursor.map(CreationKey::encode),
                "agent_tasks": tasks, "next_tasks_cursor": next_tasks_cursor,
            }),
        })
    }
}

/// What an orchestrator needs to pick and watch an agent; `chariox.get_session_agent` has the rest.
fn agent_listing_row(description: &Value) -> Value {
    let waiting_on = description["liveness"]["waiting_on"]
        .as_object()
        .map(|waiting| {
            let kind = waiting["kind"].as_str().unwrap_or_default();
            match waiting.get("detail").and_then(Value::as_str) {
                Some(detail) if !detail.is_empty() => {
                    let detail: String = detail.chars().take(WAITING_ON_CHARS).collect();
                    format!("{kind}: {detail}")
                }
                _ => kind.to_owned(),
            }
        });
    let mut row = json!({
        "id": description["id"],
        "alias": description["alias"],
        "state": description["state"],
        "provider": description["provider"],
        "model": description["model"],
        "execution_mode": description["execution_mode"],
        "permission_level": description["permission_level"],
        "parent_agent_id": description["parent_agent_id"],
        "has_active_prompt": description["has_active_prompt"],
        "queued_prompt_count": description["queued_prompt_count"],
        "waiting_on": waiting_on,
    });
    // The agent's own `liveness` has the whole wait; the row keeps the two readings that tell a
    // watcher an agent went quiet: when it last wrote and whether it is flagged stalled.
    let liveness = &description["liveness"];
    let mut quiet_signal = serde_json::Map::new();
    for field in ["last_activity_at_ms", "stalled"] {
        if !liveness[field].is_null() {
            quiet_signal.insert(field.into(), liveness[field].clone());
        }
    }
    if !quiet_signal.is_empty() {
        row["liveness"] = quiet_signal.into();
    }
    row
}

pub(super) fn task_listing_row(
    mut task: AgentTaskExecution,
    include_history: bool,
    budget: usize,
) -> Value {
    if !include_history {
        task.obligations
            .retain(|obligation| obligation.requires_resolution());
    }
    let mut row = json!(task);
    if row.to_string().len() + 3 > budget {
        let obligations = row["obligations"].as_array().map_or(0, Vec::len);
        row["obligations"] = json!([]);
        row["obligations_omitted"] = json!(obligations);
        if row.to_string().len() + 3 > budget {
            row = json!({
                "task_id": task.task_id, "agent_id": task.agent_id, "state": task.state,
                "obligations": [], "obligations_omitted": obligations, "details_omitted": true,
            });
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    fn description(liveness: Value) -> Value {
        json!({
            "id": "agent-1", "alias": "builder", "state": "working", "provider": "codex",
            "model": "m", "execution_mode": "act", "permission_level": "full",
            "parent_agent_id": null, "has_active_prompt": true, "queued_prompt_count": 0,
            "liveness": liveness,
            "address": "@builder", "location": {"kind": "local"}, "is_self": false,
        })
    }

    /// A quiet working agent is flagged in the compact row itself: an orchestrator reading the
    /// list sees the stall and when the agent last wrote, without a detail call per agent.
    #[test]
    fn a_stalled_agent_row_carries_its_stall_and_last_activity_only() {
        let row = agent_listing_row(&description(json!({
            "last_activity_at_ms": 1_700_000_000_000_u64,
            "stalled": true,
            "waiting_on": {"kind": "provider", "since_ms": 1_699_999_000_000_u64},
            "waiting_events": [{"label": "a long wait label the row does not carry"}],
        })));
        assert_eq!(
            row["liveness"],
            json!({"last_activity_at_ms": 1_700_000_000_000_u64, "stalled": true})
        );
        assert_eq!(row["waiting_on"], "provider");
        for detail_field in ["address", "location", "is_self"] {
            assert!(row.get(detail_field).is_none(), "{detail_field} in the row");
        }
    }

    #[test]
    fn a_row_reads_back_as_the_liveness_the_agent_shows_elsewhere() {
        let row = agent_listing_row(&description(json!({
            "last_activity_at_ms": 40_000_u64, "stalled": false,
        })));
        let liveness: crate::runtime::projection::AgentLiveness =
            serde_json::from_value(row["liveness"].clone()).unwrap();
        assert_eq!(liveness.last_activity_at_ms, Some(40_000));
        assert!(!liveness.stalled);
    }

    #[test]
    fn a_quiet_agent_row_has_no_liveness_key() {
        for liveness in [json!(null), json!({})] {
            let row = agent_listing_row(&description(liveness));
            assert!(row.get("liveness").is_none(), "{row}");
        }
    }
}
