use super::list_page::{self, LIST_ROWS};
use super::session_agent_listing::task_listing_row;
use super::*;
use crate::durable_state::agent_lifecycle::AgentTaskExecution;
use serde_json::{json, Value};

impl KernelRuntimeState {
    pub(super) fn agent_subscription_listing(
        &self,
        task: &AgentTaskExecution,
        args: &Value,
    ) -> Result<Value, DaemonError> {
        let history = list_page::include_history(args)?;
        let current = list_page::cursor_number(args, "registrations_cursor")?;
        let others = list_page::cursor_number(args, "other_tasks_cursor")?;
        let store = &self.owned.durable_state_store;
        let read = |before, other_tasks| {
            let rows = store.agent_registration_page(
                (&task.room_id, &task.agent_id),
                &task.task_id,
                before,
                other_tasks,
                history,
                LIST_ROWS + 1,
            )?;
            // Include the task's summary with both registration lists.
            list_page::page_with_budget(
                rows.into_iter().map(|(key, row)| (key, json!(row))),
                22 * 1024,
            )
        };
        let (registrations, next_registrations_cursor) = if current.is_some() || others.is_none() {
            read(current, false)?
        } else {
            (Vec::new(), None)
        };
        let (other_tasks, next_other_tasks_cursor) = if others.is_some() || current.is_none() {
            read(others, true)?
        } else {
            (Vec::new(), None)
        };
        // Row 470: every wait of this agent's other tasks, read-only: a retained watch of a task
        // that still waits is listed here even where its registrations are not.
        let waits: Vec<Value> = store
            .agent_tasks(Some(&task.room_id), Some(&task.agent_id))?
            .iter()
            .filter(|t| t.task_id != task.task_id && t.wait.is_some())
            .filter(|t| t.state == crate::durable_state::agent_lifecycle::ExecutionState::Waiting)
            .take(32)
            .map(super::agent_wake_listing::wait_row)
            .collect();
        Ok(json!({
            "waits": waits,
            "task": task_listing_row(task.clone(), history, 16 * 1024),
            "registrations": registrations, "next_registrations_cursor": next_registrations_cursor,
            "other_tasks": other_tasks, "next_other_tasks_cursor": next_other_tasks_cursor,
        }))
    }
}
