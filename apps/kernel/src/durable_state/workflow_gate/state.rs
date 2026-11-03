//! Owner controls for persistent gate-owned state; configuration stays revisioned.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub revision: u64,
    pub state: gate::State,
}
impl DurableKernelStateStore {
    pub(crate) fn gate_state(
        &self,
        owner: &str,
        session: &str,
        workflow: &str,
        binding: Option<&str>,
    ) -> Result<Snapshot> {
        let db = self.lock_connection("workflow.gate.state")?;
        let read = |binding: &str| -> Result<(u64, gate::JsonObject)> {
            let row:Option<(i64,String)>=db.query_row("SELECT revision,state_json FROM workflow_gate_state WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND binding_id=?4",params![owner,session,workflow,binding],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
            row.map(|(revision, json)| decode(&json).map(|state| (revision as u64, state)))
                .unwrap_or_else(|| Ok((0, Default::default())))
        };
        let (revision, workflow) = read("")?;
        let binding = binding
            .map(|key| read(key).map(|(_, state)| state))
            .transpose()?
            .unwrap_or_default();
        Ok(Snapshot {
            revision,
            state: gate::State { workflow, binding },
        })
    }
}
pub(super) fn reset(
    tx: &Transaction<'_>,
    owner: &str,
    session: &str,
    workflow: &str,
    reason: &str,
    actor: &str,
) -> Result<()> {
    if reason.trim().is_empty() || reason.len() > 512 {
        return Err(error("state reset requires a reason of at most 512 bytes"));
    }
    for id in ids(tx,"SELECT entry_id FROM workflow_gate_entries WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND disposition NOT IN ('queued','dropped','failed','expired','cancelled')",params![owner,session,workflow])? {
        let mut stored=load(tx,&id)?.ok_or_else(||error("gate reset entry missing"))?;
        terminal(tx,&mut stored,Disposition::Cancelled,Some("gate_state_reset".into()))?;
    }
    tx.execute(
        "DELETE FROM workflow_gate_state WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3",
        params![owner, session, workflow],
    )
    .map_err(sql)?;
    let now = crate::session::unix_epoch_ms();
    tx.execute("INSERT INTO durable_state_events(event_id,kind,subject_id,timestamp_ms,payload_json) VALUES(?1,'workflow.gate.state_reset',?2,?3,?4)",params![format!("gate_reset_{:032x}",rand::random::<u128>()),workflow,now as i64,encode(&serde_json::json!({"owner_id":owner,"session_id":session,"workflow_id":workflow,"actor":actor,"reason":reason}))?]).map_err(sql)?;
    Ok(())
}
