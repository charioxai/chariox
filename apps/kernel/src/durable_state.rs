use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

#[derive(Debug, Clone)]
pub struct DurableKernelStateStore {
    path: PathBuf,
    connection: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableStateEvent {
    pub sequence: u64,
    pub event_id: String,
    pub kind: String,
    pub subject_id: Option<String>,
    pub timestamp_ms: u64,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableStateSnapshot {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub payload: serde_json::Value,
}

impl DurableKernelStateStore {
    pub fn open(path: PathBuf) -> Result<Self, DaemonError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.open",
                message: error.to_string(),
            })?;
        }
        let connection = Connection::open(&path).map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.open",
            message: error.to_string(),
        })?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.wal",
                message: error.to_string(),
            })?;
        connection
            .execute_batch(DURABLE_STATE_SCHEMA)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.migrate",
                message: error.to_string(),
            })?;
        Ok(Self {
            path,
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn append_event(
        &self,
        kind: impl Into<String>,
        subject_id: Option<String>,
        payload: serde_json::Value,
    ) -> Result<DurableStateEvent, DaemonError> {
        let timestamp_ms = unix_epoch_ms();
        let event_id = format!("state_evt_{timestamp_ms}_{}", rand_suffix());
        let kind = kind.into();
        let payload_json =
            serde_json::to_string(&payload).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.encode_event",
                message: error.to_string(),
            })?;
        let connection = self.lock_connection("durable_state.append_event")?;
        connection
            .execute(
                "INSERT INTO durable_state_events (
                    event_id,
                    kind,
                    subject_id,
                    timestamp_ms,
                    payload_json
                ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    event_id.as_str(),
                    kind.as_str(),
                    subject_id.as_deref(),
                    timestamp_ms as i64,
                    payload_json,
                ],
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.append_event",
                message: error.to_string(),
            })?;
        let sequence = connection.last_insert_rowid().max(0) as u64;
        Ok(DurableStateEvent {
            sequence,
            event_id,
            kind,
            subject_id,
            timestamp_ms,
            payload,
        })
    }

    pub fn load_events_after(&self, sequence: u64) -> Result<Vec<DurableStateEvent>, DaemonError> {
        let connection = self.lock_connection("durable_state.load_events")?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, event_id, kind, subject_id, timestamp_ms, payload_json
                 FROM durable_state_events
                 WHERE sequence > ?1
                 ORDER BY sequence ASC",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_events",
                message: error.to_string(),
            })?;
        let mut rows = statement.query(params![sequence as i64]).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "durable_state.load_events",
                message: error.to_string(),
            }
        })?;
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.load_events",
            message: error.to_string(),
        })? {
            let payload_json =
                row.get::<_, String>(5)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "durable_state.load_events",
                        message: error.to_string(),
                    })?;
            events.push(DurableStateEvent {
                sequence: row.get::<_, i64>(0).unwrap_or_default().max(0) as u64,
                event_id: row.get::<_, String>(1).unwrap_or_default(),
                kind: row.get::<_, String>(2).unwrap_or_default(),
                subject_id: row.get::<_, Option<String>>(3).unwrap_or_default(),
                timestamp_ms: row.get::<_, i64>(4).unwrap_or_default().max(0) as u64,
                payload: serde_json::from_str(&payload_json).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "durable_state.decode_event",
                        message: error.to_string(),
                    }
                })?,
            });
        }
        Ok(events)
    }

    pub fn latest_event_sequence(&self) -> Result<u64, DaemonError> {
        let connection = self.lock_connection("durable_state.latest_event_sequence")?;
        let sequence = connection
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM durable_state_events",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.latest_event_sequence",
                message: error.to_string(),
            })?;
        Ok(sequence.max(0) as u64)
    }

    pub fn latest_snapshot_sequence(&self) -> Result<u64, DaemonError> {
        let connection = self.lock_connection("durable_state.latest_snapshot_sequence")?;
        let sequence = connection
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM durable_state_snapshots",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.latest_snapshot_sequence",
                message: error.to_string(),
            })?;
        Ok(sequence.max(0) as u64)
    }

    pub fn save_snapshot(
        &self,
        sequence: u64,
        payload: serde_json::Value,
    ) -> Result<DurableStateSnapshot, DaemonError> {
        let timestamp_ms = unix_epoch_ms();
        let payload_json =
            serde_json::to_string(&payload).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.encode_snapshot",
                message: error.to_string(),
            })?;
        let connection = self.lock_connection("durable_state.save_snapshot")?;
        connection
            .execute(
                "INSERT INTO durable_state_snapshots (
                    sequence,
                    timestamp_ms,
                    payload_json
                ) VALUES (?1, ?2, ?3)",
                params![sequence as i64, timestamp_ms as i64, payload_json],
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.save_snapshot",
                message: error.to_string(),
            })?;
        Ok(DurableStateSnapshot {
            sequence,
            timestamp_ms,
            payload,
        })
    }

    pub fn latest_snapshot(&self) -> Result<Option<DurableStateSnapshot>, DaemonError> {
        let connection = self.lock_connection("durable_state.latest_snapshot")?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, timestamp_ms, payload_json
                 FROM durable_state_snapshots
                 ORDER BY sequence DESC, snapshot_id DESC
                 LIMIT 1",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.latest_snapshot",
                message: error.to_string(),
            })?;
        let result = statement.query_row([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        });
        let (sequence, timestamp_ms, payload_json) = match result {
            Ok(value) => value,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(error) => {
                return Err(DaemonError::LocalTransport {
                    operation: "durable_state.latest_snapshot",
                    message: error.to_string(),
                })
            }
        };
        Ok(Some(DurableStateSnapshot {
            sequence: sequence.max(0) as u64,
            timestamp_ms: timestamp_ms.max(0) as u64,
            payload: serde_json::from_str(&payload_json).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "durable_state.decode_snapshot",
                    message: error.to_string(),
                }
            })?,
        }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock_connection(
        &self,
        operation: &'static str,
    ) -> Result<std::sync::MutexGuard<'_, Connection>, DaemonError> {
        self.connection
            .lock()
            .map_err(|error| DaemonError::LocalTransport {
                operation,
                message: error.to_string(),
            })
    }
}

const DURABLE_STATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS durable_state_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    subject_id TEXT,
    timestamp_ms INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_durable_state_events_kind_subject
    ON durable_state_events(kind, subject_id);

CREATE TABLE IF NOT EXISTS durable_state_snapshots (
    snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,
    sequence INTEGER NOT NULL,
    timestamp_ms INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_durable_state_snapshots_sequence
    ON durable_state_snapshots(sequence);
"#;

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn rand_suffix() -> u64 {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    rng.next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_state_store_appends_events_and_loads_latest_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "arroba-durable-state-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");

        let first = store
            .append_event(
                "session.created",
                Some("session-1".to_string()),
                serde_json::json!({"session_id": "session-1"}),
            )
            .expect("first event should append");
        let second = store
            .append_event(
                "agent.created",
                Some("agent-1".to_string()),
                serde_json::json!({"agent_id": "agent-1"}),
            )
            .expect("second event should append");

        assert!(second.sequence > first.sequence);
        assert_eq!(
            store
                .load_events_after(first.sequence)
                .expect("events should load")
                .len(),
            1
        );
        store
            .save_snapshot(
                second.sequence,
                serde_json::json!({"sessions": ["session-1"]}),
            )
            .expect("snapshot should save");

        drop(store);
        let store = DurableKernelStateStore::open(path.clone()).expect("store should reopen");
        let latest = store
            .latest_snapshot()
            .expect("latest snapshot should load")
            .expect("snapshot should exist");
        assert_eq!(latest.sequence, second.sequence);
        assert_eq!(latest.payload["sessions"][0], "session-1");
        assert_eq!(
            store
                .latest_event_sequence()
                .expect("latest event sequence should load"),
            second.sequence
        );
        assert_eq!(
            store
                .latest_snapshot_sequence()
                .expect("latest snapshot sequence should load"),
            second.sequence
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
