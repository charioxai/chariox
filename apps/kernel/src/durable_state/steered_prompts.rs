//! A prompt steered into a running turn has no turn of its own: its answer is that turn's.
use super::DurableKernelStateStore;
use crate::error::DaemonError;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub(super) const EVENT_KIND: &str = "session.prompt.steered";

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_steered_prompts (
            session_id TEXT NOT NULL, prompt_id TEXT NOT NULL, task_id TEXT NOT NULL,
            PRIMARY KEY (session_id, prompt_id)
        );
        CREATE TRIGGER IF NOT EXISTS durable_steered_prompts_session_deleted
        AFTER INSERT ON durable_state_events WHEN NEW.kind = 'session.deleted'
        BEGIN DELETE FROM durable_steered_prompts WHERE session_id = NEW.subject_id; END;",
    )
}

/// Records, with the handoff event, the task that received the steered prompt.
pub(super) fn record_in(
    transaction: &Transaction<'_>,
    session_id: Option<&str>,
    payload_json: &str,
) -> rusqlite::Result<()> {
    let payload: serde_json::Value = serde_json::from_str(payload_json)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let (Some(session_id), Some(prompt_id), Some(task_id)) = (
        session_id,
        payload.get("prompt_id").and_then(serde_json::Value::as_str),
        payload
            .get("target_task_id")
            .and_then(serde_json::Value::as_str),
    ) else {
        return Ok(());
    };
    transaction.execute(
        "INSERT OR IGNORE INTO durable_steered_prompts (session_id, prompt_id, task_id)
         VALUES (?1, ?2, ?3)",
        params![session_id, prompt_id, task_id],
    )?;
    Ok(())
}

impl DurableKernelStateStore {
    /// The task whose turn received this steered prompt, when it was steered.
    pub(crate) fn steered_into_task(
        &self,
        session_id: &str,
        prompt_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        self.lock_connection("durable_state.steered_prompts")?
            .query_row(
                "SELECT task_id FROM durable_steered_prompts WHERE session_id = ?1 AND prompt_id = ?2",
                params![session_id, prompt_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.steered_prompts",
                message: error.to_string(),
            })
    }
}
