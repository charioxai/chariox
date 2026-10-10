//! MP-08/MP-09/MP-10/MP-11 A10: durable execution receipts, never worker task authority.
use super::*;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedProcessBinding {
    pub worker_kernel_id: String,
    pub worker_machine_id: String,
    pub execution_lease_id: String,
    pub leased_agent_id: String,
    pub home_prompt_id: String,
    pub worker_provider_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedProcessReceipt {
    pub binding: LeasedProcessBinding,
    pub wake: AgentWake,
    pub command_digest: String,
    pub worker_boot_id: String,
    pub process_start_ticks: Option<u64>,
    pub matched_line: Option<String>,
    pub output_tail: String,
    pub revision: u64,
    pub backing_room: String,
    pub backing_agent: String,
}

#[derive(Clone)]
pub(crate) enum LeasedProcessMutation {
    Intent(LeasedProcessReceipt),
    Update {
        id: String,
        expected_revision: u64,
        receipt: LeasedProcessReceipt,
    },
}

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_leased_process_receipts(id TEXT PRIMARY KEY,payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS agent_leased_wake_bindings(id TEXT PRIMARY KEY,payload TEXT NOT NULL);").map_err(sql)
}

pub(super) fn apply(
    tx: &Transaction<'_>,
    mutation: LeasedProcessMutation,
) -> Result<Outcome, DaemonError> {
    let receipt = match mutation {
        LeasedProcessMutation::Intent(receipt) => {
            if let Some(stored) = read(tx, &receipt.wake.id)? {
                if stored.binding != receipt.binding
                    || stored.command_digest != receipt.command_digest
                    || stored.wake.task_id != receipt.wake.task_id
                    || stored.wake.room_id != receipt.wake.room_id
                    || stored.wake.agent_id != receipt.wake.agent_id
                {
                    return Err(error("leased process start identity collision"));
                }
                return Ok(Outcome::LeasedProcess(Some(stored)));
            }
            if receipt.wake.state != "starting"
                || receipt.wake.pid.is_some()
                || receipt.process_start_ticks.is_some()
            {
                return Err(error(
                    "leased process requires a fresh durable launch intent",
                ));
            }
            receipt
        }
        LeasedProcessMutation::Update {
            id,
            expected_revision,
            mut receipt,
        } => {
            let stored = read(tx, &id)?.ok_or_else(|| error("leased process intent missing"))?;
            if stored.revision != expected_revision
                || receipt.binding != stored.binding
                || receipt.command_digest != stored.command_digest
                || receipt.wake.id != id
                || receipt.worker_boot_id != stored.worker_boot_id
                || stored.wake.command != receipt.wake.command
                || stored.wake.room_id != receipt.wake.room_id
                || stored.wake.agent_id != receipt.wake.agent_id
                || stored.wake.task_id != receipt.wake.task_id
                || stored.wake.registration_id != receipt.wake.registration_id
                || stored.wake.match_text != receipt.wake.match_text
                || stored.backing_room != receipt.backing_room
                || stored.backing_agent != receipt.backing_agent
                || stored
                    .process_start_ticks
                    .is_some_and(|ticks| receipt.process_start_ticks != Some(ticks))
                || stored
                    .wake
                    .pid
                    .is_some_and(|pid| receipt.wake.pid != Some(pid))
            {
                return Err(error("leased process receipt identity or revision changed"));
            }
            if matches!(stored.wake.state.as_str(), "exited" | "lost" | "cancelled") {
                return Ok(Outcome::LeasedProcess(Some(stored)));
            }
            receipt.revision = stored.revision + 1;
            receipt
        }
    };
    tx.execute("INSERT INTO agent_leased_process_receipts VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![receipt.wake.id,encode(&receipt)?]).map_err(sql)?;
    Ok(Outcome::LeasedProcess(Some(receipt)))
}

fn read(db: &Connection, id: &str) -> Result<Option<LeasedProcessReceipt>, DaemonError> {
    let row: Option<String> = db
        .query_row(
            "SELECT payload FROM agent_leased_process_receipts WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    row.map(|s| decode(&s)).transpose()
}

impl DurableKernelStateStore {
    pub(crate) fn leased_process_receipt(
        &self,
        id: &str,
    ) -> Result<Option<LeasedProcessReceipt>, DaemonError> {
        {
            let db = self.lock_connection("agent.leased_process.receipt")?;
            read(&db, id)
        }
    }
    pub(crate) fn leased_process_receipts(&self) -> Result<Vec<LeasedProcessReceipt>, DaemonError> {
        let db = self.lock_connection("agent.leased_process.receipts")?;
        let mut q = db
            .prepare("SELECT payload FROM agent_leased_process_receipts")
            .map_err(sql)?;
        let rows = q.query_map([], |r| r.get::<_, String>(0)).map_err(sql)?;
        rows.map(|r| decode(&r.map_err(sql)?)).collect()
    }
    pub(crate) fn leased_wake_binding(
        &self,
        id: &str,
    ) -> Result<Option<LeasedProcessBinding>, DaemonError> {
        let db = self.lock_connection("agent.leased_wake.binding")?;
        let row: Option<String> = db
            .query_row(
                "SELECT payload FROM agent_leased_wake_bindings WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?;
        row.map(|s| decode(&s)).transpose()
    }
}

pub(super) fn create_home_wake(
    tx: &Transaction<'_>,
    task: String,
    prompt: String,
    wake: AgentWake,
    binding: LeasedProcessBinding,
) -> Result<Outcome, DaemonError> {
    let id = wake.id.clone();
    if wake.kind != "process" || binding.home_prompt_id != prompt {
        return Err(error("leased wake origin mismatch"));
    }
    let outcome = super::wakes::apply(tx, Operation::CreateWake { task, prompt, wake })?;
    tx.execute(
        "INSERT INTO agent_leased_wake_bindings VALUES(?1,?2)",
        params![id, encode(&binding)?],
    )
    .map_err(sql)?;
    Ok(outcome)
}
