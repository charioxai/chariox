//! Entry ownership on the kernel's sole durable writer. Workers propose a result;
//! only this journal can advance state or authorize an ordinary queue handoff.
pub(crate) mod admission;
mod decisions;
mod retention;
pub(crate) mod schedule;
mod state;
pub use state::Snapshot as StateSnapshot;
#[cfg(test)]
mod tests;
use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::{
    error::DaemonError,
    session::{RuntimeSession, WorkflowQueuedPrompt},
};
use chariox_app_runtime::workflow_gate as gate;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::sync::mpsc;

pub(crate) fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "workflow.gate",
        message: message.into(),
    }
}
fn sql(e: rusqlite::Error) -> DaemonError {
    super::storage_full::observe(&e);
    error(e.to_string())
}
fn encode(v: &impl Serialize) -> Result<String> {
    serde_json::to_string(v).map_err(|_| error("gate encoding failed"))
}
fn decode<T: serde::de::DeserializeOwned>(v: &str) -> Result<T> {
    serde_json::from_str(v).map_err(|_| error("gate journal corrupt"))
}
type Result<T> = std::result::Result<T, DaemonError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Ready,
    Evaluating,
    Deferred,
    Batched,
    WaitingForQueue,
    Queued,
    Dropped,
    Failed,
    Expired,
    Cancelled,
}
impl Disposition {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Dropped | Self::Failed | Self::Expired | Self::Cancelled
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub entry_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub workflow_revision: u64,
    pub module_sha256: String,
    pub config_sha256: String,
    pub binding_id: String,
    pub disposition: Disposition,
    pub accepted_at_ms: u64,
    pub deadline_ms: u64,
    pub next_due_at_ms: Option<u64>,
    pub evaluation_count: u32,
    pub queued_prompt_id: Option<String>,
    pub failure: Option<String>,
    pub decisions: Vec<DecisionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_compacted_at_ms: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub evaluation_number: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<gate::Evaluation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<gate::Report>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compacted: Option<retention::DecisionSummary>,
    pub state_revision_before: u64,
    pub state_revision_after: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Admission {
    pub kernel_id: String,
    pub session_id: String,
    pub owner: String,
    pub workflow_id: String,
    pub workflow_revision: u64,
    pub definition: gate::Definition,
    pub binding_id: String,
    pub dedupe_id: String,
    pub deadline_ms: u64,
    pub trigger: gate::Trigger,
    pub allowed_nodes: Vec<String>,
    pub template: WorkflowQueuedPrompt,
    pub source_receipt: Option<String>,
    pub notification: Option<crate::local::WorkflowNotificationSubscription>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stored {
    pub entry: Entry,
    pub admission: Admission,
    pub claim: Option<String>,
    pub claimed_evaluation: Option<gate::Evaluation>,
    pub claimed_revision: u64,
    pub batch: Option<gate::Batch>,
    pub routes: Vec<gate::Route>,
    pub source_authority: Option<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct Claim {
    pub stored: Stored,
    pub token: String,
    pub evaluation: gate::Evaluation,
    pub component: Vec<u8>,
}
pub(super) struct Request {
    operation: Operation,
    response: mpsc::Sender<Result<Outcome>>,
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GateRequest(..)")
    }
}
pub(crate) enum Operation {
    Put {
        owner: String,
        digest: String,
        component: Vec<u8>,
    },
    ResetState {
        owner: String,
        session: String,
        workflow: String,
        actor: String,
        reason: String,
    },
    Accept(Box<Admission>),
    AcceptScheduled {
        admission: Box<Admission>,
        session: Box<RuntimeSession>,
        schedule_id: String,
    },
    Claim {
        now: u64,
    },
    Complete {
        id: String,
        token: String,
        report: gate::Report,
        now: u64,
    },
    Release {
        id: String,
        before: Box<RuntimeSession>,
        after: Box<RuntimeSession>,
    },
    Cancel {
        owner: String,
        id: String,
        reason: String,
    },
}
pub(crate) enum Outcome {
    Unit,
    Entry(Entry),
    Claim(Option<Box<Claim>>),
    Scheduled(Box<RuntimeSession>),
}

pub(super) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS workflow_gate_modules(owner_id TEXT NOT NULL,digest TEXT NOT NULL,component BLOB NOT NULL,created_at_ms INTEGER NOT NULL,PRIMARY KEY(owner_id,digest));
    CREATE TABLE IF NOT EXISTS workflow_gate_lanes(owner_id TEXT NOT NULL,session_id TEXT NOT NULL,workflow_id TEXT NOT NULL,last_sequence INTEGER NOT NULL,PRIMARY KEY(owner_id,session_id,workflow_id));
    CREATE TABLE IF NOT EXISTS workflow_gate_state(owner_id TEXT NOT NULL,session_id TEXT NOT NULL,workflow_id TEXT NOT NULL,binding_id TEXT NOT NULL,revision INTEGER NOT NULL,state_json TEXT NOT NULL,PRIMARY KEY(owner_id,session_id,workflow_id,binding_id));
    CREATE TABLE IF NOT EXISTS workflow_gate_entries(sequence INTEGER PRIMARY KEY AUTOINCREMENT,entry_id TEXT NOT NULL UNIQUE,owner_id TEXT NOT NULL,session_id TEXT NOT NULL,workflow_id TEXT NOT NULL,binding_id TEXT NOT NULL,dedupe_id TEXT NOT NULL,content_digest TEXT NOT NULL,disposition TEXT NOT NULL,due_at_ms INTEGER NOT NULL,deadline_ms INTEGER NOT NULL,body_json TEXT NOT NULL,terminal_at_ms INTEGER,summary_json TEXT,UNIQUE(owner_id,session_id,workflow_id,binding_id,dedupe_id));
    CREATE INDEX IF NOT EXISTS workflow_gate_due ON workflow_gate_entries(disposition,due_at_ms,sequence);
    CREATE INDEX IF NOT EXISTS workflow_gate_deadlines ON workflow_gate_entries(deadline_ms,sequence) WHERE disposition NOT IN ('queued','dropped','failed','expired','cancelled');
    CREATE TABLE IF NOT EXISTS workflow_gate_decisions(entry_id TEXT NOT NULL,evaluation_number INTEGER NOT NULL,body_json TEXT NOT NULL,PRIMARY KEY(entry_id,evaluation_number));
    CREATE TABLE IF NOT EXISTS workflow_gate_batches(group_id TEXT PRIMARY KEY,scope_sha256 TEXT NOT NULL,entry_id TEXT NOT NULL,flush_at_ms INTEGER NOT NULL,closed INTEGER NOT NULL DEFAULT 0);
    CREATE UNIQUE INDEX IF NOT EXISTS workflow_gate_open_batch_scope ON workflow_gate_batches(scope_sha256) WHERE closed=0;
    CREATE TABLE IF NOT EXISTS workflow_gate_members(group_id TEXT NOT NULL,entry_id TEXT NOT NULL UNIQUE,payload_json TEXT NOT NULL,PRIMARY KEY(group_id,entry_id));").map_err(sql)?;
    // A claimed unfinished evaluation is never automatically retried. Its captured
    // input/state survives so the failure is replayable after a process crash.
    let interrupted = ids(
        db,
        "SELECT entry_id FROM workflow_gate_entries WHERE disposition='evaluating'",
        [],
    )?;
    for id in interrupted {
        let tx = db.unchecked_transaction().map_err(sql)?;
        let db = &*tx;
        let mut stored = load(db, &id)?.ok_or_else(|| error("gate recovery entry missing"))?;
        if let Some(evaluation) = stored.claimed_evaluation.take() {
            let record = DecisionRecord {
                evaluation_number: stored.entry.evaluation_count,
                evaluation: Some(evaluation),
                report: Some(gate::Report {
                    outcome: Err(gate::Failure::GateInterrupted),
                    fuel_consumed: 0,
                    execution_ms: 0,
                    profile: gate::PROFILE.into(),
                }),
                compacted: None,
                state_revision_before: stored.claimed_revision,
                state_revision_after: stored.claimed_revision,
            };
            db.execute(
                "INSERT OR IGNORE INTO workflow_gate_decisions VALUES(?1,?2,?3)",
                params![id, record.evaluation_number, encode(&record)?],
            )
            .map_err(sql)?;
        }
        terminal(
            db,
            &mut stored,
            Disposition::Failed,
            Some("gate_interrupted".into()),
        )?;
        tx.commit().map_err(sql)?;
    }
    Ok(())
}
fn ids(db: &Connection, query: &str, p: impl rusqlite::Params) -> Result<Vec<String>> {
    let mut q = db.prepare(query).map_err(sql)?;
    let rows = q.query_map(p, |r| r.get(0)).map_err(sql)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql)
}
fn load(db: &Connection, id: &str) -> Result<Option<Stored>> {
    db.query_row(
        "SELECT body_json FROM workflow_gate_entries WHERE entry_id=?1",
        [id],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(sql)?
    .map(|v| decode(&v))
    .transpose()
}
fn entry(db: &Connection, id: &str) -> Result<Option<Entry>> {
    db.query_row("SELECT coalesce(summary_json,json_extract(nullif(body_json,''),'$.entry')) FROM workflow_gate_entries WHERE entry_id=?1", [id], |r|r.get::<_,String>(0)).optional().map_err(sql)?.map(|json|decode(&json)).transpose()
}
fn save(db: &Connection, s: &Stored) -> Result<()> {
    let disposition =
        serde_json::to_value(&s.entry.disposition).map_err(|_| error("gate disposition"))?;
    db.execute("UPDATE workflow_gate_entries SET disposition=?2,due_at_ms=?3,deadline_ms=?4,body_json=?5,terminal_at_ms=CASE WHEN ?2 IN ('queued','dropped','failed','expired','cancelled') THEN coalesce(terminal_at_ms,?6) ELSE terminal_at_ms END WHERE entry_id=?1",params![s.entry.entry_id,disposition.as_str(),s.entry.next_due_at_ms.unwrap_or(s.entry.accepted_at_ms) as i64,s.entry.deadline_ms as i64,encode(s)?,crate::session::unix_epoch_ms() as i64]).map_err(sql)?;
    Ok(())
}
fn terminal(
    db: &Connection,
    s: &mut Stored,
    disposition: Disposition,
    failure: Option<String>,
) -> Result<()> {
    s.entry.disposition = disposition;
    s.entry.failure = failure;
    s.entry.next_due_at_ms = None;
    s.claim = None;
    save(db, s)?;
    if let Some(receipt) = &s.admission.source_receipt {
        let state = match s.entry.disposition {
            Disposition::Queued => "queued",
            Disposition::Dropped => "delivered",
            Disposition::Expired => "expired",
            _ => "failed",
        };
        db.execute("UPDATE app_outbox SET state=?2,queued_prompt_id=?3,queued_session_id=?4,revision=revision+1 WHERE receipt_id=?1 AND state='entry_owned'",params![receipt,state,s.entry.queued_prompt_id,s.entry.session_id]).map_err(sql)?;
    }
    db.execute(
        "UPDATE workflow_gate_batches SET closed=1 WHERE entry_id=?1",
        [&s.entry.entry_id],
    )
    .map_err(sql)?;
    let members=ids(db,"SELECT entry_id FROM workflow_gate_members WHERE group_id IN (SELECT group_id FROM workflow_gate_batches WHERE entry_id=?1)",[&s.entry.entry_id])?;
    for id in members {
        let mut member = load(db, &id)?.ok_or_else(|| error("gate batch member missing"))?;
        member.entry.disposition = s.entry.disposition.clone();
        member.entry.failure = s.entry.failure.clone();
        member.entry.queued_prompt_id = s.entry.queued_prompt_id.clone();
        member.entry.next_due_at_ms = None;
        terminal(
            db,
            &mut member,
            s.entry.disposition.clone(),
            s.entry.failure.clone(),
        )?;
    }
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn gate_module(&self, owner: &str, digest: &str) -> Result<Vec<u8>> {
        let db = self.lock_connection("workflow.gate.module")?;
        db.query_row(
            "SELECT component FROM workflow_gate_modules WHERE owner_id=?1 AND digest=?2",
            params![owner, digest],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .ok_or_else(|| error("gate module not found"))
    }
    pub(crate) fn gate(&self, operation: Operation) -> Result<Outcome> {
        let (response, rx) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::WorkflowGate(Box::new(Request {
                operation,
                response,
            })))?;
        super::await_writer_response(rx)
            .map_err(|_| error("gate commit unknown; restart required"))?
    }
    pub(crate) fn gate_entries(
        &self,
        owner: &str,
        session: &str,
        workflow: &str,
        after: Option<&str>,
        limit: u16,
    ) -> Result<(Vec<Entry>, Option<String>)> {
        if !(1..=100).contains(&limit) {
            return Err(error("gate entry page limit must be 1..100"));
        }
        let after = after
            .map(|s| {
                s.parse::<i64>()
                    .map_err(|_| error("invalid gate entry cursor"))
            })
            .transpose()?
            .unwrap_or(0);
        if after < 0 {
            return Err(error("invalid gate entry cursor"));
        }
        let db = self.lock_connection("workflow.gate.entries")?;
        let mut q=db.prepare("SELECT coalesce(summary_json,json_extract(nullif(body_json,''),'$.entry')),sequence FROM workflow_gate_entries WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND sequence>?4 ORDER BY sequence LIMIT ?5").map_err(sql)?;
        let rows = q
            .query_map(
                params![owner, session, workflow, after, i64::from(limit) + 1],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .map_err(sql)?;
        let mut entries = Vec::new();
        let mut cursor = None;
        let mut last = after;
        for row in rows {
            let (json, sequence) = row.map_err(sql)?;
            if entries.len() == usize::from(limit) {
                cursor = Some(last.to_string());
                break;
            }
            entries.push(decode(&json)?);
            last = sequence;
        }
        Ok((entries, cursor))
    }
    pub(crate) fn gate_entry(&self, owner: &str, id: &str) -> Result<Entry> {
        let db = self.lock_connection("workflow.gate.entry")?;
        let owned:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM workflow_gate_entries WHERE entry_id=?1 AND owner_id=?2)",params![id,owner],|r|r.get(0)).map_err(sql)?;
        let mut entry = entry(&db, id)?
            .filter(|_| owned)
            .ok_or_else(|| error("gate entry not found"))?;
        let mut q=db.prepare("SELECT body_json FROM workflow_gate_decisions WHERE entry_id=?1 ORDER BY evaluation_number LIMIT 32").map_err(sql)?;
        let rows = q.query_map([id], |r| r.get::<_, String>(0)).map_err(sql)?;
        for row in rows {
            entry.decisions.push(decode(&row.map_err(sql)?)?);
        }
        Ok(entry)
    }
    pub(crate) fn gate_waiting(&self) -> Result<Vec<Stored>> {
        let db = self.lock_connection("workflow.gate.waiting")?;
        ids(&db,"SELECT e.entry_id FROM workflow_gate_entries e WHERE e.disposition='waiting_for_queue' AND EXISTS(SELECT 1 FROM durable_workflow_hot_entities q WHERE q.owner_id=json_extract(nullif(e.body_json,''),'$.admission.kernel_id') AND q.session_id=e.session_id AND q.entity_kind='queue' AND q.entity_id=json_extract(nullif(e.body_json,''),'$.admission.template.queue_id') AND json_extract(q.payload_json,'$.enabled')=1) AND (SELECT count(*) FROM durable_workflow_hot_entities p WHERE p.owner_id=json_extract(nullif(e.body_json,''),'$.admission.kernel_id') AND p.session_id=e.session_id AND p.entity_kind='queued_prompt' AND json_extract(p.payload_json,'$.status')='queued')<1024 ORDER BY e.sequence LIMIT 8",[])?
            .into_iter().map(|id|load(&db,&id)?.ok_or_else(||error("gate entry missing"))).collect()
    }
    pub(crate) fn gate_next_due(&self) -> Result<Option<u64>> {
        let db = self.lock_connection("workflow.gate.due")?;
        db.query_row("SELECT min(due) FROM (SELECT due_at_ms AS due FROM workflow_gate_entries WHERE disposition IN ('ready','deferred') UNION ALL SELECT deadline_ms AS due FROM workflow_gate_entries WHERE disposition NOT IN ('queued','dropped','failed','expired','cancelled'))",[],|r|r.get::<_,Option<i64>>(0)).map(|v|v.map(|v|v as u64)).map_err(sql)
    }
}
pub(super) fn execute(db: &mut Connection, request: Request) -> bool {
    let mut uncertain = false;
    let result = (|| {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let result = match request.operation {
            Operation::Put {
                owner,
                digest,
                component,
            } => {
                retention::sweep(&tx, crate::session::unix_epoch_ms())?;
                if component.len() > 2 * 1024 * 1024 || gate::sha256(&component) != digest {
                    return Err(error("gate module digest/limit invalid"));
                }
                let used:i64=tx.query_row("SELECT coalesce(sum(length(component)),0) FROM workflow_gate_modules WHERE NOT (owner_id=?1 AND digest=?2)",params![owner,digest],|r|r.get(0)).map_err(sql)?;
                if used + component.len() as i64 > 256 * 1024 * 1024 {
                    return Err(error("gate module storage limit reached"));
                }
                tx.execute(
                    "INSERT OR IGNORE INTO workflow_gate_modules VALUES(?1,?2,?3,?4)",
                    params![
                        owner,
                        digest,
                        component,
                        crate::session::unix_epoch_ms() as i64
                    ],
                )
                .map_err(sql)?;
                Outcome::Unit
            }
            Operation::ResetState {
                owner,
                session,
                workflow,
                actor,
                reason,
            } => {
                state::reset(&tx, &owner, &session, &workflow, &reason, &actor)?;
                Outcome::Unit
            }
            Operation::Accept(a) => Outcome::Entry(admission::accept_in(&tx, *a)?),
            Operation::AcceptScheduled {
                admission,
                mut session,
                schedule_id,
            } => {
                let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM workflow_gate_entries WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND binding_id=?4 AND dedupe_id=?5)",params![admission.owner,admission.session_id,admission.workflow_id,admission.binding_id,admission.dedupe_id],|r|r.get(0)).map_err(sql)?;
                let entry = admission::accept_in(&tx, *admission)?;
                let schedule = session
                    .workflow_watchdog_mut(&schedule_id)
                    .ok_or_else(|| error("gate schedule missing"))?;
                if !exists {
                    schedule.set_wakeups_executed(schedule.wakeups_executed().saturating_add(1));
                }
                schedule.set_pending_run(false);
                schedule.set_last_status(Some(format!("gate_entry:{}", entry.entry_id)));
                schedule.set_last_error(None);
                if schedule
                    .max_wakeups()
                    .is_some_and(|limit| schedule.wakeups_executed() >= limit)
                {
                    schedule.set_enabled(false);
                }
                let encoded = super::workflow_runtime::encode_workflow_session(&session)?;
                let now = crate::session::unix_epoch_ms();
                super::workflow_runtime::write_workflow_runtime_transition(&tx,super::workflow_runtime::WorkflowRuntimeTransitionWrite {
                    event_id:&format!("state_evt_{now}_{}",super::rand_suffix()),event_kind:"workflow.runtime.updated",timestamp_ms:now,
                    payload_json:&serde_json::json!({"reason":"workflow_gate_schedule_accepted","entry_id":entry.entry_id}).to_string(),
                    owner_id:session.host_daemon_id(),source_owner_id:session.owner_user_id(),session_id:session.id(),hot_entities:&encoded.hot_entities,
                    workflow_runs:&encoded.workflow_runs,delivery_receipts:&encoded.delivery_receipts,prompt_state_jsons:&[],invocation_creator:None,
                }).map_err(sql)?;
                Outcome::Scheduled(session)
            }
            Operation::Claim { now } => Outcome::Claim(decisions::claim(&tx, now)?.map(Box::new)),
            Operation::Complete {
                id,
                token,
                report,
                now,
            } => Outcome::Entry(decisions::complete(&tx, &id, &token, report, now)?),
            Operation::Release { id, before, after } => {
                decisions::release(&tx, &id, &before, &after)?;
                Outcome::Unit
            }
            Operation::Cancel { owner, id, reason } => {
                if entry(&tx, &id)?.is_some_and(|e| e.disposition.is_terminal()) {
                    return Err(error("gate entry already handed or terminal"));
                }
                let mut s = load(&tx, &id)?
                    .filter(|s| s.admission.owner == owner)
                    .ok_or_else(|| error("gate entry not found"))?;
                if s.entry.disposition.is_terminal() {
                    return Err(error("gate entry already handed or terminal"));
                }
                terminal(&tx, &mut s, Disposition::Cancelled, Some(reason))?;
                Outcome::Entry(s.entry)
            }
        };
        if let Err(e) = tx.commit() {
            uncertain = !super::storage_full::observe(&e);
            return Err(error(if uncertain {
                "gate commit unknown; restart required".into()
            } else {
                e.to_string()
            }));
        }
        Ok(result)
    })();
    let _ = request.response.send(result);
    uncertain
}

pub(super) fn require_queue_pass_in(
    tx: &Transaction<'_>,
    queued: &WorkflowQueuedPrompt,
) -> Result<()> {
    let Some(id) = queued.gate_entry_id() else {
        return Ok(());
    };
    let s = load(tx, id)?.ok_or_else(|| error("gate passed entry missing"))?;
    let expected = s.admission.template.with_gate_pass(id.into(), s.routes);
    if s.entry.disposition != Disposition::Queued
        || s.entry.queued_prompt_id.as_deref() != Some(queued.id())
        || expected.workflow_id() != queued.workflow_id()
        || expected.endpoint_id() != queued.endpoint_id()
        || expected.prompt() != queued.prompt()
        || expected.publication_invocation() != queued.publication_invocation()
        || expected.gate_routes() != queued.gate_routes()
    {
        return Err(error(
            "gate queue proof does not match committed routing intent",
        ));
    }
    Ok(())
}

/// Revision activation and cancellation commit together with the ordinary
/// workflow edit. An already running old worker loses its claim at this fence.
pub(super) fn cancel_stale_in(
    tx: &Transaction<'_>,
    kernel: &str,
    session: &str,
) -> rusqlite::Result<()> {
    let result = (|| -> Result<()> {
        let entries=ids(tx,"SELECT entry_id FROM workflow_gate_entries WHERE session_id=?1 AND json_extract(nullif(body_json,''),'$.admission.kernel_id')=?2 AND disposition NOT IN ('queued','dropped','failed','expired','cancelled')",params![session,kernel])?;
        for id in entries {
            let mut s = load(tx, &id)?.ok_or_else(|| error("gate entry missing"))?;
            if !admission::current_stored(tx, &s)? {
                terminal(
                    tx,
                    &mut s,
                    Disposition::Cancelled,
                    Some("gate_definition_changed".into()),
                )?;
            }
        }
        Ok(())
    })();
    result.map_err(|_| rusqlite::Error::InvalidQuery)
}
