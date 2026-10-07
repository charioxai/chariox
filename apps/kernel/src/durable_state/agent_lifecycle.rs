//! MP-08 / MP-09 / MP-10 / MP-11 A02: home-owned task/inbox transactions.
//! No provider I/O occurs here. Intent commits precede every dispatch.
mod delivery;
mod migration;
mod quarantine;
#[cfg(test)]
mod tests;
mod transitions;
use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::sync::mpsc;
use transitions::apply;
pub(crate) const SWEEP_MS: u64 = 30_000;
pub(crate) const DELIVERY_TIMEOUT_MS: u64 = 120_000;
pub(crate) const LONG_WAIT_MS: u64 = 900_000;
pub(crate) const NO_PROGRESS_LIMIT: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Working,
    Waiting,
    Blocked,
    Done,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTaskExecution {
    pub task_id: String,
    pub room_id: String,
    #[serde(default)]
    pub owner_user_id: String,
    pub agent_id: String,
    pub prompt_id: String,
    pub provider_run_id: Option<String>,
    pub revision: u64,
    #[serde(default)]
    pub blocked_revision: u64,
    pub state: ExecutionState,
    pub reason: String,
    pub obligations: Vec<AgentObligation>,
    pub wait: Option<AgentWait>,
    pub last_progress_at_ms: u64,
    pub progress_sequence: u64,
    pub no_progress_wakes: u32,
    pub correction_used: bool,
    pub pending_prompt_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentObligation {
    pub id: String,
    pub kind: String,
    pub resource_id: Option<String>,
    #[serde(default)]
    pub completion_task_id: Option<String>,
    pub status: String,
    pub dispatch_state: String,
}
impl AgentObligation {
    pub(crate) fn completion_source(&self) -> Option<&str> {
        self.completion_task_id
            .as_deref()
            .or(self.resource_id.as_deref())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWait {
    pub registration_ids: Vec<String>,
    pub deadline_ms: u64,
    pub started_at_ms: u64,
    pub inbox_cursor: u64,
    pub long_wait_notified: bool,
    #[serde(default)]
    pub last_checked_at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Registration {
    pub id: String,
    pub task_id: String,
    pub source_id: String,
    pub obligation_id: Option<String>,
    #[serde(default)]
    pub source_cursor: u64,
    pub live: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InboxEvent {
    pub sequence: u64,
    pub room_id: String,
    pub agent_id: String,
    pub source_id: String,
    pub occurrence_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub urgent: bool,
    pub reply_requested: bool,
    pub state: String,
    pub prompt_id: Option<String>,
    pub target_prompt_id: Option<String>,
    pub provider_run_id: Option<String>,
    pub attempted_at_ms: Option<u64>,
    pub submit_epoch: Option<u64>,
}
pub(crate) enum Operation {
    Begin {
        owner: String,
        room: String,
        agent: String,
        prompt: String,
        run: Option<String>,
        now: u64,
    },
    RegisterObligation {
        owner: String,
        room: String,
        agent: String,
        prompt: String,
        run: Option<String>,
        id: String,
        kind: String,
        resource: Option<String>,
        now: u64,
    },
    DispatchReceipt {
        id: String,
        accepted: bool,
        resource: Option<String>,
    },
    Subscribe {
        task: String,
        prompt: String,
        registration: Registration,
    },
    Unsubscribe {
        task: String,
        prompt: String,
        registration: String,
    },
    Yield {
        task: String,
        prompt: String,
        registrations: Vec<String>,
        cursor: u64,
        deadline: u64,
        reason: String,
        now: u64,
    },
    Block {
        task: String,
        prompt: String,
        reason: String,
    },
    Settle {
        room: String,
        agent: String,
        prompt: String,
        run: String,
        has_answer: bool,
        cancelled: bool,
        now: u64,
    },
    Occur(InboxEvent),
    Send {
        task: String,
        prompt: String,
        event: InboxEvent,
    },
    Attempt {
        room: String,
        agent: String,
        sequence: u64,
        prompt: String,
        target: Option<String>,
        run: Option<String>,
        now: u64,
    },
    Defer {
        room: String,
        agent: String,
        sequence: u64,
        now: u64,
    },
    BindAttempt {
        room: String,
        agent: String,
        sequence: u64,
        run: String,
        submit_epoch: u64,
    },
    Receipt {
        room: String,
        agent: String,
        sequence: u64,
        state: String,
    },
    Ack {
        room: String,
        agent: String,
        sequence: u64,
        handled: bool,
        now: u64,
    },
    SourceOutcome {
        room: String,
        source: String,
        occurrence: String,
        success: bool,
        now: u64,
    },
    Sweep {
        now: u64,
    },
    CancelTask {
        task: String,
        owner: String,
        revision: u64,
    },
    OwnerResponse {
        task: String,
        revision: u64,
        resume: bool,
        now: u64,
    },
}
#[derive(Debug)]
pub(crate) enum Outcome {
    Task(AgentTaskExecution),
    Event(InboxEvent),
    Settled {
        task: AgentTaskExecution,
        correction: bool,
    },
    Swept(Vec<AgentTaskExecution>),
    Saved,
}
pub(super) struct Request {
    operation: Operation,
    response: mpsc::Sender<Result<Outcome, DaemonError>>,
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentLifecycleRequest")
            .finish_non_exhaustive()
    }
}
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
    CREATE TABLE IF NOT EXISTS agent_registrations(id TEXT PRIMARY KEY,task_id TEXT NOT NULL,payload TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS agent_lifecycle_quarantine(kind TEXT NOT NULL,id TEXT NOT NULL,payload TEXT NOT NULL,at_ms INTEGER NOT NULL,PRIMARY KEY(kind,id));
    CREATE TABLE IF NOT EXISTS agent_source_occurrences(sequence INTEGER PRIMARY KEY AUTOINCREMENT,room_id TEXT NOT NULL,source_id TEXT NOT NULL,occurrence_id TEXT NOT NULL,success INTEGER NOT NULL,UNIQUE(room_id,source_id,occurrence_id));
    CREATE TABLE IF NOT EXISTS agent_inbox(sequence INTEGER PRIMARY KEY AUTOINCREMENT,room_id TEXT NOT NULL,agent_id TEXT NOT NULL,source_id TEXT NOT NULL,occurrence_id TEXT NOT NULL,payload TEXT NOT NULL,UNIQUE(room_id,agent_id,source_id,occurrence_id));
    CREATE INDEX IF NOT EXISTS agent_inbox_recipient ON agent_inbox(room_id,agent_id,sequence);").map_err(sql)?;
    migration::migrate(db)
}
impl DurableKernelStateStore {
    pub(crate) fn agent_lifecycle(&self, operation: Operation) -> Result<Outcome, DaemonError> {
        let (response, receive) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AgentLifecycle(Box::new(Request {
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
        let db = self.lock_connection("agent.lifecycle.front")?;
        let row:Option<String>=db.query_row("SELECT payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.state') IN ('pending','submitting','uncertain','blocked') ORDER BY sequence LIMIT 1",params![room,agent],|r|r.get(0)).optional().map_err(sql)?;
        row.map(|row| decode(&row)).transpose()
    }
    pub(crate) fn agent_event_for_prompt(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<Option<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.receipt")?;
        let row:Option<String>=db.query_row("SELECT payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.prompt_id')=?3",params![room,agent,prompt],|r|r.get(0)).optional().map_err(sql)?;
        row.map(|row| decode(&row)).transpose()
    }
    pub(crate) fn agent_inbox(
        &self,
        room: &str,
        agent: &str,
        after: u64,
    ) -> Result<Vec<InboxEvent>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.inbox")?;
        let mut q=db.prepare("SELECT payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence>?3 ORDER BY sequence LIMIT 128").map_err(sql)?;
        let rows = q
            .query_map(params![room, agent, sql_integer(after)?], |r| {
                r.get::<_, String>(0)
            })
            .map_err(sql)?;
        rows.map(|r| decode(&r.map_err(sql)?)).collect()
    }
}
pub(super) fn execute(db: &mut Connection, request: Request) -> bool {
    let mut uncertain = false;
    let result = (|| {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let value = apply(&tx, request.operation)?;
        if let Err(e) = tx.commit() {
            uncertain = !super::storage_full::observe(&e);
            return Err(error(if uncertain {
                "commit uncertain; restart required".into()
            } else {
                e.to_string()
            }));
        }
        Ok(value)
    })();
    let _ = request.response.send(result);
    uncertain
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
            .any(|o| matches!(o.status.as_str(), "open" | "failed" | "settling"))
    {
        return Err(error(
            "done has unfinished obligations; quarantine required",
        ));
    }
    Ok(t)
}
fn save(tx: &Transaction<'_>, task: &AgentTaskExecution) -> Result<(), DaemonError> {
    tx.execute("INSERT INTO agent_tasks VALUES(?1,?2,?3,?4,?5) ON CONFLICT(task_id) DO UPDATE SET prompt_id=excluded.prompt_id,payload=excluded.payload",params![task.task_id,task.room_id,task.agent_id,task.prompt_id,encode(task)?]).map_err(sql)?;
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
        .prepare("SELECT payload FROM agent_registrations WHERE task_id=?1")
        .map_err(sql)?;
    let rows = q
        .query_map([task], |r| r.get::<_, String>(0))
        .map_err(sql)?;
    rows.map(|r| decode(&r.map_err(sql)?)).collect()
}
fn current(task: &AgentTaskExecution, prompt: &str) -> Result<(), DaemonError> {
    if task.prompt_id != prompt
        || task.state != ExecutionState::Working
        || task.pending_prompt_id.is_some()
    {
        return Err(error("turn mutation admission closed or stale"));
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
    let previous:Option<String>=tx.query_row("SELECT payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND source_id=?3 AND occurrence_id=?4",params![e.room_id,e.agent_id,e.source_id,e.occurrence_id],|r|r.get(0)).optional().map_err(sql)?;
    if let Some(previous) = previous {
        let p: InboxEvent = decode(&previous)?;
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
    let count:i64=tx.query_row("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.state')!='handled'",params![e.room_id,e.agent_id],|r|r.get(0)).map_err(sql)?;
    if count >= 1024 {
        return Err(error("inbox full; unacknowledged events retained"));
    }
    tx.execute("INSERT INTO agent_inbox(room_id,agent_id,source_id,occurrence_id,payload) VALUES(?1,?2,?3,?4,'{}')",params![e.room_id,e.agent_id,e.source_id,e.occurrence_id]).map_err(sql)?;
    e.sequence = tx.last_insert_rowid() as u64;
    save_event(tx, &e)?;
    Ok(e)
}
fn save_event(tx: &Transaction<'_>, e: &InboxEvent) -> Result<(), DaemonError> {
    tx.execute(
        "UPDATE agent_inbox SET payload=?2 WHERE sequence=?1",
        params![sql_integer(e.sequence)?, encode(e)?],
    )
    .map_err(sql)?;
    Ok(())
}
fn get_event(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    seq: u64,
) -> Result<InboxEvent, DaemonError> {
    let s: String = tx
        .query_row(
            "SELECT payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence=?3",
            params![room, agent, sql_integer(seq)?],
            |r| r.get(0),
        )
        .map_err(sql)?;
    decode(&s)
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

/// MP-08/MP-10/MP-11: both existing provider reapers settle the exact inbox intent.
/// An enqueue ACK or a changed/ended run cannot substitute for this receipt.
pub(crate) fn finish_provider_event_submit(
    store: &DurableKernelStateStore,
    epoch: u64,
    finished: &crate::provider::FinishedProviderPromptSubmitJob,
) -> Result<bool, DaemonError> {
    let Some(event) = store.agent_event_for_prompt(
        &finished.session_id,
        &finished.agent_id,
        &finished.prompt_id,
    )?
    else {
        return Ok(false);
    };
    if event.provider_run_id.as_deref() != Some(&finished.provider_run_id)
        || event.submit_epoch != Some(epoch)
    {
        return Err(error(
            "provider event receipt has a stale run or submit epoch",
        ));
    }
    if matches!(event.state.as_str(), "submitting" | "uncertain" | "blocked") {
        let state = match &finished.result {
            Ok(_) => "accepted",
            Err(DaemonError::ProviderPromptSteerRejected { .. }) => "rejected",
            Err(_) => "uncertain",
        };
        store.agent_lifecycle(Operation::Receipt {
            room: finished.session_id.clone(),
            agent: finished.agent_id.clone(),
            sequence: event.sequence,
            state: state.into(),
        })?;
    }
    Ok(event.target_prompt_id.is_some())
}
