//! MP-09 A03: retain each fire's receipts, including late receipts after another fire.
use super::*;
use serde::Deserialize;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct WakeFireReceipt {
    pub wake_id: String,
    pub sequence: u64,
    pub fired_at_ms: u64,
    pub delivered_at_ms: Option<u64>,
    #[serde(default)]
    pub turn_started_at_ms: Option<u64>,
    pub acknowledged_at_ms: Option<u64>,
    pub delivery: String,
    pub alerted: bool,
}

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_wake_receipts(wake_id TEXT NOT NULL, sequence INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(wake_id,sequence));").map_err(sql)
}

impl DurableKernelStateStore {
    /// MP-09: provider cold admission has its own bounded delivery clock;
    /// waiting for a wake did not constitute model progress.
    pub(crate) fn agent_has_live_wake_admission(
        &self,
        task: &AgentTaskExecution,
        clock: impl FnOnce() -> u64,
    ) -> Result<bool, DaemonError> {
        // Admission promotes the pending prompt before provider I/O starts.
        // Its exact submitting receipt remains the bounded liveness witness.
        let prompt = task.pending_prompt_id.as_deref().unwrap_or(&task.prompt_id);
        let event = self.agent_event_for_prompt(&task.room_id, &task.agent_id, prompt)?;
        // A sweep may await recovery before this read. Sample afterward so its
        // newly committed receipt is not rejected against the sweep-start clock.
        let now = clock();
        Ok(event.iter().any(|event| {
            event.state == "submitting"
                && event.prompt_id.as_deref() == Some(prompt)
                && event
                    .attempted_at_ms
                    .is_some_and(|started| started <= now && now - started < DELIVERY_TIMEOUT_MS)
                && (event.payload["task_id"].as_str() == Some(task.task_id.as_str())
                    || event.payload["task_ids"].as_array().is_some_and(|ids| {
                        ids.iter()
                            .any(|id| id.as_str() == Some(task.task_id.as_str()))
                    }))
        }))
    }

    pub(crate) fn agent_wake_receipts(
        &self,
        room: Option<&str>,
        agent: Option<&str>,
    ) -> Result<Vec<WakeFireReceipt>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.wake_receipts")?;
        let mut q = db.prepare("SELECT r.payload FROM agent_wake_receipts r JOIN agent_wakes w ON w.id=r.wake_id WHERE (?1 IS NULL OR w.room_id=?1) AND (?2 IS NULL OR w.agent_id=?2) AND ((json_extract(r.payload,'$.delivered_at_ms') IS NULL AND json_extract(r.payload,'$.delivery') NOT IN ('expired','failed')) OR r.rowid IN (SELECT r2.rowid FROM agent_wake_receipts r2 JOIN agent_wakes w2 ON w2.id=r2.wake_id WHERE (?1 IS NULL OR w2.room_id=?1) AND (?2 IS NULL OR w2.agent_id=?2) ORDER BY r2.rowid DESC LIMIT 128)) ORDER BY r.sequence").map_err(sql)?;
        let rows = q
            .query_map(params![room, agent], |r| r.get::<_, String>(0))
            .map_err(sql)?;
        rows.map(|row| decode(&row.map_err(sql)?)).collect()
    }
}

fn save(tx: &Transaction<'_>, r: &WakeFireReceipt) -> Result<(), DaemonError> {
    tx.execute("INSERT INTO agent_wake_receipts VALUES(?1,?2,?3) ON CONFLICT(wake_id,sequence) DO UPDATE SET payload=excluded.payload", params![r.wake_id,sql_integer(r.sequence)?,encode(r)?]).map_err(sql)?;
    Ok(())
}

/// Per-wake bound on settled receipts; undelivered ones are always kept.
const SETTLED_RECEIPTS_RETAINED: i64 = 64;

pub(super) fn fire(
    tx: &Transaction<'_>,
    wake: &AgentWake,
    e: &InboxEvent,
    now: u64,
) -> Result<(), DaemonError> {
    tx.execute("DELETE FROM agent_wake_receipts WHERE wake_id=?1 AND sequence IN (SELECT sequence FROM agent_wake_receipts WHERE wake_id=?1 AND (json_extract(payload,'$.delivered_at_ms') IS NOT NULL OR json_extract(payload,'$.delivery') IN ('expired','failed')) ORDER BY sequence DESC LIMIT -1 OFFSET ?2)", params![wake.id, SETTLED_RECEIPTS_RETAINED - 1]).map_err(sql)?;
    save(
        tx,
        &WakeFireReceipt {
            wake_id: wake.id.clone(),
            sequence: e.sequence,
            fired_at_ms: now,
            delivered_at_ms: None,
            turn_started_at_ms: None,
            acknowledged_at_ms: None,
            delivery: e.state.clone(),
            alerted: false,
        },
    )
}

pub(super) fn delivery(tx: &Transaction<'_>, e: &InboxEvent, now: u64) -> Result<(), DaemonError> {
    let payload: Option<String> = tx
        .query_row(
            "SELECT payload FROM agent_wake_receipts WHERE wake_id=?1 AND sequence=?2",
            params![e.source_id, sql_integer(e.sequence)?],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    let Some(payload) = payload else {
        return Ok(());
    };
    let mut receipt: WakeFireReceipt = decode(&payload)?;
    receipt.delivery = e.state.clone();
    if e.state == "accepted" {
        receipt.turn_started_at_ms.get_or_insert(now);
    }
    if matches!(e.state.as_str(), "accepted" | "acknowledged" | "handled") {
        receipt.delivered_at_ms.get_or_insert(now);
    }
    if matches!(e.state.as_str(), "acknowledged" | "handled") {
        receipt.acknowledged_at_ms.get_or_insert(now);
    }
    save(tx, &receipt)
}

pub(super) fn alerted(tx: &Transaction<'_>, wake: &str, sequence: u64) -> Result<(), DaemonError> {
    tx.execute("UPDATE agent_wake_receipts SET payload=json_set(payload,'$.alerted',json('true')) WHERE wake_id=?1 AND sequence=?2", params![wake,sql_integer(sequence)?]).map_err(sql)?;
    Ok(())
}
