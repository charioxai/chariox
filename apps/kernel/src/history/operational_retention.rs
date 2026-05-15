use rusqlite::params;

use crate::error::DaemonError;

use super::{unix_epoch_ms, OperationalHistoryStore};

impl OperationalHistoryStore {
    pub fn prune_events_before(
        &self,
        cutoff_timestamp_ms: u64,
        allow_unarchived_delete: bool,
    ) -> Result<usize, DaemonError> {
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let mut session_statement = connection
            .prepare(
                "SELECT DISTINCT session_id
                 FROM history_events
                 WHERE timestamp_ms < ?1 AND session_id IS NOT NULL",
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "prepare operational history prune session scan",
                message: error.to_string(),
            })?;
        let session_ids = session_statement
            .query_map(params![cutoff_timestamp_ms as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "scan operational history prune sessions",
                message: error.to_string(),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "read operational history prune sessions",
                message: error.to_string(),
            })?;
        drop(session_statement);

        let deleted = if allow_unarchived_delete {
            connection
                .execute(
                    "DELETE FROM history_events WHERE timestamp_ms < ?1",
                    params![cutoff_timestamp_ms as i64],
                )
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "prune operational history events",
                    message: error.to_string(),
                })?
        } else {
            connection
                .execute(
                    "DELETE FROM history_events
                     WHERE timestamp_ms < ?1
                       AND event_id IN (
                         SELECT event_id
                         FROM history_archive_outbox
                         WHERE archived_at_ms IS NOT NULL
                       )",
                    params![cutoff_timestamp_ms as i64],
                )
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "prune archived operational history events",
                    message: error.to_string(),
                })?
        };
        let now = unix_epoch_ms();
        for session_id in session_ids {
            let remaining = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM history_events WHERE session_id = ?1 LIMIT 1)",
                    params![session_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.clone()),
                    operation: "check operational history after prune",
                    message: error.to_string(),
                })?;
            if remaining == 0 {
                connection
                    .execute(
                        "INSERT INTO history_session_markers (
                            session_id,
                            legacy_fallback_disabled_at_ms
                         ) VALUES (?1, ?2)
                         ON CONFLICT(session_id) DO UPDATE SET
                            legacy_fallback_disabled_at_ms = excluded.legacy_fallback_disabled_at_ms",
                        params![session_id.as_str(), now as i64],
                    )
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: Some(session_id.clone()),
                        operation: "mark legacy history fallback disabled",
                        message: error.to_string(),
                    })?;
            }
        }
        Ok(deleted)
    }
}
