//! MP-08/MP-10/MP-11: settlement receipts survive interrupted hold removal.
use super::DurableKernelStateStore;
use crate::error::DaemonError;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_trigger_firings (
            session_id TEXT NOT NULL, prompt_id TEXT NOT NULL,
            PRIMARY KEY (session_id, prompt_id)
        );
        CREATE TRIGGER IF NOT EXISTS durable_trigger_firings_session_deleted
        AFTER INSERT ON durable_state_events WHEN NEW.kind = 'session.deleted'
        BEGIN DELETE FROM durable_trigger_firings WHERE session_id = NEW.subject_id; END;",
    )
}

pub(super) fn record_settlement_in(
    transaction: &Transaction<'_>,
    session_id: Option<&str>,
    payload_json: &str,
) -> rusqlite::Result<()> {
    let payload: serde_json::Value = serde_json::from_str(payload_json)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let (Some(session_id), Some(prompt_id)) = (
        session_id,
        payload
            .get("settled_trigger_prompt_id")
            .and_then(serde_json::Value::as_str),
    ) else {
        return Ok(());
    };
    transaction.execute(
        "INSERT OR IGNORE INTO durable_trigger_firings (session_id, prompt_id) VALUES (?1, ?2)",
        params![session_id, prompt_id],
    )?;
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn trigger_firing_settled(
        &self,
        session_id: &str,
        prompt_id: &str,
    ) -> Result<bool, DaemonError> {
        self.lock_connection("durable_state.trigger_firings")?
            .query_row(
                "SELECT 1 FROM durable_trigger_firings WHERE session_id = ?1 AND prompt_id = ?2",
                params![session_id, prompt_id],
                |_| Ok(()),
            )
            .optional()
            .map(|receipt| receipt.is_some())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.trigger_firings",
                message: error.to_string(),
            })
    }
}
