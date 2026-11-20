//! The durable event trail of the agent ledger.
use super::*;
pub(super) fn audit(
    tx: &Transaction<'_>,
    kind: &str,
    id: &str,
    payload: serde_json::Value,
) -> Result<(), DaemonError> {
    tx.execute("INSERT INTO durable_state_events(event_id,kind,subject_id,timestamp_ms,payload_json) VALUES(?1,?2,?3,?4,?5)",params![format!("agent-ledger-{:032x}",rand::random::<u128>()),kind,id,sql_integer(crate::session::unix_epoch_ms())?,encode(&payload)?]).map_err(sql)?;
    Ok(())
}
