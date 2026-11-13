//! MP-08 / MP-09 / MP-10 / MP-11 A02: home-owned task/inbox transactions.
//! No provider I/O occurs here. Intent commits precede every dispatch.
#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod catalog_continuation_tests;
pub(crate) mod completion;
mod continuations;
#[cfg(test)]
mod deadline_tests;
mod deadlines;
mod delegation;
mod delivery;
mod leased_processes;
#[cfg(test)]
mod live_wait_tests;
pub(crate) use leased_processes::LeasedProcessMutation;
#[cfg(test)]
mod inbox_lookup_tests;
#[cfg(test)]
mod unread_steer_tests;
mod unread_steers;
mod worker_stops;
pub use leased_processes::{LeasedProcessBinding, LeasedProcessReceipt};
pub(crate) use worker_stops::{UnstoppedWorkerRun, WorkerStopMutation};
#[cfg(test)]
mod blocked_intent_tests;
#[cfg(test)]
mod child_end_receipt_tests;
pub(crate) mod created_peers;
mod delivery_waits;
#[cfg(test)]
mod fault_tests;
pub(crate) mod faults;
mod goal_evidence;
#[cfg(test)]
mod goal_tests;
pub(crate) mod goals;
mod migration;
pub(crate) mod outside;
#[cfg(test)]
mod outside_tests;
pub(crate) mod peer_follows;
#[cfg(test)]
mod peer_room_reply_tests;
#[cfg(test)]
mod peer_task_follow_tests;
#[cfg(test)]
mod plain_tests;
mod quarantine;
mod replies;
#[cfg(test)]
mod reply_terminal_tests;
mod settled_waits;
mod subscriptions;
mod supervision;
#[cfg(test)]
mod tests;
mod transitions;
mod types;
pub(crate) use types::{DelegateGoneReason, ReplyOutcome};
#[cfg(test)]
mod wake_tests;
mod wakeable;
mod wakes;
mod workflow_tasks;
#[cfg(test)]
mod yield_settled_tests;
use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use std::ops::ControlFlow;
use std::sync::mpsc;
use transitions::apply;
pub(super) use types::Request;
pub use types::{
    AgentObligation, AgentTaskExecution, AgentWait, AgentWake, ExecutionState, TaskOwnerAction,
};
mod wake_receipts;
pub(crate) use types::{
    unresolved_source_outcome, CancelInitiator, InboxEvent, Operation, Outcome, Registration,
    WakeEnd,
};
pub(crate) const SWEEP_MS: u64 = 30_000;
pub(crate) const DELIVERY_TIMEOUT_MS: u64 = 120_000;
pub(crate) const LONG_WAIT_MS: u64 = 900_000;
pub(crate) const RESOLVED_WAIT_REASON: &str = "All obligations handled and wait sources resolved";

fn sql_integer(value: u64) -> Result<i64, DaemonError> {
    i64::try_from(value).map_err(|_| error("integer exceeds durable storage bound"))
}
pub(crate) fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "agent.lifecycle",
        message: message.into(),
    }
}
fn sql(e: rusqlite::Error) -> DaemonError {
    super::storage_full::observe(&e);
    error(e.to_string())
}
fn encode(v: &impl Serialize) -> Result<String, DaemonError> {
    serde_json::to_string(v).map_err(|_| error("state encoding failed"))
}
fn decode<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, DaemonError> {
    serde_json::from_str(s).map_err(|_| error("state corrupt; quarantine required"))
}
pub(super) fn initialize(db: &mut Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_tasks(task_id TEXT PRIMARY KEY,room_id TEXT NOT NULL,agent_id TEXT NOT NULL,prompt_id TEXT NOT NULL,payload TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS agent_tasks_room ON agent_tasks(room_id,agent_id);
    CREATE UNIQUE INDEX IF NOT EXISTS agent_task_turn ON agent_tasks(room_id,agent_id,prompt_id);
    CREATE TABLE IF NOT EXISTS agent_progress_receipts(task_id TEXT NOT NULL,receipt_id TEXT NOT NULL,recorded_at_ms INTEGER NOT NULL,PRIMARY KEY(task_id,receipt_id));
    CREATE TABLE IF NOT EXISTS agent_delegation_messages(sequence INTEGER PRIMARY KEY,task_id TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS agent_urgent_reply_links(sequence INTEGER PRIMARY KEY,target_task_id TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS agent_registrations(id TEXT PRIMARY KEY,task_id TEXT NOT NULL,payload TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS agent_lifecycle_quarantine(kind TEXT NOT NULL,id TEXT NOT NULL,payload TEXT NOT NULL,at_ms INTEGER NOT NULL,PRIMARY KEY(kind,id));
    CREATE TABLE IF NOT EXISTS agent_source_occurrences(sequence INTEGER PRIMARY KEY AUTOINCREMENT,room_id TEXT NOT NULL,source_id TEXT NOT NULL,occurrence_id TEXT NOT NULL,success INTEGER NOT NULL,public_answer TEXT,UNIQUE(room_id,source_id,occurrence_id));
    CREATE TABLE IF NOT EXISTS agent_inbox(sequence INTEGER PRIMARY KEY AUTOINCREMENT,room_id TEXT NOT NULL,agent_id TEXT NOT NULL,source_id TEXT NOT NULL,occurrence_id TEXT NOT NULL,payload TEXT NOT NULL,UNIQUE(room_id,agent_id,source_id,occurrence_id));
    CREATE TABLE IF NOT EXISTS agent_inbox_refusals(sequence INTEGER PRIMARY KEY,first_refused_at_ms INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS agent_inbox_shown(sequence INTEGER PRIMARY KEY);
    CREATE TABLE IF NOT EXISTS agent_inbox_unread_reverts(sequence INTEGER PRIMARY KEY,count INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS agent_inbox_recipient ON agent_inbox(room_id,agent_id,sequence);
    CREATE INDEX IF NOT EXISTS agent_inbox_occurrence ON agent_inbox(room_id,agent_id,occurrence_id);
    CREATE INDEX IF NOT EXISTS agent_inbox_prompt ON agent_inbox(room_id,agent_id,CASE WHEN json_valid(payload) THEN json_extract(payload,'$.prompt_id') END);
    CREATE INDEX IF NOT EXISTS agent_inbox_target_prompt ON agent_inbox(room_id,agent_id,CASE WHEN json_valid(payload) THEN json_extract(payload,'$.target_prompt_id') END);
    CREATE TABLE IF NOT EXISTS peer_agent_requests(sender_kernel_id TEXT NOT NULL,request_id TEXT NOT NULL,answer TEXT NOT NULL,PRIMARY KEY(sender_kernel_id,request_id));").map_err(sql)?;
    let columns: Vec<String> = db
        .prepare("PRAGMA table_info(agent_source_occurrences)")
        .map_err(sql)?
        .query_map([], |r| r.get(1))
        .map_err(sql)?
        .collect::<Result<_, _>>()
        .map_err(sql)?;
    if !columns.iter().any(|c| c == "public_answer") {
        db.execute_batch("ALTER TABLE agent_source_occurrences ADD COLUMN public_answer TEXT")
            .map_err(sql)?;
    }
    index_task_lookups(db)?;
    outside::initialize(db)?;
    created_peers::initialize(db)?;
    peer_follows::initialize(db)?;
    deadlines::initialize(db)?;
    wakes::initialize(db)?;
    leased_processes::initialize(db)?;
    worker_stops::initialize(db)?;
    workflow_tasks::initialize(db)?;
    migration::migrate(db)
}
impl DurableKernelStateStore {
    pub(crate) fn agent_lifecycle(&self, operation: Operation) -> Result<Outcome, DaemonError> {
        self.enqueue_agent_lifecycle(operation)?.wait()
    }

    pub(crate) fn enqueue_agent_lifecycle(
        &self,
        operation: Operation,
    ) -> Result<PendingLifecycleOutcome, DaemonError> {
        let (response, receive) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AgentLifecycle(Box::new(Request {
                enqueued_at: std::time::Instant::now(),
                operation,
                response,
            })))?;
        Ok(PendingLifecycleOutcome(receive))
    }
    pub(crate) fn agent_tasks(
        &self,
        room: Option<&str>,
        agent: Option<&str>,
    ) -> Result<Vec<AgentTaskExecution>, DaemonError> {
        self.read_tasks(
            "agent.lifecycle.tasks",
            "(?1 IS NULL OR room_id=?1) AND (?2 IS NULL OR agent_id=?2)",
            params![room, agent],
        )
    }
    /// One task by its immutable identity, without reading its agent's history.
    pub(crate) fn agent_task(&self, task: &str) -> Result<Option<AgentTaskExecution>, DaemonError> {
        Ok(self
            .read_tasks("agent.lifecycle.task", "task_id=?1", [task])?
            .pop())
    }
    pub(crate) fn agent_unpublished_done_tasks(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<Vec<AgentTaskExecution>, DaemonError> {
        self.read_tasks(
            "agent.lifecycle.unpublished_done",
            "rowid IN (SELECT rowid FROM agent_tasks INDEXED BY agent_tasks_done_pending \
             WHERE state='done' AND terminal_outcome_recorded=0 AND room_id=?1 AND agent_id=?2)",
            params![room, agent],
        )
    }
    /// The tasks a lifecycle sweep can act on: unfinished or unreadable ones, cancelled ones whose
    /// terminal outcome is not recorded yet, and finished ones that carry obligations or still
    /// owe their terminal outcome. Recorded plain history stays out of the sweep.
    pub(crate) fn agent_tasks_to_sweep(&self) -> Result<Vec<AgentTaskExecution>, DaemonError> {
        self.read_tasks(
            "agent.lifecycle.sweep_tasks",
            "rowid IN (\
             SELECT rowid FROM agent_tasks WHERE state IN ('working','waiting','blocked') \
             UNION SELECT rowid FROM agent_tasks WHERE state IS NULL \
             UNION SELECT rowid FROM agent_tasks WHERE obligated=1 \
             UNION SELECT rowid FROM agent_tasks INDEXED BY agent_tasks_cancelled_pending \
              WHERE state='cancelled' AND terminal_outcome_recorded=0 \
             UNION SELECT rowid FROM agent_tasks INDEXED BY agent_tasks_done_pending \
              WHERE state='done' AND terminal_outcome_recorded=0)",
            [],
        )
    }
    /// The agent's task for one turn. Tasks are kept after they finish, so the reads made on
    /// every turn take it by index instead of decoding the agent's whole history.
    pub(crate) fn agent_turn_task(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<Option<AgentTaskExecution>, DaemonError> {
        Ok(self
            .read_tasks(
                "agent.lifecycle.turn_task",
                "room_id=?1 AND agent_id=?2 AND prompt_id=?3",
                params![room, agent, prompt],
            )?
            .pop())
    }
    /// The agent's first task running `prompt` or holding it as its pending continuation.
    pub(crate) fn agent_turn_or_continuation_task(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<Option<AgentTaskExecution>, DaemonError> {
        Ok(self
            .read_tasks(
                "agent.lifecycle.turn_task",
                "rowid IN (SELECT rowid FROM agent_tasks WHERE room_id=?1 AND agent_id=?2 AND prompt_id=?3 \
                 UNION SELECT rowid FROM agent_tasks WHERE pending_prompt_id=?3 AND room_id=?1 AND agent_id=?2)",
                params![room, agent, prompt],
            )?
            .into_iter()
            .next())
    }
    /// Whether the agent has an unfinished task other than `task`; an unreadable one counts.
    pub(crate) fn agent_has_other_unfinished_task(
        &self,
        room: &str,
        agent: &str,
        task: &str,
    ) -> Result<bool, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.unfinished")?;
        // The planner otherwise prefers the per-agent history index before ANALYZE.
        let mut q = db.prepare("SELECT EXISTS(SELECT 1 FROM agent_tasks INDEXED BY agent_tasks_unfinished WHERE room_id=?1 AND agent_id=?2 AND task_id<>?3 \
             AND (state IS NULL OR state NOT IN ('done','cancelled')))").map_err(sql)?;
        let exists = q
            .query_row(params![room, agent, task], |r| r.get(0))
            .map_err(sql)?;
        #[cfg(test)]
        UNFINISHED_TASK_STEPS.with(|n| n.set(q.get_status(rusqlite::StatementStatus::VmStep)));
        Ok(exists)
    }
    fn read_tasks(
        &self,
        operation: &'static str,
        filter: &str,
        values: impl rusqlite::Params,
    ) -> Result<Vec<AgentTaskExecution>, DaemonError> {
        let db = self.lock_connection(operation)?;
        let mut q = db
            .prepare_cached(&format!(
                "SELECT task_id,room_id,agent_id,prompt_id,payload FROM agent_tasks WHERE {filter} ORDER BY rowid"
            ))
            .map_err(sql)?;
        #[cfg(test)]
        q.reset_status(rusqlite::StatementStatus::VmStep);
        let rows = q
            .query_map(values, |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(sql)?;
        let tasks = rows
            .map(|row| {
                let (id, room, agent, prompt, payload) = row.map_err(sql)?;
                Ok(decode_task(&id, &room, &agent, &prompt, &payload)
                    .unwrap_or_else(|_| quarantine::task(id, room, agent, prompt)))
            })
            .collect();
        #[cfg(test)]
        TASK_QUERY_STEPS.with(|n| n.set(q.get_status(rusqlite::StatementStatus::VmStep)));
        tasks
    }
    /// The agents of `room` that still hold lifecycle work: a task that is working, waiting or
    /// blocked, or a wake that can still fire. An agent that yielded on a wake, or whose turn
    /// ended under a background hold, is idle in the roster but is not finished.
    pub(crate) fn agents_with_unfinished_work(
        &self,
        room: &str,
    ) -> Result<std::collections::HashSet<String>, DaemonError> {
        let armed = wakes::ARMED
            .iter()
            .map(|state| format!("'{state}'"))
            .collect::<Vec<_>>()
            .join(",");
        let db = self.lock_connection("agent.lifecycle.unfinished_agents")?;
        let mut q = db
            .prepare(&format!(
                "SELECT agent_id FROM agent_tasks WHERE room_id=?1 AND state IN ('working','waiting','blocked') \
                 UNION SELECT agent_id FROM agent_wakes WHERE room_id=?1 AND json_extract(payload,'$.state') IN ({armed})"
            ))
            .map_err(sql)?;
        let agents = q
            .query_map([room], |row| row.get::<_, String>(0))
            .map_err(sql)?
            .collect::<Result<_, _>>()
            .map_err(sql)?;
        Ok(agents)
    }
    /// The room's unfinished tasks and its `finished` most recent finished ones, in ledger
    /// order. Older finished tasks are not read, so a long room costs a read no more.
    pub(crate) fn visible_agent_tasks(
        &self,
        room: &str,
        finished: usize,
    ) -> Result<Vec<AgentTaskExecution>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.visible_tasks")?;
        // Unfinished tasks and unreadable ones (state NULL, so decoding quarantines them), plus the
        // most recent `finished` done or cancelled ones, each found through the (room, state) index.
        let mut q = db
            .prepare(
                "SELECT task_id,room_id,agent_id,prompt_id,payload FROM agent_tasks WHERE rowid IN (\
                 SELECT rowid FROM agent_tasks WHERE room_id=?1 AND state IN ('working','waiting','blocked') \
                 UNION ALL SELECT rowid FROM agent_tasks WHERE room_id=?1 AND state IS NULL \
                 UNION ALL SELECT rowid FROM (\
                  SELECT rowid FROM (SELECT rowid FROM agent_tasks WHERE room_id=?1 AND state='done' ORDER BY rowid DESC LIMIT ?2) \
                  UNION ALL SELECT rowid FROM (SELECT rowid FROM agent_tasks WHERE room_id=?1 AND state='cancelled' ORDER BY rowid DESC LIMIT ?2) \
                  ORDER BY rowid DESC LIMIT ?2)) ORDER BY rowid",
            )
            .map_err(sql)?;
        let rows = q
            .query_map(params![room, sql_integer(finished as u64)?], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        #[cfg(test)]
        VISIBLE_TASK_STEPS.with(|n| n.set(q.get_status(rusqlite::StatementStatus::VmStep)));
        rows.into_iter()
            .map(|(id, room, agent, prompt, payload)| {
                Ok(decode_task(&id, &room, &agent, &prompt, &payload)
                    .unwrap_or_else(|_| quarantine::task(id, room, agent, prompt)))
            })
            .collect()
    }
    /// Whether the task's terminal outcome is recorded. It is published only once the task's
    /// resources are settled, so a cancelled task that has it needs no further cancellation.
    pub(crate) fn agent_task_terminal_recorded(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<bool, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.terminal_recorded")?;
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2 AND occurrence_id=?3)",
            params![task.room_id, task.task_id, format!("task-terminal-{}", task.task_id)],
            |r| r.get(0),
        )
        .map_err(sql)
    }
    /// Passive owner notices are delivered with the next ordinary task context,
    /// without creating a provider turn or displacing pending source receipts.
    pub(crate) fn agent_task_cancellation_notices(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<Vec<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.cancellation_notices")?;
        let mut q = db.prepare("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.kind')='task_cancelled' AND json_extract(payload,'$.state') IN ('pending','acknowledged') ORDER BY sequence LIMIT 128").map_err(sql)?;
        let rows = q
            .query_map(params![room, agent], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(sql)?;
        rows.map(|row| {
            let (sequence, source, occurrence, payload) = row.map_err(sql)?;
            decode_inbox(sequence, room, agent, &source, &occurrence, &payload)
        })
        .collect()
    }
    pub(crate) fn agent_registrations(&self, task: &str) -> Result<Vec<Registration>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.subscriptions")?;
        registrations(&db, task)
    }
    /// The live follows of peers an agent made in any of its tasks.
    pub(crate) fn agent_peer_follows(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<Vec<Registration>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.subscriptions")?;
        peer_follows::held_by(&db, room, agent)
    }
    pub(crate) fn agent_delivery_front(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<Option<InboxEvent>, DaemonError> {
        self.agent_work_delivery_front(room, agent, None)
    }
    /// A04 causal fence: with `work`, pending events of other origins wait
    /// behind the elevated task's own correlated wakes instead of blocking them.
    pub(crate) fn agent_work_delivery_front(
        &self,
        room: &str,
        agent: &str,
        work: Option<&str>,
    ) -> Result<Option<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.front")?;
        let row:Option<(i64,String,String,String)>=db.query_row(&format!("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.state') IN ('submitting','uncertain','blocked') OR (json_extract(payload,'$.state')='pending' AND json_extract(payload,'$.kind') IS NOT 'task_cancelled' AND (?3 IS NULL OR {WORK_CORRELATED})) ELSE 1 END ORDER BY sequence LIMIT 1"),params![room,agent,work],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql)?;
        row.map(|(seq, source, id, payload)| decode_inbox(seq, room, agent, &source, &id, &payload))
            .transpose()
    }
    /// Urgent steering is separate from FIFO idle wake admission. Any unresolved
    /// attempt still blocks subsequent delivery, regardless of priority.
    pub(crate) fn agent_urgent_delivery_front(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<Option<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.urgent")?;
        let row:Option<(i64,String,String,String)>=db.query_row("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.state') IN ('submitting','uncertain','blocked') OR (json_extract(payload,'$.state')='pending' AND json_extract(payload,'$.kind') IS NOT 'task_cancelled' AND json_extract(payload,'$.urgent')=1 AND json_extract(payload,'$.attempted_at_ms') IS NULL) ELSE 1 END ORDER BY CASE WHEN json_valid(payload) AND json_extract(payload,'$.state')='pending' THEN 1 ELSE 0 END,sequence LIMIT 1",params![room,agent],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql)?;
        row.map(|(seq, source, id, payload)| decode_inbox(seq, room, agent, &source, &id, &payload))
            .transpose()
    }
    /// Includes accepted old messages: their recipient can disappear after delivery.
    pub(crate) fn agent_unsettled_reply_recipients(
        &self,
    ) -> Result<Vec<(String, String)>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.reply_recipients")?;
        // Outside recipients (room '') neither request replies nor take deliveries.
        let mut q = db.prepare("SELECT DISTINCT i.room_id,i.agent_id FROM agent_inbox i WHERE i.room_id>'' AND json_valid(i.payload) AND json_extract(i.payload,'$.reply_requested')=1 AND NOT EXISTS(SELECT 1 FROM agent_source_occurrences o WHERE o.room_id=i.room_id AND o.source_id='agent-event-' || i.agent_id || '-' || i.sequence) ORDER BY i.room_id,i.agent_id").map_err(sql)?;
        let rows = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }
    /// Pending inbox rows, rather than task existence, own periodic delivery retries.
    pub(crate) fn agent_pending_inbox_recipients(
        &self,
    ) -> Result<Vec<(String, String)>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.pending_recipients")?;
        let mut q = db.prepare("SELECT DISTINCT room_id,agent_id FROM agent_inbox WHERE room_id>'' AND json_valid(payload) AND json_extract(payload,'$.state')='pending' AND json_extract(payload,'$.kind') IS NOT 'task_cancelled' ORDER BY room_id,agent_id").map_err(sql)?;
        let rows = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }
    /// Steers into a leased turn whose outcome is still unconfirmed, as
    /// `(room, agent, steer id)`: a restart must ask their workers again.
    pub(crate) fn agent_unconfirmed_leased_steers(
        &self,
    ) -> Result<Vec<(String, String, String)>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.unconfirmed_leased_steers")?;
        let mut q = db.prepare("SELECT room_id,agent_id,json_extract(payload,'$.prompt_id') FROM agent_inbox WHERE json_valid(payload) AND json_extract(payload,'$.state') IN ('uncertain','blocked') AND json_extract(payload,'$.target_prompt_id') IS NOT NULL AND json_extract(payload,'$.prompt_id') IS NOT NULL AND json_extract(payload,'$.provider_run_id') LIKE 'leased:%' ORDER BY sequence").map_err(sql)?;
        let rows = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }
    /// The batch a prompt delivered, in sequence order.
    pub(crate) fn agent_events_for_prompt(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<Vec<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.receipt")?;
        let mut q=db.prepare("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.prompt_id')=?3 ELSE 0 END ORDER BY sequence").map_err(sql)?;
        let rows = q
            .query_map(params![room, agent, prompt], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(sql)?;
        rows.map(|row| {
            let (seq, source, id, payload) = row.map_err(sql)?;
            decode_inbox(seq, room, agent, &source, &id, &payload)
        })
        .collect()
    }
    /// The row that leads the batch of a prompt: its receipt is the batch's.
    pub(crate) fn agent_event_for_prompt(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<Option<InboxEvent>, DaemonError> {
        Ok(self
            .agent_events_for_prompt(room, agent, prompt)?
            .into_iter()
            .next())
    }
    /// Source outcomes handled by this turn, including urgent steering into it.
    pub(crate) fn agent_handled_source_events_for_turn(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<Vec<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.handled_turn_sources")?;
        let mut query = db.prepare("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.state')='handled' AND json_extract(payload,'$.kind') IN ('source_completed','source_lost','child_finished','peer_event') AND (json_extract(payload,'$.prompt_id')=?3 OR json_extract(payload,'$.target_prompt_id')=?3) ELSE 0 END ORDER BY sequence").map_err(sql)?;
        let rows = query
            .query_map(params![room, agent, prompt], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(sql)?;
        rows.map(|row| {
            let (sequence, source, occurrence, payload) = row.map_err(sql)?;
            decode_inbox(sequence, room, agent, &source, &occurrence, &payload)
        })
        .collect()
    }
    /// Pending rows after `lead` that one wake may carry with it, in order,
    /// up to the first reply request (answered by a turn of its own).
    pub(crate) fn agent_delivery_batch(
        &self,
        lead: &InboxEvent,
        steer: bool,
        work: Option<&str>,
    ) -> Result<Vec<InboxEvent>, DaemonError> {
        if lead.reply_requested {
            return Ok(Vec::new());
        }
        let mut batch = Vec::new();
        for event in self.agent_work_inbox(&lead.room_id, &lead.agent_id, lead.sequence, None)? {
            if event.reply_requested {
                break;
            }
            if delivery::batch_member(&event, steer, work) {
                batch.push(event);
            }
        }
        Ok(batch)
    }
    /// Where an agent starts reading for a wake led by `lead`: before the
    /// oldest row still pending for it, so a message held for its running turn
    /// is read with the urgent one that skips it (an ack through the urgent
    /// row would settle the unread one).
    pub(crate) fn agent_read_cursor(
        &self,
        lead: &InboxEvent,
        work: Option<&str>,
    ) -> Result<u64, DaemonError> {
        let mut oldest = lead.sequence;
        self.scan_agent_inbox(&lead.room_id, &lead.agent_id, 0, work, |event| {
            if event.sequence >= lead.sequence {
                return ControlFlow::Break(());
            }
            if event.state == "pending" {
                oldest = event.sequence;
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        })?;
        Ok(oldest.saturating_sub(1))
    }
    /// Visits an agent's inbox rows after `after` in order, page by page, until
    /// `visit` breaks: closed rows are kept, so one page is not the inbox.
    pub(crate) fn scan_agent_inbox(
        &self,
        room: &str,
        agent: &str,
        mut after: u64,
        work: Option<&str>,
        mut visit: impl FnMut(InboxEvent) -> ControlFlow<()>,
    ) -> Result<(), DaemonError> {
        loop {
            let page = self.agent_work_inbox(room, agent, after, work)?;
            let full = page.len() >= INBOX_PAGE;
            let Some(last) = page.last().map(|event| event.sequence) else {
                return Ok(());
            };
            for event in page {
                if visit(event).is_break() {
                    return Ok(());
                }
            }
            if !full {
                return Ok(());
            }
            after = last;
        }
    }
    #[cfg(test)]
    pub(crate) fn agent_inbox(
        &self,
        room: &str,
        agent: &str,
        after: u64,
    ) -> Result<Vec<InboxEvent>, DaemonError> {
        self.agent_work_inbox(room, agent, after, None)
    }

    /// The answer this kernel gave to a peer request of another kernel, if it already acted on it.
    pub(crate) fn peer_agent_request_answer(
        &self,
        kernel: &str,
        request_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.peer_requests")?;
        db.query_row(
            "SELECT answer FROM peer_agent_requests WHERE sender_kernel_id=?1 AND request_id=?2",
            params![kernel, request_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)
    }

    /// Keeps the first answer to a peer request, so a retry gets the same one.
    pub(crate) fn remember_peer_agent_request_answer(
        &self,
        kernel: &str,
        request_id: &str,
        answer: &str,
    ) -> Result<(), DaemonError> {
        self.agent_lifecycle(Operation::RememberPeerRequest {
            kernel: kernel.into(),
            request_id: request_id.into(),
            answer: answer.into(),
        })
        .map(|_| ())
    }

    /// Whether this occurrence of `source` already reached the agent's inbox.
    pub(crate) fn agent_inbox_has_occurrence(
        &self,
        room: &str,
        agent: &str,
        source: &str,
        occurrence: &str,
    ) -> Result<bool, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.inbox")?;
        db.query_row(
            "SELECT 1 FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND source_id=?3 AND occurrence_id=?4",
            params![room, agent, source, occurrence],
            |_| Ok(()),
        )
        .optional()
        .map(|found| found.is_some())
        .map_err(sql)
    }

    pub(crate) fn agent_work_inbox(
        &self,
        room: &str,
        agent: &str,
        after: u64,
        work: Option<&str>,
    ) -> Result<Vec<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.inbox")?;
        let correlated = WORK_CORRELATED.replace("?3", "?4");
        let mut q=db.prepare(&format!("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence>?3 AND (?4 IS NULL OR {correlated}) ORDER BY sequence LIMIT {INBOX_PAGE}")).map_err(sql)?;
        let rows = q
            .query_map(params![room, agent, sql_integer(after)?, work], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(sql)?;
        rows.map(|r| {
            let (seq, source, id, payload) = r.map_err(sql)?;
            #[cfg(test)]
            INBOX_ROWS_READ.with(|n| n.set(n.get() + 1));
            decode_inbox(seq, room, agent, &source, &id, &payload)
        })
        .collect()
    }

    /// The recipient's rows for one occurrence id, read through the occurrence index.
    pub(crate) fn agent_inbox_occurrence(
        &self,
        room: &str,
        agent: &str,
        occurrence: &str,
    ) -> Result<Vec<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.inbox")?;
        let mut q = db
            .prepare_cached("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND occurrence_id=?3 ORDER BY sequence")
            .map_err(sql)?;
        let rows = q
            .query_map(params![room, agent, occurrence], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(sql)?;
        rows.map(|r| {
            let (seq, source, id, payload) = r.map_err(sql)?;
            #[cfg(test)]
            INBOX_ROWS_READ.with(|n| n.set(n.get() + 1));
            decode_inbox(seq, room, agent, &source, &id, &payload)
        })
        .collect()
    }

    /// Open rows with `after < sequence <= upto` the agent was never shown by an inbox read, whatever
    /// cursor it passed: the read state is the ledger's, not the agent's.
    pub(crate) fn agent_unread_inbox(
        &self,
        room: &str,
        agent: &str,
        after: u64,
        upto: u64,
        work: Option<&str>,
    ) -> Result<Vec<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.inbox.unread")?;
        let correlated = WORK_CORRELATED.replace("?3", "?5");
        let mut q=db.prepare(&format!("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence>?3 AND sequence<=?4 AND NOT EXISTS(SELECT 1 FROM agent_inbox_shown s WHERE s.sequence=agent_inbox.sequence) AND json_extract(payload,'$.state') NOT IN ('acknowledged','handled','expired','failed') AND (?5 IS NULL OR {correlated}) ORDER BY sequence LIMIT {INBOX_PAGE}")).map_err(sql)?;
        let rows = q
            .query_map(
                params![room, agent, sql_integer(after)?, sql_integer(upto)?, work],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .map_err(sql)?;
        rows.map(|r| {
            let (seq, source, id, payload) = r.map_err(sql)?;
            decode_inbox(seq, room, agent, &source, &id, &payload)
        })
        .collect()
    }
}
pub(super) fn execute(
    db: &mut Connection,
    request: Request,
    faults: &faults::AgentFaults,
    followed: &outside::Followed,
) -> bool {
    // MP-08 / MP-09 / MP-10 / MP-11 A12-S02: no objective or operation payload in diagnostics.
    let trace = match &request.operation {
        Operation::ControlGoal { task, action, .. } => match action.as_str() {
            "pause" => Some((task.clone(), "pause")),
            "resume" => Some((task.clone(), "resume")),
            "cancel" => Some((task.clone(), "cancel")),
            "interval" => Some((task.clone(), "interval")),
            "edit" => Some((task.clone(), "edit")),
            _ => None,
        },
        _ => None,
    };
    let changes_targets = match &request.operation {
        Operation::Outside(mutation) => Some(mutation.changes_targets()),
        _ => None,
    };
    let started = std::time::Instant::now();
    let mut uncertain = false;
    let result = (|| {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let begun = std::time::Instant::now();
        let duplicate = faults
            .duplicates_deliveries()
            .then(|| {
                faults::delivery_kind(&request.operation)
                    .map(|kind| (kind, request.operation.clone()))
            })
            .flatten();
        let value = apply(&tx, request.operation)?;
        let applied = std::time::Instant::now();
        if faults.writer_fails() {
            drop(tx);
            return Err(error("agent lifecycle write failed (CHARIOX_AGENT_FAULT_DIR/writer-fail); nothing was committed"));
        }
        let committed = tx.commit();
        if let Some((key, action)) = trace {
            crate::runtime::command_latency::log_goal_writer_phases(
                &key,
                action,
                started.duration_since(request.enqueued_at),
                begun.duration_since(started),
                applied.duration_since(begun),
                applied.elapsed(),
            );
        }
        if let Err(e) = committed {
            uncertain = !super::storage_full::observe(&e);
            return Err(error(if uncertain {
                "commit uncertain; restart required".into()
            } else {
                e.to_string()
            }));
        }
        // A scheduler tick that fired nothing delivered nothing.
        let delivered = !matches!(&value, Outcome::Wakes(fired) if fired.is_empty());
        if let Some((kind, operation)) = duplicate.filter(|_| delivered) {
            redeliver(db, faults, kind, operation);
        }
        Ok(value)
    })();
    // Publishers re-read the followed targets and offer every run state again after a
    // target change, and after an outside mutation nobody may be waiting for that failed.
    if changes_targets.is_some_and(|changes| changes || result.is_err()) {
        followed.changed();
    }
    let _ = request.response.send(result);
    uncertain
}
/// G05 drill: the same delivery arrives again after the first committed.
fn redeliver(db: &mut Connection, faults: &faults::AgentFaults, kind: &str, operation: Operation) {
    let outcome = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql)
        .and_then(|tx| {
            let applied = apply(&tx, operation).map(|_| ());
            tx.commit().map_err(sql)?;
            applied
        });
    faults.record_duplicate(kind, outcome.as_ref().err());
}
fn load(tx: &Transaction<'_>, id: &str) -> Result<AgentTaskExecution, DaemonError> {
    let row: Option<(String, String, String, String)> = tx
        .query_row(
            "SELECT room_id,agent_id,prompt_id,payload FROM agent_tasks WHERE task_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(sql)?;
    let (room, agent, prompt, payload) = row.ok_or_else(|| error("task unavailable"))?;
    decode_task(id, &room, &agent, &prompt, &payload)
}
fn for_turn(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    prompt: &str,
) -> Result<Option<AgentTaskExecution>, DaemonError> {
    let row: Option<(String,String)> = tx.query_row("SELECT task_id,payload FROM agent_tasks WHERE room_id=?1 AND agent_id=?2 AND prompt_id=?3",params![room,agent,prompt],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
    row.map(|(id, payload)| decode_task(&id, room, agent, prompt, &payload))
        .transpose()
}
#[cfg(test)]
thread_local! {
    /// Virtual machine steps of the last visible-tasks read on this thread.
    pub(crate) static VISIBLE_TASK_STEPS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// Tasks decoded on this thread, so a test can bound what one operation reads.
    pub(crate) static DECODED_TASKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Inbox rows decoded by the recipient reads on this thread.
    pub(crate) static INBOX_ROWS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Virtual machine steps of the per-prompt inbox lookups (batch siblings, unread steers) on this thread.
    pub(crate) static INBOX_LOOKUP_STEPS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}
fn decode_task(
    id: &str,
    room: &str,
    agent: &str,
    prompt: &str,
    payload: &str,
) -> Result<AgentTaskExecution, DaemonError> {
    #[cfg(test)]
    DECODED_TASKS.with(|n| n.set(n.get() + 1));
    let t: AgentTaskExecution = decode(payload)?;
    if t.task_id != id || t.room_id != room || t.agent_id != agent || t.prompt_id != prompt {
        return Err(error("task identity corrupt; quarantine required"));
    }
    if let Some(g) = &t.goal {
        if g.schema_version != 1
            || g.owner_user_id != t.owner_user_id
            || g.room_id != room
            || g.agent_id != agent
            || g.original_prompt_id != id
            || g.id != format!("goal:{id}")
            || !(1..=60).contains(&g.check_in_minutes)
            || g.objective_revision == 0
            || g.objective.trim().is_empty()
            || g.objective.len() > goals::RESULT_LIMIT
            || g.evaluation.as_ref().is_some_and(|e| {
                e.origin != "completion_check"
                    || e.prompt_id.is_empty()
                    || e.work_prompt_id.is_empty()
                    || e.objective_revision == 0
                    || e.objective_revision > g.objective_revision
                    || e.ledger_revision == 0
                    || e.deadline_ms != e.started_at_ms.saturating_add(goals::CHECK_BOUND_MS)
            })
            || (t.state == ExecutionState::Done
                && (g.final_answer.as_ref().is_none_or(|s| s.trim().is_empty())
                    || g.evidence.is_empty()))
        {
            return Err(error("goal binding/schema corrupt; quarantine required"));
        }
    }
    if t.state == ExecutionState::Waiting
        && t.wait.as_ref().is_none_or(|w| {
            w.registration_ids.is_empty()
                || w.deadline_ms.is_some_and(|deadline| {
                    deadline <= w.started_at_ms || deadline > i64::MAX as u64
                })
        })
    {
        return Err(error("wait state corrupt; quarantine required"));
    }
    if t.state == ExecutionState::Done
        && t.obligations
            .iter()
            .any(AgentObligation::requires_resolution)
    {
        return Err(error(
            "done has unfinished obligations; quarantine required",
        ));
    }
    Ok(t)
}
fn save(tx: &Transaction<'_>, task: &AgentTaskExecution) -> Result<(), DaemonError> {
    let previous = tx
        .query_row(
            "SELECT payload FROM agent_tasks WHERE task_id=?1",
            [&task.task_id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(sql)?
        .map(|s| decode::<AgentTaskExecution>(&s))
        .transpose()?;
    tx.execute("INSERT INTO agent_tasks(task_id,room_id,agent_id,prompt_id,payload) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(task_id) DO UPDATE SET prompt_id=excluded.prompt_id,payload=excluded.payload",params![task.task_id,task.room_id,task.agent_id,task.prompt_id,encode(task)?]).map_err(sql)?;
    outside::publish_task(tx, task, previous.as_ref())?;
    if matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled) {
        // A follow of a peer belongs to the agent that made it: the turn that made it may end.
        tx.execute("UPDATE agent_registrations SET payload=json_set(payload,'$.live',json('false')) WHERE task_id=?1 AND json_valid(payload) AND COALESCE(json_extract(payload,'$.peer'),0)=0", [&task.task_id]).map_err(sql)?;
    }
    Ok(())
}
/// Tasks are kept after they finish, so the lookups that run on every turn and every sweep
/// read an index instead of decoding the whole ledger. An unreadable payload indexes as NULL.
fn index_task_lookups(db: &Connection) -> Result<(), DaemonError> {
    let columns: Vec<String> = db
        .prepare("PRAGMA table_xinfo(agent_tasks)")
        .map_err(sql)?
        .query_map([], |r| r.get(1))
        .map_err(sql)?
        .collect::<Result<_, _>>()
        .map_err(sql)?;
    for (column, value) in [
        ("state", "TEXT", "json_extract(payload,'$.state')"),
        ("pending_prompt_id", "TEXT", "json_extract(payload,'$.pending_prompt_id')"),
        (
            "obligated",
            "INTEGER",
            "json_array_length(payload,'$.obligations')>0",
        ),
    ]
    .map(|(column, kind, value)| (column, format!("{kind} GENERATED ALWAYS AS (CASE WHEN json_valid(payload) THEN {value} END) VIRTUAL")))
    {
        if !columns.iter().any(|c| c == column) {
            db.execute_batch(&format!("ALTER TABLE agent_tasks ADD COLUMN {column} {value}"))
                .map_err(sql)?;
        }
    }
    if !columns
        .iter()
        .any(|column| column == "terminal_outcome_recorded")
    {
        // A cancelled task and its terminal source commit separately. Remember that
        // completion durably, so every later sweep reads only the pending work.
        db.execute_batch(
            "SAVEPOINT task_terminal_projection;
             ALTER TABLE agent_tasks ADD COLUMN terminal_outcome_recorded INTEGER NOT NULL DEFAULT 0
                 CHECK(terminal_outcome_recorded IN (0,1));
             UPDATE agent_tasks AS t SET terminal_outcome_recorded=1 WHERE EXISTS(
                 SELECT 1 FROM agent_source_occurrences o WHERE o.room_id=t.room_id
                 AND o.source_id=t.task_id AND o.occurrence_id='task-terminal-'||t.task_id);
             RELEASE task_terminal_projection;",
        )
        .map_err(sql)?;
    }
    let registration_columns: Vec<String> = db
        .prepare("PRAGMA table_xinfo(agent_registrations)")
        .map_err(sql)?
        .query_map([], |r| r.get(1))
        .map_err(sql)?
        .collect::<Result<_, _>>()
        .map_err(sql)?;
    if !registration_columns.iter().any(|c| c == "source_id") {
        db.execute_batch(
            "ALTER TABLE agent_registrations ADD COLUMN source_id TEXT GENERATED ALWAYS AS \
             (CASE WHEN json_valid(payload) THEN json_extract(payload,'$.source_id') END) VIRTUAL",
        )
        .map_err(sql)?;
    }
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS agent_tasks_state ON agent_tasks(state);
         CREATE INDEX IF NOT EXISTS agent_tasks_cancelled_pending ON agent_tasks(task_id)
             WHERE state='cancelled' AND terminal_outcome_recorded=0;
         CREATE INDEX IF NOT EXISTS agent_tasks_done_pending ON agent_tasks(room_id,agent_id,task_id)
             WHERE state='done' AND terminal_outcome_recorded=0;
         CREATE INDEX IF NOT EXISTS agent_tasks_unfinished ON agent_tasks(room_id,agent_id)
             WHERE state IS NULL OR state NOT IN ('done','cancelled');
         CREATE INDEX IF NOT EXISTS agent_tasks_live ON agent_tasks(room_id,agent_id,state);
         CREATE INDEX IF NOT EXISTS agent_tasks_visible ON agent_tasks(room_id,state);
         CREATE INDEX IF NOT EXISTS agent_tasks_pending_prompt ON agent_tasks(pending_prompt_id);
         CREATE INDEX IF NOT EXISTS agent_tasks_obligated ON agent_tasks(room_id,obligated,state);
         CREATE INDEX IF NOT EXISTS agent_tasks_obligated_any ON agent_tasks(obligated);
         CREATE INDEX IF NOT EXISTS agent_registrations_task ON agent_registrations(task_id);
         CREATE INDEX IF NOT EXISTS agent_registrations_source ON agent_registrations(source_id);",
    )
    .map_err(sql)
}
fn tasks(tx: &Transaction<'_>) -> Result<Vec<AgentTaskExecution>, DaemonError> {
    tasks_where(tx, "1", [])
}
/// Waiting tasks, and unreadable ones so they are quarantined as before.
fn waiting_tasks(tx: &Transaction<'_>) -> Result<Vec<AgentTaskExecution>, DaemonError> {
    tasks_where(tx, "state='waiting' OR state IS NULL", [])
}
/// An agent's unfinished tasks, and unreadable ones so they are quarantined.
fn live_tasks(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
) -> Result<Vec<AgentTaskExecution>, DaemonError> {
    tasks_where(
        tx,
        "room_id=?1 AND agent_id=?2 AND (state IN ('working','waiting','blocked') OR state IS NULL)",
        params![room, agent],
    )
}
/// The room's tasks a terminal outcome of `source` can change, in ledger order: those
/// registered on it, those holding obligations, and unreadable ones so they are quarantined.
/// Plain turns stay in the ledger, so the outcome of every turn would otherwise read them all.
/// A peer follows across the rooms of its owner, so its registration is found in any room.
const SOURCE_TASKS: &str = "rowid IN (\
    SELECT t.rowid FROM agent_registrations r CROSS JOIN agent_tasks t ON t.task_id=r.task_id \
    WHERE ((r.source_id=?2 OR r.source_id IS NULL) AND t.room_id=?1) \
    OR (r.source_id=?2 AND json_extract(r.payload,'$.peer')=1) \
    UNION SELECT rowid FROM agent_tasks WHERE room_id=?1 AND obligated=1 AND state IS NOT 'done' \
    UNION SELECT rowid FROM agent_tasks WHERE state IS NULL AND +room_id=?1)";
fn source_tasks(
    tx: &Transaction<'_>,
    room: &str,
    source: &str,
) -> Result<Vec<AgentTaskExecution>, DaemonError> {
    tasks_where(tx, SOURCE_TASKS, params![room, source])
}
/// A room's tasks (one agent's when given), in ledger order.
fn room_tasks(
    tx: &Transaction<'_>,
    room: &str,
    agent: Option<&str>,
) -> Result<Vec<AgentTaskExecution>, DaemonError> {
    tasks_where(
        tx,
        "room_id=?1 AND (?2 IS NULL OR agent_id=?2)",
        params![room, agent],
    )
}
/// An agent's tasks that hold a pending continuation.
fn continuing_tasks(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
) -> Result<Vec<AgentTaskExecution>, DaemonError> {
    tasks_where(
        tx,
        "room_id=?1 AND agent_id=?2 AND pending_prompt_id IS NOT NULL",
        params![room, agent],
    )
}
/// The task that holds `prompt` as its pending continuation.
fn pending_continuation(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    prompt: &str,
) -> Result<Option<AgentTaskExecution>, DaemonError> {
    Ok(tasks_where(
        tx,
        "pending_prompt_id=?1 AND room_id=?2 AND agent_id=?3",
        params![prompt, room, agent],
    )?
    .into_iter()
    .next())
}
fn tasks_where(
    tx: &Transaction<'_>,
    filter: &str,
    values: impl rusqlite::Params,
) -> Result<Vec<AgentTaskExecution>, DaemonError> {
    let mut q = tx
        .prepare(&format!(
            "SELECT task_id,room_id,agent_id,prompt_id,payload FROM agent_tasks WHERE {filter} ORDER BY rowid"
        ))
        .map_err(sql)?;
    let rows = q
        .query_map(values, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    drop(q);
    let mut result = vec![];
    for (id, room, agent, prompt, payload) in rows {
        match decode_task(&id, &room, &agent, &prompt, &payload) {
            Ok(t) => result.push(t),
            Err(_) => {
                quarantine::retain(tx, "task", &id, &payload)?;
                let t = quarantine::task(id, room, agent, prompt);
                save(tx, &t)?;
                result.push(t);
            }
        }
    }
    Ok(result)
}

fn registrations(tx: &Connection, task: &str) -> Result<Vec<Registration>, DaemonError> {
    let mut q = tx
        .prepare("SELECT id,payload FROM agent_registrations WHERE task_id=?1")
        .map_err(sql)?;
    let rows = q
        .query_map([task], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(sql)?;
    rows.map(|row| {
        let (id, payload) = row.map_err(sql)?;
        let registration: Registration = decode(&payload)?;
        if registration.id != id
            || registration.task_id != task
            || registration.source_id.is_empty()
        {
            return Err(error(
                "source registration identity corrupt; quarantine required",
            ));
        }
        Ok(registration)
    })
    .collect()
}
fn current_prompt(task: &AgentTaskExecution, prompt: &str) -> Result<(), DaemonError> {
    if goals::is_check(task, prompt)
        && completion::evaluation(task).is_some_and(|e| e.result.is_none())
    {
        return Err(error("completion_check cannot mutate work or authority"));
    }
    if task.prompt_id != prompt {
        return Err(error("origin_prompt_id is not this running turn's. Retry with the task_id and origin_prompt_id from this turn's <chariox-task-context>; never reuse ids from an earlier turn"));
    }
    Ok(())
}
fn current(task: &AgentTaskExecution, prompt: &str) -> Result<(), DaemonError> {
    current_prompt(task, prompt)?;
    if task.state != ExecutionState::Working || task.pending_prompt_id.is_some() {
        return Err(error("this turn can no longer change its task: it is waiting, finished or already ending. End the turn without more tools"));
    }
    Ok(())
}
fn new_admitted_task(
    tx: &Transaction<'_>,
    room: String,
    agent: String,
    prompt: String,
    run: Option<String>,
    now: u64,
) -> Result<AgentTaskExecution, DaemonError> {
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_tasks WHERE task_id=?1)",
            [&prompt],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if exists {
        return Err(error("superseded turn or conflicting task identity"));
    }
    Ok(new_task(room, agent, prompt, run, now))
}
fn new_task(
    room: String,
    agent: String,
    prompt: String,
    run: Option<String>,
    now: u64,
) -> AgentTaskExecution {
    AgentTaskExecution {
        goal: None,
        task_id: prompt.clone(),
        room_id: room,
        owner_user_id: String::new(),
        agent_id: agent,
        prompt_id: prompt,
        provider_run_id: run,
        revision: 1,
        blocked_revision: 0,
        state: ExecutionState::Working,
        reason: String::new(),
        owner_action: None,
        obligations: vec![],
        wait: None,
        last_progress_at_ms: now,
        progress_sequence: 0,
        no_progress_wakes: 0,
        correction_used: false,
        pending_prompt_id: None,
    }
}
fn event(tx: &Transaction<'_>, mut e: InboxEvent) -> Result<InboxEvent, DaemonError> {
    crate::secret_redaction::redact_json_secrets(&mut e.payload);
    let recipient = outside::Recipient::SessionAgent {
        session_id: e.room_id.clone(),
        agent_id: e.agent_id.clone(),
    };
    let (room, agent) = recipient.address();
    let previous:Option<String>=tx.query_row("SELECT payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND source_id=?3 AND occurrence_id=?4",params![room,agent,e.source_id,e.occurrence_id],|r|r.get(0)).optional().map_err(sql)?;
    if let Some(previous) = previous {
        let mut p: InboxEvent = decode(&previous)?;
        if matches!(
            e.kind.as_str(),
            "source_completed" | "source_lost" | "peer_event"
        ) && p.kind == e.kind
        {
            let mut left = p.payload.clone();
            let mut right = e.payload.clone();
            if let (Some(left), Some(right)) = (left.as_object_mut(), right.as_object_mut()) {
                left.remove("task_id");
                left.remove("task_ids");
                right.remove("task_id");
                right.remove("task_ids");
            }
            if left == right {
                let mut ids = std::collections::BTreeSet::new();
                for value in [&p.payload, &e.payload] {
                    if let Some(id) = value["task_id"].as_str() {
                        ids.insert(id.to_owned());
                    }
                    if let Some(existing) = value["task_ids"].as_array() {
                        for id in existing.iter().filter_map(|v| v.as_str()) {
                            ids.insert(id.to_owned());
                        }
                    }
                }
                p.payload["task_ids"] = serde_json::json!(ids);
                save_event(tx, &p)?;
                return Ok(p);
            }
        }
        if p.payload != e.payload
            || p.kind != e.kind
            || p.urgent != e.urgent
            || p.reply_requested != e.reply_requested
        {
            return Err(error("occurrence conflict"));
        }
        return Ok(p);
    }
    if encode(&e.payload)?.len() > 32_768 {
        return Err(error("event payload limit"));
    }
    age_delivered_events(tx, &e.room_id, &e.agent_id)?;
    let count:i64=tx.query_row("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.kind')='message' AND json_extract(payload,'$.state') IN ('pending','submitting','uncertain','blocked')",params![e.room_id,e.agent_id],|r|r.get(0)).map_err(sql)?;
    // Only agent-origin messages consume this admission quota. Kernel outcomes
    // are deduped by source/occurrence and must commit even at recipient pressure.
    if e.kind == "message" && count >= 1024 {
        return Err(error("inbox full; unacknowledged events retained"));
    }
    // A child's end is told once, by its `child_finished`: a receipt of the same end arrives already handled.
    if receipt_folded(tx, &e)? {
        e.state = "handled".into();
    }
    tx.execute("INSERT INTO agent_inbox(room_id,agent_id,source_id,occurrence_id,payload) VALUES(?1,?2,?3,?4,'{}')",params![e.room_id,e.agent_id,e.source_id,e.occurrence_id]).map_err(sql)?;
    e.sequence = tx.last_insert_rowid() as u64;
    save_event(tx, &e)?;
    if let Some(task) = e.occurrence_id.strip_prefix("child-finished-") {
        tx.execute("UPDATE agent_inbox SET payload=json_set(payload,'$.state','handled') WHERE room_id=?1 AND agent_id=?2 AND occurrence_id=?3 AND json_extract(payload,'$.kind') IN ('source_completed','source_lost') AND json_extract(payload,'$.state')='pending'", params![room, agent, format!("task-terminal-{task}")]).map_err(sql)?;
    }
    Ok(e)
}
/// The receipt `e` of a task's end whose `child_finished` the recipient has already handled.
fn folded_into_handled_child_end(
    tx: &Transaction<'_>,
    e: &InboxEvent,
) -> Result<bool, DaemonError> {
    let Some(task) = e.occurrence_id.strip_prefix("task-terminal-") else {
        return Ok(false);
    };
    tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND occurrence_id=?3 AND json_extract(payload,'$.kind')='child_finished' AND json_extract(payload,'$.state')='handled')", params![e.room_id, e.agent_id, format!("child-finished-{task}")], |r| r.get(0)).map_err(sql)
}
/// The receipt `e` of a task's end, for a recipient that already has that task's `child_finished`.
fn receipt_folded(tx: &Transaction<'_>, e: &InboxEvent) -> Result<bool, DaemonError> {
    let Some(task) = e.occurrence_id.strip_prefix("task-terminal-") else {
        return Ok(false);
    };
    if !matches!(e.kind.as_str(), "source_completed" | "source_lost") {
        return Ok(false);
    }
    tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND occurrence_id=?3 AND json_extract(payload,'$.kind')='child_finished')", params![e.room_id, e.agent_id, format!("child-finished-{task}")], |r| r.get(0)).map_err(sql)
}
// Retain occurrence identity for replay dedup; age the delivered message window.
// Source outcomes and unresolved reply links keep their handling authority.
fn age_delivered_events(tx: &Transaction<'_>, room: &str, agent: &str) -> Result<(), DaemonError> {
    tx.execute("UPDATE agent_inbox SET payload=json_set(payload,'$.state','expired') WHERE sequence IN (SELECT sequence FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.kind')='message' AND json_extract(payload,'$.state') IN ('accepted','acknowledged') AND NOT EXISTS(SELECT 1 FROM agent_urgent_reply_links l WHERE l.sequence=agent_inbox.sequence) ORDER BY sequence DESC LIMIT -1 OFFSET 256)", params![room,agent]).map_err(sql)?;
    Ok(())
}
fn save_event(tx: &Transaction<'_>, e: &InboxEvent) -> Result<(), DaemonError> {
    tx.execute(
        "UPDATE agent_inbox SET payload=?2 WHERE sequence=?1",
        params![sql_integer(e.sequence)?, encode(e)?],
    )
    .map_err(sql)?;
    age_delivered_events(tx, &e.room_id, &e.agent_id)?;
    if !matches!(e.state.as_str(), "pending" | "submitting") {
        tx.execute(
            "DELETE FROM agent_inbox_refusals WHERE sequence=?1",
            [sql_integer(e.sequence)?],
        )
        .map_err(sql)?;
    }
    Ok(())
}
fn decode_inbox(
    seq: i64,
    room: &str,
    agent: &str,
    source: &str,
    id: &str,
    payload: &str,
) -> Result<InboxEvent, DaemonError> {
    let e: InboxEvent = decode(payload)?;
    if sql_integer(e.sequence)? != seq
        || e.room_id != room
        || e.agent_id != agent
        || e.source_id != source
        || e.occurrence_id != id
        || !matches!(
            e.state.as_str(),
            "pending"
                | "submitting"
                | "uncertain"
                | "blocked"
                | "accepted"
                | "acknowledged"
                | "handled"
                | "expired"
                | "failed"
        )
    {
        return Err(error("inbox identity/state corrupt; quarantine required"));
    }
    Ok(e)
}
fn get_event(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    seq: u64,
) -> Result<InboxEvent, DaemonError> {
    let (source,id,payload):(String,String,String)=tx.query_row("SELECT source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence=?3",params![room,agent,sql_integer(seq)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(sql)?;
    decode_inbox(sql_integer(seq)?, room, agent, &source, &id, &payload)
}
/// SQL predicate: the event is a kernel-correlated wake of task `?3`.
pub(super) const WORK_CORRELATED: &str = "(json_extract(payload,'$.kind')<>'message' AND (json_extract(payload,'$.payload.task_id')=?3 OR EXISTS(SELECT 1 FROM json_each(json_extract(payload,'$.payload.task_ids')) WHERE value=?3)))";

/// Rows one inbox read returns at most.
pub(crate) const INBOX_PAGE: usize = 128;

/// Events whose outcome the agent already settled are not read again.
pub(crate) fn is_open(event: &InboxEvent) -> bool {
    !matches!(
        event.state.as_str(),
        "acknowledged" | "handled" | "expired" | "failed"
    )
}

/// A decision an agent must answer or pass before its event can be acknowledged. Answering or
/// passing it closes the event, so a closed one needs nothing more.
pub(crate) fn needs_answer(event: &InboxEvent) -> bool {
    event.kind == "child_decision" && is_open(event)
}

pub(crate) fn work_correlated(event: &InboxEvent, work: &str) -> bool {
    event.kind != "message"
        && (event.payload["task_id"].as_str() == Some(work)
            || event.payload["task_ids"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(work))))
}

pub(crate) fn occurrence(
    room: &str,
    agent: &str,
    source: &str,
    id: &str,
    kind: &str,
    payload: serde_json::Value,
) -> InboxEvent {
    InboxEvent {
        sequence: 0,
        room_id: room.into(),
        agent_id: agent.into(),
        source_id: source.into(),
        occurrence_id: id.into(),
        kind: kind.into(),
        payload,
        urgent: false,
        reply_requested: false,
        state: "pending".into(),
        prompt_id: None,
        target_prompt_id: None,
        provider_run_id: None,
        attempted_at_ms: None,
        submit_epoch: None,
    }
}

/// MP-08/MP-09/MP-10/MP-11: receipt for one delivery attempt. `None` means
/// the submission failed before any provider I/O.
pub(crate) fn delivery_receipt_state(
    submitted: Option<&Result<bool, DaemonError>>,
    structured: bool,
) -> &'static str {
    match submitted {
        Some(Ok(true)) if structured => "submitting",
        // A non-structured adapter write is its only acceptance receipt.
        Some(Ok(true)) => "accepted",
        Some(Ok(false) | Err(DaemonError::ProviderPromptSteerRejected { .. })) | None => "rejected",
        Some(Err(_)) => "uncertain",
    }
}

/// MP-08/MP-10/MP-11 G02: classify against the durable provider-I/O fence.
pub(crate) fn delivery_attempt_receipt_state(
    bound_to_provider_io: bool,
    result: &Result<bool, DaemonError>,
    structured: bool,
) -> &'static str {
    if !bound_to_provider_io && result.is_err() {
        "rejected"
    } else {
        delivery_receipt_state(Some(result), structured)
    }
}

/// MP-08/MP-09/MP-10/MP-11: a Working task whose prompt is neither active
/// nor queued blocks after the delivery timeout. A matching completion intent
/// retains its own bounded admission window, independent of the work turn's age.
pub(crate) fn lacks_live_executor(
    task: &AgentTaskExecution,
    now: u64,
    active: bool,
    queued: bool,
) -> bool {
    let awaiting_completion = completion::evaluation(task).is_some_and(|check| {
        check.result.is_none()
            && check.prompt_id == task.pending_prompt_id.as_deref().unwrap_or(&task.prompt_id)
            && check.started_at_ms <= now
            && (check.paused_at_ms.is_some() || now < check.deadline_ms)
    });
    task.state == ExecutionState::Working
        && now.saturating_sub(task.last_progress_at_ms) >= DELIVERY_TIMEOUT_MS
        && !active
        && !queued
        && !awaiting_completion
}

/// A plain turn has no goal, obligation, wait or owed reply: the ledger only
/// records it, and it is never corrected, held to a wait or blocked.
pub(super) fn is_plain(
    tx: &Transaction<'_>,
    task: &AgentTaskExecution,
) -> Result<bool, DaemonError> {
    if task.goal.is_some() || !task.obligations.is_empty() || task.wait.is_some() {
        return Ok(false);
    }
    let owes_reply: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_urgent_reply_links WHERE target_task_id=?1)",
            [&task.task_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    Ok(!owes_reply)
}

/// MP-08/MP-09/MP-10/MP-11: a failed or vanished provider turn ends a Working task
/// (plain: cancelled; with ledger work: blocked for the owner); an already
/// blocked/terminal task keeps its disposition.
pub(crate) fn end_failed_turn(
    store: &DurableKernelStateStore,
    task: &AgentTaskExecution,
    reason: String,
) -> Result<Option<AgentTaskExecution>, DaemonError> {
    if task.state != ExecutionState::Working {
        return Ok(None);
    }
    let Outcome::Task(ended) = store.agent_lifecycle(Operation::EndFailedTurn {
        task: task.task_id.clone(),
        prompt: task.prompt_id.clone(),
        reason,
    })?
    else {
        unreachable!()
    };
    Ok(Some(ended))
}

/// MP-08/MP-09/MP-10/MP-11: client-visible text for an exact delivery receipt.
pub(crate) fn receipt_notice(event: &InboxEvent) -> String {
    let n = event.sequence;
    match event.state.as_str() {
        "accepted" if event.target_prompt_id.is_some() => {
            format!("Agent inbox: event {n} accepted into the running turn")
        }
        "accepted" => format!("Agent inbox: event {n} accepted as a new turn"),
        "pending" => format!("Agent inbox: event {n} not accepted now; retained for the next turn"),
        state => format!("Agent inbox: event {n} delivery {state}; reconcile before any retry"),
    }
}

/// MP-08/MP-10/MP-11: both existing provider reapers settle the exact inbox intent.
/// An enqueue ACK or a changed/ended run cannot substitute for this receipt.
pub(crate) struct EventSubmitReceipt {
    /// The event steered an existing turn; the normal prompt settlement is skipped.
    pub steered: bool,
    /// Superseded run/epoch: discard this result before normal prompt settlement.
    pub stale: bool,
    /// Client-visible receipt, present only when this job recorded it.
    pub notice: Option<String>,
    /// The event, when the provider refused it and it is pending again.
    pub refused: Option<InboxEvent>,
}

pub(crate) fn finish_provider_event_submit(
    store: &DurableKernelStateStore,
    epoch: u64,
    finished: &crate::provider::FinishedProviderPromptSubmitJob,
) -> Result<Option<EventSubmitReceipt>, DaemonError> {
    let Some(event) = store.agent_event_for_prompt(
        &finished.session_id,
        &finished.agent_id,
        &finished.prompt_id,
    )?
    else {
        return Ok(None);
    };
    if event.provider_run_id.as_deref() != Some(&finished.provider_run_id)
        || event.submit_epoch != Some(epoch)
    {
        return Ok(Some(EventSubmitReceipt {
            steered: event.target_prompt_id.is_some(),
            stale: true,
            notice: None,
            refused: None,
        }));
    }
    let steered = event.target_prompt_id.is_some();
    if !matches!(event.state.as_str(), "submitting" | "uncertain" | "blocked") {
        return Ok(Some(EventSubmitReceipt {
            steered,
            stale: false,
            notice: None,
            refused: None,
        }));
    }
    let state = match &finished.result {
        Ok(_) => "accepted",
        Err(DaemonError::ProviderPromptSteerRejected { .. }) => "rejected",
        Err(_) => "uncertain",
    };
    match store.agent_lifecycle(Operation::Receipt {
        room: finished.session_id.clone(),
        agent: finished.agent_id.clone(),
        sequence: event.sequence,
        state: state.into(),
        now: crate::session::unix_epoch_ms(),
    })? {
        Outcome::Event(settled) => Ok(Some(EventSubmitReceipt {
            steered,
            stale: false,
            notice: Some(receipt_notice(&settled)),
            refused: (settled.state == "pending").then_some(settled),
        })),
        _ => Err(error("receipt outcome mismatch")),
    }
}

pub(crate) struct PendingLifecycleOutcome(mpsc::Receiver<Result<Outcome, DaemonError>>);
impl PendingLifecycleOutcome {
    pub(crate) fn try_result(&self) -> Option<Result<(), DaemonError>> {
        match self.0.try_recv() {
            Ok(result) => Some(result.map(|_| ())),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(error("writer response lost; intent uncertain")))
            }
        }
    }

    pub(crate) fn wait(self) -> Result<Outcome, DaemonError> {
        crate::durable_state::await_writer_response(self.0)
            .map_err(|_| error("writer response lost; intent uncertain"))?
    }
}

#[cfg(test)]
thread_local! { static UNFINISHED_TASK_STEPS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
thread_local! { static TASK_QUERY_STEPS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) }; }
