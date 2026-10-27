//! MP-08 / MP-10 / MP-11 A02: preserve damaged bytes, project a safe owner block.
use super::*;
pub(super) fn retain(
    tx: &Transaction<'_>,
    kind: &str,
    id: &str,
    bytes: &str,
) -> Result<(), DaemonError> {
    tx.execute("INSERT INTO agent_lifecycle_quarantine(kind,id,payload,at_ms) VALUES(?1,?2,?3,?4) ON CONFLICT(kind,id) DO NOTHING",params![kind,id,bytes,sql_integer(crate::session::unix_epoch_ms())?]).map_err(sql)?;
    Ok(())
}
pub(super) fn task(id: String, room: String, agent: String, prompt: String) -> AgentTaskExecution {
    let mut t = new_task(room, agent, prompt, None, crate::session::unix_epoch_ms());
    t.task_id = id;
    t.state = ExecutionState::Blocked;
    t.blocked_revision = t.revision;
    t.reason="Task ledger is quarantined; owner must restore authoritative state or cancel. Obligation coverage is unknown".into();
    t
}

pub(super) fn registrations(tx: &Transaction<'_>, task: &str) -> Result<(), DaemonError> {
    let mut q = tx
        .prepare("SELECT id,payload FROM agent_registrations WHERE task_id=?1")
        .map_err(sql)?;
    let rows = q
        .query_map([task], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(sql)?;
    for row in rows {
        let (id, raw) = row.map_err(sql)?;
        retain(tx, "registration", &id, &raw)?;
    }
    Ok(())
}
