//! MP-08/MP-10/MP-11: a resend is consumed by durable prompt admission, even
//! after its replacement finishes or a later snapshot contains stale state.
use super::DurableKernelStateStore;
use crate::{
    durable_prompt_state::DurablePromptStateEventPayload, error::DaemonError,
    session::PromptQueueItem,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub(crate) struct TriggerResendReceipt {
    pub(crate) session_id: String,
    pub(crate) original_prompt_id: String,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_trigger_resends (
            session_id TEXT NOT NULL, original_prompt_id TEXT NOT NULL,
            replacement_prompt_id TEXT NOT NULL,
            PRIMARY KEY (session_id, original_prompt_id)
        );
        CREATE TRIGGER IF NOT EXISTS durable_trigger_resends_session_deleted
        AFTER INSERT ON durable_state_events WHEN NEW.kind = 'session.deleted'
        BEGIN DELETE FROM durable_trigger_resends WHERE session_id = NEW.subject_id; END;",
    )
}

/// The existing event and its receipt commit in the same writer transaction.
pub(super) fn record_prompt_state_in(tx: &Transaction<'_>, payload: &str) -> rusqlite::Result<()> {
    if !payload.contains(PromptQueueItem::TRIGGER_RESEND_OPERATION_PREFIX) {
        return Ok(());
    }
    let mut event: DurablePromptStateEventPayload =
        serde_json::from_str(payload).map_err(|_| rusqlite::Error::InvalidQuery)?;
    event.restore_private_states();
    for prompt in event
        .active_prompt
        .iter()
        .chain(event.queued_prompts.iter())
    {
        let Some(original) = prompt.trigger_resend_of() else {
            continue;
        };
        if prompt.target_agent_id() != event.agent_id {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let replacement = prompt.pending_prompt_id().unwrap_or(prompt.id());
        tx.execute(
            "INSERT INTO durable_trigger_resends (session_id, original_prompt_id, replacement_prompt_id)
             VALUES (?1, ?2, ?3) ON CONFLICT (session_id, original_prompt_id) DO NOTHING",
            params![event.session_id, original, replacement],
        )?;
        let recorded: String = tx.query_row(
            "SELECT replacement_prompt_id FROM durable_trigger_resends
             WHERE session_id = ?1 AND original_prompt_id = ?2",
            params![event.session_id, original],
            |row| row.get(0),
        )?;
        if recorded != replacement {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                Some("trigger resend already admitted a different replacement prompt; do not send it again".into()),
            ));
        }
    }
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn trigger_resend_admitted(
        &self,
        session_id: &str,
        original_prompt_id: &str,
    ) -> Result<bool, DaemonError> {
        self.lock_connection("durable_state.trigger_resends")?
            .query_row(
                "SELECT 1 FROM durable_trigger_resends WHERE session_id = ?1 AND original_prompt_id = ?2",
                params![session_id, original_prompt_id],
                |_| Ok(()),
            )
            .optional()
            .map(|receipt| receipt.is_some())
            .map_err(storage_error)
    }

    pub(crate) fn admitted_trigger_resends(
        &self,
    ) -> Result<Vec<TriggerResendReceipt>, DaemonError> {
        let connection = self.lock_connection("durable_state.trigger_resends")?;
        let mut statement = connection
            .prepare("SELECT session_id, original_prompt_id FROM durable_trigger_resends")
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok(TriggerResendReceipt {
                    session_id: row.get(0)?,
                    original_prompt_id: row.get(1)?,
                })
            })
            .map_err(storage_error)?;
        rows.collect::<rusqlite::Result<_>>().map_err(storage_error)
    }
}

fn storage_error(error: rusqlite::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "durable_state.trigger_resends",
        message: error.to_string(),
    }
}
