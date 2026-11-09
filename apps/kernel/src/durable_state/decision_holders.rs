//! MP-08 / MP-09 / MP-10 / MP-11 PR5-D1: who holds a pending question and since which hand-on.
//! A pass to the user leaves no inbox event, so the holder is stored with every hand-on and a
//! question registered again after a restart keeps it instead of being handed on afresh.
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use super::DurableKernelStateStore;
use crate::error::DaemonError;

const CHANGED: &str = "decision_holder.changed";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredDecisionHolder {
    pub(crate) room_id: String,
    pub(crate) interaction_id: String,
    /// `None` when the user holds the question.
    pub(crate) holder_agent_id: Option<String>,
    pub(crate) tenure: u32,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS decision_holders (
            interaction_id TEXT PRIMARY KEY, room_id TEXT NOT NULL,
            holder_agent_id TEXT, tenure INTEGER NOT NULL
        );",
    )
}

pub(super) fn apply_event(tx: &Transaction<'_>, kind: &str, payload: &str) -> rusqlite::Result<()> {
    if kind != CHANGED {
        return Ok(());
    }
    let holder: StoredDecisionHolder =
        serde_json::from_str(payload).map_err(|_| rusqlite::Error::InvalidQuery)?;
    tx.execute(
        "INSERT INTO decision_holders (interaction_id, room_id, holder_agent_id, tenure)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(interaction_id) DO UPDATE SET
            room_id=excluded.room_id, holder_agent_id=excluded.holder_agent_id,
            tenure=excluded.tenure",
        params![
            holder.interaction_id,
            holder.room_id,
            holder.holder_agent_id,
            holder.tenure
        ],
    )?;
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn record_decision_holder(
        &self,
        holder: &StoredDecisionHolder,
    ) -> Result<(), DaemonError> {
        let payload = serde_json::to_value(holder).map_err(|error| error_for(error.to_string()))?;
        self.append_event(CHANGED, Some(holder.interaction_id.clone()), payload)?;
        Ok(())
    }

    pub(crate) fn decision_holder(
        &self,
        interaction_id: &str,
    ) -> Result<Option<StoredDecisionHolder>, DaemonError> {
        self.lock_connection("decision_holders.read")?
            .query_row(
                "SELECT room_id, holder_agent_id, tenure FROM decision_holders
                WHERE interaction_id=?1",
                params![interaction_id],
                |row| {
                    Ok(StoredDecisionHolder {
                        room_id: row.get(0)?,
                        interaction_id: interaction_id.to_string(),
                        holder_agent_id: row.get(1)?,
                        tenure: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error_for(error.to_string()))
    }
}

fn error_for(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "decision_holders",
        message,
    }
}
