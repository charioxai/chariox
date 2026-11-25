use super::list_page::{self, CreationKey, LIST_BYTES, LIST_ROWS};
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, AgentTaskExecution};
use crate::transport::runtime_tools::RuntimeToolResult;
use serde_json::{json, Value};

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
                list_page::page(task.obligations.into_iter().enumerate().rev().filter_map(
                    |(index, obligation)| {
                        let position = index + 1;
                        (obligations_cursor.is_none_or(|before| position < before as usize)
                            && (include_history || obligation.requires_resolution()))
                        .then(|| (position, json!(obligation)))
                    },
                ))?;
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
            let page = list_page::page(agents.iter().filter_map(|agent| {
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
                    Ok(description) => Some((
                        key,
                        list_page::bounded_row(
                            description,
                            &[
                                "id",
                                "agent_ref",
                                "alias",
                                "address",
                                "state",
                                "queued_prompt_count",
                            ],
                        ),
                    )),
                    Err(error) => {
                        failure.get_or_insert(error);
                        None
                    }
                }
            }));
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
            list_page::page(
                tasks
                    .into_iter()
                    .map(|(key, task)| (key, task_listing_row(task, include_history, LIST_BYTES))),
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
