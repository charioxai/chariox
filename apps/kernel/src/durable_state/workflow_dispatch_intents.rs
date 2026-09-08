//! Generic queue-entry intent, composed with normalized WorkflowRun creation and
//! the existing durable prompt-state event. It is not another prompt queue.
use super::DurableKernelStateStore;
use crate::{
    durable_prompt_state::DurablePromptStateEventPayload,
    error::DaemonError,
    session::{PromptQueueItem, WorkflowQueuedPrompt, WorkflowRun},
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};

pub(crate) const OPERATION_PREFIX: &str = "kernel-workflow-entry:";
#[derive(Debug, Clone)]
pub(crate) struct WorkflowDispatchIntent {
    pub(crate) session_id: String,
    pub(crate) run_id: String,
    pub(crate) node_id: String,
    pub(crate) agent_id: String,
    pub(crate) operation_id: String,
    pub(crate) fingerprint: String,
    pub(crate) submitted: bool,
    pub(crate) queued_prompt: WorkflowQueuedPrompt,
}
pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS durable_workflow_dispatch_intents(
        owner_id TEXT NOT NULL,session_id TEXT NOT NULL,run_id TEXT NOT NULL,node_id TEXT NOT NULL,
        agent_id TEXT NOT NULL,queued_json TEXT NOT NULL,operation_id TEXT NOT NULL UNIQUE,fingerprint TEXT NOT NULL,
        submitted INTEGER NOT NULL DEFAULT 0 CHECK(submitted IN (0,1)),prompt_id TEXT,
        PRIMARY KEY(owner_id,session_id,run_id)
    ); CREATE INDEX IF NOT EXISTS durable_workflow_dispatch_pending ON durable_workflow_dispatch_intents(owner_id,session_id,run_id) WHERE submitted=0;
        CREATE TRIGGER IF NOT EXISTS durable_workflow_dispatch_run_deleted
        AFTER DELETE ON durable_workflow_runs BEGIN
          DELETE FROM durable_workflow_dispatch_intents WHERE owner_id=OLD.owner_id AND session_id=OLD.session_id AND run_id=OLD.run_id;
        END;")
}
pub(super) fn insert_in(
    tx: &Transaction<'_>,
    owner: &str,
    session: &str,
    queued: &WorkflowQueuedPrompt,
    run: &WorkflowRun,
) -> rusqlite::Result<()> {
    let node = run
        .node_runs()
        .first()
        .ok_or(rusqlite::Error::InvalidQuery)?;
    let fingerprint = format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(&serde_json::json!([
                "chariox.workflow-entry.v1",
                owner,
                session,
                run.id(),
                node.id(),
                node.agent_id(),
                run.invocation_prompt()
            ]))
            .map_err(|_| rusqlite::Error::InvalidQuery)?
        )
    );
    let existing = load(tx, owner, session, run.id())?;
    if let Some(existing) = existing {
        if existing.fingerprint != fingerprint
            || existing.node_id != node.id()
            || existing.agent_id != node.agent_id()
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        return Ok(());
    }
    let count:i64=tx.query_row("SELECT count(*) FROM durable_workflow_dispatch_intents WHERE owner_id=?1 AND session_id=?2 AND submitted=0",params![owner,session],|r|r.get(0))?;
    if count >= 1024 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    tx.execute("INSERT INTO durable_workflow_dispatch_intents(owner_id,session_id,run_id,node_id,agent_id,operation_id,fingerprint,queued_json)
      VALUES(?1,?2,?3,?4,?5,?6||lower(hex(randomblob(32))),?7,?8)",params![owner,session,run.id(),node.id(),node.agent_id(),OPERATION_PREFIX,fingerprint,serde_json::to_string(queued).map_err(|_|rusqlite::Error::InvalidQuery)?])?;
    Ok(())
}
fn load(
    connection: &Connection,
    owner: &str,
    session: &str,
    run: &str,
) -> rusqlite::Result<Option<WorkflowDispatchIntent>> {
    connection.query_row("SELECT session_id,run_id,node_id,agent_id,operation_id,fingerprint,submitted,queued_json FROM durable_workflow_dispatch_intents WHERE owner_id=?1 AND session_id=?2 AND run_id=?3",params![owner,session,run],decode).optional()
}
fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowDispatchIntent> {
    Ok(WorkflowDispatchIntent {
        session_id: row.get(0)?,
        run_id: row.get(1)?,
        node_id: row.get(2)?,
        agent_id: row.get(3)?,
        operation_id: row.get(4)?,
        fingerprint: row.get(5)?,
        submitted: row.get(6)?,
        queued_prompt: serde_json::from_str(&row.get::<_, String>(7)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}
impl DurableKernelStateStore {
    pub(crate) fn workflow_dispatch_intent(
        &self,
        owner: &str,
        session: &str,
        run: &str,
    ) -> Result<Option<WorkflowDispatchIntent>, DaemonError> {
        let connection = self.lock_connection("workflow.entry.intent")?;
        load(&connection, owner, session, run)
            .map_err(|error| storage_error("read workflow entry intent", error))
    }
    pub(crate) fn pending_workflow_dispatch_intent(
        &self,
        owner: &str,
        session: &str,
    ) -> Result<Option<WorkflowDispatchIntent>, DaemonError> {
        let connection = self.lock_connection("workflow.entry.pending")?;
        connection.query_row("SELECT i.session_id,i.run_id,i.node_id,i.agent_id,i.operation_id,i.fingerprint,i.submitted,i.queued_json
         FROM durable_workflow_dispatch_intents i JOIN durable_workflow_runs r ON r.owner_id=i.owner_id AND r.session_id=i.session_id AND r.run_id=i.run_id
         WHERE i.owner_id=?1 AND i.session_id=?2 AND i.submitted=0 AND r.status NOT IN ('Completed','Failed','Stopped') ORDER BY r.created_at_ms,i.run_id LIMIT 1",params![owner,session],decode).optional().map_err(|error|storage_error("read pending workflow entry intent",error))
    }
    pub(crate) fn pending_workflow_dispatch_sessions(
        &self,
        owner: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, DaemonError> {
        if !(1..=16).contains(&limit) {
            return Err(storage_error(
                "invalid workflow entry page",
                rusqlite::Error::InvalidQuery,
            ));
        }
        let connection = self.lock_connection("workflow.entry.sessions")?;
        let mut statement=connection.prepare("SELECT DISTINCT i.session_id FROM durable_workflow_dispatch_intents i JOIN durable_workflow_runs r ON r.owner_id=i.owner_id AND r.session_id=i.session_id AND r.run_id=i.run_id WHERE i.owner_id=?1 AND i.submitted=0 AND i.session_id>?2 AND r.status NOT IN ('Completed','Failed','Stopped') ORDER BY i.session_id LIMIT ?3").map_err(|error|storage_error("prepare workflow entry sessions",error))?;
        let rows = statement
            .query_map(params![owner, after.unwrap_or(""), limit as i64], |r| {
                r.get(0)
            })
            .map_err(|error| storage_error("query workflow entry sessions", error))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| storage_error("read workflow entry sessions", error))
    }
}

/// Called in the same transaction AFTER inserting the existing prompt-state
/// event. Private operation metadata is restored from the actual event payload.
/// Prompt completion may remove live queues later; this immutable submission
/// receipt prevents the entry from being replayed after that point.
pub(super) fn record_prompt_state_in(tx: &Transaction<'_>, payload: &str) -> rusqlite::Result<()> {
    let mut event: DurablePromptStateEventPayload =
        serde_json::from_str(payload).map_err(|_| rusqlite::Error::InvalidQuery)?;
    event.restore_private_states();
    for prompt in event
        .active_prompt
        .iter()
        .chain(event.queued_prompts.iter())
    {
        record_prompt(tx, &event.session_id, &event.agent_id, prompt)?;
    }
    Ok(())
}
fn record_prompt(
    tx: &Transaction<'_>,
    session: &str,
    agent: &str,
    prompt: &PromptQueueItem,
) -> rusqlite::Result<()> {
    let Some(operation) = prompt
        .durable_operation_id()
        .filter(|id| id.starts_with(OPERATION_PREFIX))
    else {
        return Ok(());
    };
    let (Some(run), Some(node), Some(fingerprint)) = (
        prompt.workflow_run_id(),
        prompt.workflow_node_run_id(),
        prompt.durable_operation_fingerprint(),
    ) else {
        return Ok(());
    };
    if prompt.target_agent_id() != agent {
        return Err(rusqlite::Error::InvalidQuery);
    }
    // Corroborate against the actual normalized WorkflowRun, not only the
    // private prompt metadata or an uncommitted SessionService projection.
    let encoded: Option<String> = tx.query_row(
        "SELECT r.payload_json FROM durable_workflow_dispatch_intents i
         JOIN durable_workflow_runs r ON r.owner_id=i.owner_id AND r.session_id=i.session_id AND r.run_id=i.run_id
         WHERE i.session_id=?1 AND i.run_id=?2 AND i.node_id=?3 AND i.agent_id=?4 AND i.operation_id=?5 AND i.fingerprint=?6",
        params![session,run,node,agent,operation,fingerprint], |r|r.get(0),
    ).optional()?;
    let Some(encoded) = encoded else {
        return Ok(());
    };
    let persisted: WorkflowRun =
        serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if persisted.id() != run
        || !persisted
            .node_runs()
            .first()
            .is_some_and(|entry| entry.id() == node && entry.agent_id() == agent)
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    // Exact trusted intent corroborates the private workflow context; an external
    // command choosing the reserved prefix cannot turn an unrelated prompt into
    // a submitted workflow entry. Zero rows means no such kernel intent exists.
    tx.execute("UPDATE durable_workflow_dispatch_intents SET submitted=1,prompt_id=coalesce(prompt_id,?1)
       WHERE session_id=?2 AND run_id=?3 AND node_id=?4 AND agent_id=?5 AND operation_id=?6 AND fingerprint=?7",
       params![prompt.id(),session,run,node,agent,operation,fingerprint])?;
    Ok(())
}

fn storage_error(operation: &'static str, error: rusqlite::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}
