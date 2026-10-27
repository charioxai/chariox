//! MP-08 / MP-09 / MP-10 / MP-11 A02: home-owned task/inbox transactions.
//! No provider I/O occurs here. Intent commits precede every dispatch.
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
pub(crate) use leased_processes::LeasedProcessMutation;
pub use leased_processes::{LeasedProcessBinding, LeasedProcessReceipt};
#[cfg(test)]
mod blocked_intent_tests;
mod delivery_waits;
#[cfg(test)]
mod fault_tests;
pub(crate) mod faults;
#[cfg(test)]
mod goal_tests;
pub(crate) mod goals;
mod migration;
pub(crate) mod outside;
#[cfg(test)]
mod outside_tests;
#[cfg(test)]
mod plain_tests;
mod quarantine;
mod replies;
#[cfg(test)]
mod reply_terminal_tests;
mod subscriptions;
mod supervision;
#[cfg(test)]
mod tests;
mod transitions;
mod types;
pub(crate) use types::{DelegateGoneReason, ReplyOutcome};
#[cfg(test)]
mod wake_tests;
mod wakes;
mod workflow_tasks;
use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use std::sync::mpsc;
use transitions::apply;
pub(super) use types::Request;
pub use types::{AgentObligation, AgentTaskExecution, AgentWait, AgentWake, ExecutionState};
mod wake_receipts;
pub(crate) use types::{
    unresolved_source_outcome, InboxEvent, Operation, Outcome, Registration, WakeEnd,
};
pub(crate) const SWEEP_MS: u64 = 30_000;
pub(crate) const DELIVERY_TIMEOUT_MS: u64 = 120_000;
pub(crate) const LONG_WAIT_MS: u64 = 900_000;
pub(crate) const NO_PROGRESS_LIMIT: u32 = 3;

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
    CREATE INDEX IF NOT EXISTS agent_inbox_recipient ON agent_inbox(room_id,agent_id,sequence);").map_err(sql)?;
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
    outside::initialize(db)?;
    deadlines::initialize(db)?;
    wakes::initialize(db)?;
    leased_processes::initialize(db)?;
    workflow_tasks::initialize(db)?;
    migration::migrate(db)
}
impl DurableKernelStateStore {
    pub(crate) fn agent_lifecycle(&self, operation: Operation) -> Result<Outcome, DaemonError> {
        let (response, receive) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AgentLifecycle(Box::new(Request {
                enqueued_at: std::time::Instant::now(),
                operation,
                response,
            })))?;
        receive
            .recv()
            .map_err(|_| error("writer response lost; intent uncertain"))?
    }
    pub(crate) fn agent_tasks(
        &self,
        room: Option<&str>,
        agent: Option<&str>,
    ) -> Result<Vec<AgentTaskExecution>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.tasks")?;
        let mut q=db.prepare("SELECT task_id,room_id,agent_id,prompt_id,payload FROM agent_tasks WHERE (?1 IS NULL OR room_id=?1) AND (?2 IS NULL OR agent_id=?2) ORDER BY rowid").map_err(sql)?;
        let rows = q
            .query_map(params![room, agent], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(sql)?;
        rows.map(|row| {
            let (id, room, agent, prompt, payload) = row.map_err(sql)?;
            Ok(decode_task(&id, &room, &agent, &prompt, &payload)
                .unwrap_or_else(|_| quarantine::task(id, room, agent, prompt)))
        })
        .collect()
    }
    pub(crate) fn agent_registrations(&self, task: &str) -> Result<Vec<Registration>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.subscriptions")?;
        registrations(&db, task)
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
        let row:Option<(i64,String,String,String)>=db.query_row(&format!("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.state') IN ('submitting','uncertain','blocked') OR (json_extract(payload,'$.state')='pending' AND (?3 IS NULL OR {WORK_CORRELATED})) ELSE 1 END ORDER BY sequence LIMIT 1"),params![room,agent,work],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql)?;
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
        let row:Option<(i64,String,String,String)>=db.query_row("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.state') IN ('submitting','uncertain','blocked') OR (json_extract(payload,'$.state')='pending' AND json_extract(payload,'$.urgent')=1 AND json_extract(payload,'$.attempted_at_ms') IS NULL) ELSE 1 END ORDER BY CASE WHEN json_valid(payload) AND json_extract(payload,'$.state')='pending' THEN 1 ELSE 0 END,sequence LIMIT 1",params![room,agent],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql)?;
        row.map(|(seq, source, id, payload)| decode_inbox(seq, room, agent, &source, &id, &payload))
            .transpose()
    }
    /// Includes accepted old messages: their recipient can disappear after delivery.
    pub(crate) fn agent_unsettled_reply_recipients(
        &self,
    ) -> Result<Vec<(String, String)>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.reply_recipients")?;
        let mut q = db.prepare("SELECT DISTINCT i.room_id,i.agent_id FROM agent_inbox i WHERE json_valid(i.payload) AND json_extract(i.payload,'$.reply_requested')=1 AND NOT EXISTS(SELECT 1 FROM agent_source_occurrences o WHERE o.room_id=i.room_id AND o.source_id='agent-event-' || i.agent_id || '-' || i.sequence) ORDER BY i.room_id,i.agent_id").map_err(sql)?;
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
        let mut q = db.prepare("SELECT DISTINCT room_id,agent_id FROM agent_inbox WHERE json_valid(payload) AND json_extract(payload,'$.state')='pending' ORDER BY room_id,agent_id").map_err(sql)?;
        let rows = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }
    pub(crate) fn agent_event_for_prompt(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<Option<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.receipt")?;
        let row:Option<(i64,String,String,String)>=db.query_row("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.prompt_id')=?3 ELSE 0 END",params![room,agent,prompt],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql)?;
        row.map(|(seq, source, id, payload)| decode_inbox(seq, room, agent, &source, &id, &payload))
            .transpose()
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

    pub(crate) fn agent_work_inbox(
        &self,
        room: &str,
        agent: &str,
        after: u64,
        work: Option<&str>,
    ) -> Result<Vec<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.inbox")?;
        let correlated = WORK_CORRELATED.replace("?3", "?4");
        let mut q=db.prepare(&format!("SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence>?3 AND (?4 IS NULL OR {correlated}) ORDER BY sequence LIMIT 128")).map_err(sql)?;
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
            decode_inbox(seq, room, agent, &source, &id, &payload)
        })
        .collect()
    }
}
pub(super) fn execute(db: &mut Connection, request: Request, faults: &faults::AgentFaults) -> bool {
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
fn decode_task(
    id: &str,
    room: &str,
    agent: &str,
    prompt: &str,
    payload: &str,
) -> Result<AgentTaskExecution, DaemonError> {
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
                || w.deadline_ms <= w.started_at_ms
                || w.deadline_ms > i64::MAX as u64
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
    tx.execute("INSERT INTO agent_tasks VALUES(?1,?2,?3,?4,?5) ON CONFLICT(task_id) DO UPDATE SET prompt_id=excluded.prompt_id,payload=excluded.payload",params![task.task_id,task.room_id,task.agent_id,task.prompt_id,encode(task)?]).map_err(sql)?;
    outside::publish_task(tx, task, previous.as_ref())?;
    if matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled) {
        tx.execute("UPDATE agent_registrations SET payload=json_set(payload,'$.live',json('false')) WHERE task_id=?1 AND json_valid(payload)", [&task.task_id]).map_err(sql)?;
    }
    Ok(())
}
fn tasks(tx: &Transaction<'_>) -> Result<Vec<AgentTaskExecution>, DaemonError> {
    let mut q = tx
        .prepare(
            "SELECT task_id,room_id,agent_id,prompt_id,payload FROM agent_tasks ORDER BY rowid",
        )
        .map_err(sql)?;
    let rows = q
        .query_map([], |r| {
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
fn current(task: &AgentTaskExecution, prompt: &str) -> Result<(), DaemonError> {
    if goals::is_check(task, prompt)
        && completion::evaluation(task).is_some_and(|e| e.result.is_none())
    {
        return Err(error("completion_check cannot mutate work or authority"));
    }
    if task.prompt_id != prompt {
        return Err(error("origin_prompt_id is not this running turn's. Retry with the task_id and origin_prompt_id from this turn's <chariox-task-context>; never reuse ids from an earlier turn"));
    }
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
        if matches!(e.kind.as_str(), "source_completed" | "source_lost") && p.kind == e.kind {
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
    tx.execute("INSERT INTO agent_inbox(room_id,agent_id,source_id,occurrence_id,payload) VALUES(?1,?2,?3,?4,'{}')",params![e.room_id,e.agent_id,e.source_id,e.occurrence_id]).map_err(sql)?;
    e.sequence = tx.last_insert_rowid() as u64;
    save_event(tx, &e)?;
    Ok(e)
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
        }));
    }
    let steered = event.target_prompt_id.is_some();
    if !matches!(event.state.as_str(), "submitting" | "uncertain" | "blocked") {
        return Ok(Some(EventSubmitReceipt {
            steered,
            stale: false,
            notice: None,
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
        })),
        _ => Err(error("receipt outcome mismatch")),
    }
}
