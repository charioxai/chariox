//! Lists retained wake sources and their delivery receipts in bounded pages.

use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, AgentTaskExecution, AgentWake};
use serde_json::{json, Value};

// The MCP envelope can carry both JSON and an escaped text copy of the reply.
const PAGE_BYTES: usize = 16 * 1024;

#[derive(serde::Deserialize)]
struct ListArgs {
    #[serde(default)]
    armed_only: bool,
    after: Option<String>,
    limit: Option<u8>,
    wake_id: Option<String>,
}

#[derive(Clone, Copy)]
enum Cursor {
    Wakes(i64),
    Receipts(u64),
}

impl Cursor {
    fn parse(value: &str) -> Option<Self> {
        let (kind, value) = value.split_once(':')?;
        match kind {
            "wakes" => Some(Self::Wakes(value.parse().ok().filter(|after| *after >= 0)?)),
            "receipts" => Some(Self::Receipts(value.parse().ok()?)),
            _ => None,
        }
    }

    fn encode(self) -> String {
        match self {
            Self::Wakes(after) => format!("wakes:{after}"),
            Self::Receipts(after) => format!("receipts:{after}"),
        }
    }
}

struct Page {
    wakes: Vec<Value>,
    receipts: Vec<Value>,
    last: Option<Cursor>,
    bytes_left: usize,
    rows_left: u8,
}

impl Page {
    fn push(&mut self, cursor: Cursor, row: Value) -> Result<bool, DaemonError> {
        let bytes = row.to_string().len() + 1;
        if self.rows_left == 0 || bytes > self.bytes_left {
            if self.last.is_none() {
                return Err(ledger::error("a wake entry exceeds the listing size limit"));
            }
            return Ok(false);
        }
        match cursor {
            Cursor::Wakes(_) => self.wakes.push(row),
            Cursor::Receipts(_) => self.receipts.push(row),
        }
        self.rows_left -= 1;
        self.bytes_left -= bytes;
        self.last = Some(cursor);
        Ok(true)
    }
}

fn wake_entry(wake: &AgentWake) -> Value {
    let mut entry = json!(wake);
    if entry.to_string().len() >= PAGE_BYTES {
        // A watched command or driver detail can be bigger than the entire page.
        entry["command"] = json!([]);
        entry["detail"] = Value::Null;
        entry["details_omitted"] = json!(true);
    }
    entry
}

impl KernelRuntimeState {
    pub(super) fn list_agent_wakes(
        &self,
        task: &AgentTaskExecution,
        args: &Value,
        now: u64,
    ) -> Result<Value, DaemonError> {
        let args: ListArgs = serde_json::from_value(args.clone())
            .map_err(|error| ledger::error(format!("invalid wakes arguments: {error}")))?;
        // Row 470: one wake's state and its task's wait, read without changing either; any task of
        // this agent's, finished or not.
        if let Some(id) = &args.wake_id {
            return self.read_agent_wake(task, id, now);
        }
        let limit = args.limit.unwrap_or(16);
        if !(1..=32).contains(&limit) {
            return Err(ledger::error("wakes limit must be between 1 and 32"));
        }
        let cursor = match args.after {
            None => Cursor::Wakes(0),
            Some(after) => Cursor::parse(&after)
                .filter(|cursor| !args.armed_only || matches!(cursor, Cursor::Wakes(_)))
                .ok_or_else(|| ledger::error("pass next_after from a page with the same filter"))?,
        };
        let mut page = Page {
            wakes: Vec::new(),
            receipts: Vec::new(),
            last: None,
            bytes_left: PAGE_BYTES,
            rows_left: limit,
        };
        let reply = |page: Page, more: bool| {
            json!({
                "now_ms": now,
                "scheduler": self.wake_scheduler_health(now),
                "wakes": page.wakes,
                "receipts": page.receipts,
                "next_after": page.last.filter(|_| more).map(Cursor::encode),
            })
        };
        let store = &self.owned.durable_state_store;
        if let Cursor::Wakes(after) = cursor {
            let wakes = store.agent_visible_wake_rows(
                &task.room_id,
                Some(&task.agent_id),
                now,
                after,
                i64::from(limit) + 1,
                args.armed_only,
            )?;
            for (row, wake) in wakes {
                if !page.push(Cursor::Wakes(row), wake_entry(&wake))? {
                    return Ok(reply(page, true));
                }
            }
        }
        if !args.armed_only {
            let after = match cursor {
                Cursor::Wakes(_) => 0,
                Cursor::Receipts(after) => after,
            };
            let receipts = store.agent_wake_receipt_rows(
                Some(&task.room_id),
                Some(&task.agent_id),
                after,
                i64::from(page.rows_left) + 1,
            )?;
            for receipt in receipts {
                if !page.push(Cursor::Receipts(receipt.sequence), json!(receipt))? {
                    return Ok(reply(page, true));
                }
            }
        }
        Ok(reply(page, false))
    }

    fn read_agent_wake(
        &self,
        task: &AgentTaskExecution,
        id: &str,
        now: u64,
    ) -> Result<Value, DaemonError> {
        let store = &self.owned.durable_state_store;
        let wake = store
            .agent_wake(id)?
            .filter(|w| w.room_id == task.room_id && w.agent_id == task.agent_id)
            .ok_or_else(|| ledger::error("wake unavailable"))?;
        let holder = store
            .agent_tasks(Some(&task.room_id), Some(&task.agent_id))?
            .into_iter()
            .find(|t| t.task_id == wake.task_id);
        Ok(json!({
            "now_ms": now,
            "wake": wake_entry(&wake),
            "task": holder.map(|t| wait_row(&t)),
        }))
    }
}

/// One task's wait as the agent reads it: what it waits on and until when.
pub(super) fn wait_row(task: &AgentTaskExecution) -> Value {
    json!({
        "task_id": task.task_id,
        "prompt_id": task.prompt_id,
        "state": task.state,
        "reason": task.reason,
        "registration_ids": task.wait.as_ref().map(|w| &w.registration_ids),
        "deadline_ms": task.wait.as_ref().and_then(|w| w.deadline_ms),
    })
}
