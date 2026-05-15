use rusqlite::params;

use crate::error::DaemonError;

use super::{unix_epoch_ms, HistoryEvent, OperationalHistoryStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryArchiveOutboxItem {
    pub event: HistoryEvent,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl OperationalHistoryStore {
    pub fn enqueue_archive_events(&self, events: &[HistoryEvent]) -> Result<(), DaemonError> {
        if events.is_empty() {
            return Ok(());
        }
        let now = unix_epoch_ms();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "lock history archive outbox",
                    message: error.to_string(),
                })?;
        for event in events {
            let event_json = serde_json::to_string(event).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: event.session_id.clone(),
                    operation: "encode history archive outbox event",
                    message: error.to_string(),
                }
            })?;
            connection
                .execute(
                    "INSERT OR IGNORE INTO history_archive_outbox (
                        event_id,
                        event_json,
                        attempts,
                        last_error,
                        archived_at_ms,
                        created_at_ms,
                        updated_at_ms
                    ) VALUES (?1, ?2, 0, NULL, NULL, ?3, ?3)",
                    params![event.event_id.as_str(), event_json, now as i64],
                )
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: event.session_id.clone(),
                    operation: "enqueue history archive outbox event",
                    message: error.to_string(),
                })?;
        }
        Ok(())
    }

    pub fn load_pending_archive_events(
        &self,
        limit: usize,
    ) -> Result<Vec<HistoryArchiveOutboxItem>, DaemonError> {
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "lock history archive outbox",
                    message: error.to_string(),
                })?;
        let mut statement = connection
            .prepare(
                "SELECT event_json, attempts, last_error, created_at_ms, updated_at_ms
                 FROM history_archive_outbox
                 WHERE archived_at_ms IS NULL
                 ORDER BY created_at_ms ASC, event_id ASC
                 LIMIT ?1",
            )
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "prepare history archive outbox load",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![limit.clamp(1, 500) as i64])
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "load history archive outbox",
                message: error.to_string(),
            })?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| DaemonError::SessionHistoryFailed {
                session_id: None,
                operation: "read history archive outbox row",
                message: error.to_string(),
            })?
        {
            let event_json =
                row.get::<_, String>(0)
                    .map_err(|error| DaemonError::SessionHistoryFailed {
                        session_id: None,
                        operation: "read history archive outbox event",
                        message: error.to_string(),
                    })?;
            let event = serde_json::from_str::<HistoryEvent>(&event_json).map_err(|error| {
                DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "decode history archive outbox event",
                    message: error.to_string(),
                }
            })?;
            items.push(HistoryArchiveOutboxItem {
                event,
                attempts: row.get::<_, i64>(1).unwrap_or_default().max(0) as u32,
                last_error: row.get::<_, Option<String>>(2).unwrap_or_default(),
                created_at_ms: row.get::<_, i64>(3).unwrap_or_default().max(0) as u64,
                updated_at_ms: row.get::<_, i64>(4).unwrap_or_default().max(0) as u64,
            });
        }
        Ok(items)
    }

    pub fn mark_archive_events_accepted(&self, event_ids: &[String]) -> Result<(), DaemonError> {
        if event_ids.is_empty() {
            return Ok(());
        }
        let now = unix_epoch_ms();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "lock history archive outbox",
                    message: error.to_string(),
                })?;
        for event_id in event_ids {
            connection
                .execute(
                    "UPDATE history_archive_outbox
                     SET archived_at_ms = ?2, updated_at_ms = ?2, last_error = NULL
                     WHERE event_id = ?1",
                    params![event_id.as_str(), now as i64],
                )
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "mark history archive outbox accepted",
                    message: error.to_string(),
                })?;
        }
        Ok(())
    }

    pub fn mark_archive_events_failed(
        &self,
        event_ids: &[String],
        message: &str,
    ) -> Result<(), DaemonError> {
        if event_ids.is_empty() {
            return Ok(());
        }
        let now = unix_epoch_ms();
        let connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "lock history archive outbox",
                    message: error.to_string(),
                })?;
        for event_id in event_ids {
            connection
                .execute(
                    "UPDATE history_archive_outbox
                     SET attempts = attempts + 1, last_error = ?2, updated_at_ms = ?3
                     WHERE event_id = ?1 AND archived_at_ms IS NULL",
                    params![event_id.as_str(), message, now as i64],
                )
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: None,
                    operation: "mark history archive outbox failed",
                    message: error.to_string(),
                })?;
        }
        Ok(())
    }
}
