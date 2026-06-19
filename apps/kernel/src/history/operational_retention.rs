use rusqlite::params;

use crate::error::DaemonError;

use super::{unix_epoch_ms, OperationalHistoryStore};

const OPERATIONAL_HISTORY_WAL_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;

impl OperationalHistoryStore {
    pub fn enforce_size_budget(&self) -> Result<(), DaemonError> {
        let mut total_deleted = 0usize;
        if self.disk_size_bytes() > self.max_size_bytes
            || self.wal_size_bytes() > OPERATIONAL_HISTORY_WAL_CHECKPOINT_BYTES
        {
            self.reclaim_disk_space()?;
        }
        while self.disk_size_bytes() > self.max_size_bytes {
            let deleted = self.prune_oldest_events(self.next_size_prune_batch_len()?)?;
            if deleted == 0 {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "operational history is over hard size budget but has no events to prune",
                    serde_json::json!({
                        "path": self.path().display().to_string(),
                        "size_bytes": self.disk_size_bytes(),
                        "max_size_bytes": self.max_size_bytes,
                    }),
                );
                break;
            }
            total_deleted += deleted;
            self.reclaim_disk_space()?;
        }
        if total_deleted > 0 {
            crate::logging::warn_with_fields(
                "daemon.history",
                "pruned operational history to enforce hard size budget",
                serde_json::json!({
                    "path": self.path().display().to_string(),
                    "deleted_events": total_deleted,
                    "size_bytes": self.disk_size_bytes(),
                    "max_size_bytes": self.max_size_bytes,
                }),
            );
        }
        Ok(())
    }

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

    fn prune_oldest_events(&self, limit: usize) -> Result<usize, DaemonError> {
        let mut connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let mut statement = connection
            .prepare(
                "SELECT event_id, session_id
                 FROM history_events
                 ORDER BY sequence ASC
                 LIMIT ?1",
            )
            .map_err(|error| history_retention_error("prepare oldest history scan", error))?;
        let rows = statement
            .query_map(params![limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|error| history_retention_error("scan oldest history events", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| history_retention_error("read oldest history events", error))?;
        drop(statement);
        if rows.is_empty() {
            return Ok(0);
        }
        let mut session_ids = rows
            .iter()
            .filter_map(|(_, session_id)| session_id.clone())
            .collect::<Vec<_>>();
        session_ids.sort();
        session_ids.dedup();
        let transaction = connection
            .transaction()
            .map_err(|error| history_retention_error("begin operational history prune", error))?;
        for (event_id, _) in &rows {
            transaction
                .execute(
                    "DELETE FROM history_events WHERE event_id = ?1",
                    params![event_id.as_str()],
                )
                .map_err(|error| history_retention_error("delete oldest history event", error))?;
            transaction
                .execute(
                    "DELETE FROM history_archive_outbox WHERE event_id = ?1",
                    params![event_id.as_str()],
                )
                .map_err(|error| {
                    history_retention_error("delete oldest history outbox entry", error)
                })?;
        }
        mark_legacy_fallback_disabled_for_empty_sessions(&transaction, &session_ids)?;
        transaction
            .commit()
            .map_err(|error| history_retention_error("commit operational history prune", error))?;
        Ok(rows.len())
    }

    fn next_size_prune_batch_len(&self) -> Result<usize, DaemonError> {
        let current_size = self.disk_size_bytes();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        let count = connection
            .query_row("SELECT COUNT(*) FROM history_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|value| value.max(0) as usize)
            .map_err(|error| history_retention_error("count operational history events", error))?;
        if count == 0 {
            return Ok(0);
        }
        let excess = current_size.saturating_sub(self.max_size_bytes);
        let ratio = (excess as f64 / current_size.max(1) as f64).clamp(0.0, 1.0);
        let batch = ((count as f64 * (ratio + 0.10)).ceil() as usize).clamp(512, 25_000);
        Ok(batch.min(count))
    }

    fn reclaim_disk_space(&self) -> Result<(), DaemonError> {
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "lock operational history store",
                    message: error.to_string(),
                })?;
        connection
            .execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .map_err(|error| history_retention_error("reclaim operational history disk", error))
    }

    fn disk_size_bytes(&self) -> u64 {
        database_file_size(self.path())
            + database_file_size(&self.path().with_extension("db-wal"))
            + database_file_size(&self.path().with_extension("db-shm"))
    }

    fn wal_size_bytes(&self) -> u64 {
        database_file_size(&self.path().with_extension("db-wal"))
    }
}

fn mark_legacy_fallback_disabled_for_empty_sessions(
    connection: &rusqlite::Transaction<'_>,
    session_ids: &[String],
) -> Result<(), DaemonError> {
    let now = unix_epoch_ms();
    for session_id in session_ids {
        let remaining = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM history_events WHERE session_id = ?1 LIMIT 1)",
                params![session_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| {
                history_retention_error("check operational history after prune", error)
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
                .map_err(|error| {
                    history_retention_error("mark legacy history fallback disabled", error)
                })?;
        }
    }
    Ok(())
}

fn database_file_size(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn history_retention_error(operation: &'static str, error: rusqlite::Error) -> DaemonError {
    DaemonError::SessionHistoryFailed {
        session_id: None,
        operation,
        message: error.to_string(),
    }
}
