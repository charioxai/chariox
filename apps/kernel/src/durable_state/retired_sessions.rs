//! Ended session records remain durable without occupying runtime stores.
use super::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::session::{RuntimeSession, SessionStatus};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS retired_sessions (
        owner_id TEXT NOT NULL, session_id TEXT NOT NULL, project_id TEXT NOT NULL,
        payload TEXT NOT NULL, PRIMARY KEY(owner_id, session_id)
    ); CREATE INDEX IF NOT EXISTS retired_sessions_project ON retired_sessions(owner_id, project_id);")
}

pub(super) fn apply_event(
    transaction: &Transaction<'_>,
    kind: &str,
    payload: &str,
) -> rusqlite::Result<()> {
    if kind == "session.ended" {
        transaction.execute("INSERT INTO retired_sessions(owner_id,session_id,project_id,payload)
            SELECT json_extract(?1,'$.session.host_daemon_id'), json_extract(?1,'$.session.id'),
                   json_extract(?1,'$.session.project_id'), json_extract(?1,'$.session')
            WHERE json_type(?1,'$.session.host_daemon_id')='text'
              AND json_type(?1,'$.session.project_id')='text'
            ON CONFLICT(owner_id,session_id) DO UPDATE SET project_id=excluded.project_id,payload=excluded.payload", [payload])?;
    } else if kind == "session.deleted" || kind == "session.updated" || kind == "session.created" {
        transaction.execute(
            "DELETE FROM retired_sessions
            WHERE owner_id=json_extract(?1,'$.session.host_daemon_id')
              AND session_id=json_extract(?1,'$.session.id')",
            [payload],
        )?;
    }
    Ok(())
}

fn decode(payload: String) -> Result<RuntimeSession, DaemonError> {
    let session: RuntimeSession =
        serde_json::from_str(&payload).map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.read_retired_session",
            message: format!("ended session record is invalid: {error}"),
        })?;
    if session.status() != SessionStatus::Ended {
        return Err(DaemonError::LocalTransport {
            operation: "durable_state.read_retired_session",
            message: "ended session record has a live status".into(),
        });
    }
    Ok(session)
}

impl DurableKernelStateStore {
    pub(crate) fn retired_session(
        &self,
        owner: &str,
        id: &str,
    ) -> Result<Option<RuntimeSession>, DaemonError> {
        let payload: Option<String> = self
            .lock_connection("durable_state.read_retired_session")?
            .query_row(
                "SELECT payload FROM retired_sessions WHERE owner_id=?1 AND session_id=?2",
                params![owner, id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.read_retired_session",
                message: error.to_string(),
            })?;
        payload.map(decode).transpose()
    }

    pub(crate) fn retired_project_sessions(
        &self,
        owner: &str,
        project: &str,
    ) -> Result<Vec<RuntimeSession>, DaemonError> {
        let connection = self.lock_connection("durable_state.read_retired_project_sessions")?;
        let read_error = |error: rusqlite::Error| DaemonError::LocalTransport {
            operation: "durable_state.read_retired_project_sessions",
            message: error.to_string(),
        };
        let mut query = connection.prepare("SELECT payload FROM retired_sessions WHERE owner_id=?1 AND project_id=?2 ORDER BY session_id").map_err(read_error)?;
        let rows = query
            .query_map(params![owner, project], |row| row.get::<_, String>(0))
            .map_err(read_error)?;
        rows.map(|row| decode(row.map_err(read_error)?)).collect()
    }
}
