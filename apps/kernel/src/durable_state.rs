// Durable state follows the kernel-wide `DaemonError` contract. Boxing that shared error is a
// cross-cutting API change, so keep these storage-boundary results explicit here.
#![allow(clippy::result_large_err)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

pub(crate) mod workflow_runtime;

#[derive(Debug, Clone)]
pub struct DurableKernelStateStore {
    path: PathBuf,
    connection: Arc<Mutex<Connection>>,
    writer: Arc<DurableStateWriter>,
}

const DURABLE_WRITE_QUEUE_CAPACITY: usize = 4_096;
const DURABLE_WRITE_BATCH_LIMIT: usize = 256;
const DURABLE_WRITE_BATCH_WINDOW: Duration = Duration::from_millis(5);

#[derive(Debug)]
struct DurableStateWriter {
    sender: Mutex<Option<SyncSender<DurableWriteRequest>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    health: Arc<DurableWriterHealth>,
}

#[derive(Debug, Default)]
struct DurableWriterHealth {
    committed_batches: AtomicU64,
    committed_records: AtomicU64,
    max_batch_records: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DurableWriterHealthSnapshot {
    pub(crate) committed_batches: u64,
    pub(crate) committed_records: u64,
    pub(crate) max_batch_records: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DurableEventTailStatistics {
    pub(crate) event_count: u64,
    pub(crate) encoded_bytes: u64,
    pub(crate) oldest_timestamp_ms: Option<u64>,
    pub(crate) latest_sequence: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DurableCheckpointMarker {
    pub(crate) sequence: u64,
    pub(crate) timestamp_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DurableIncrementalReclaimOutcome {
    pub(crate) supported: bool,
    pub(crate) free_pages_before: u64,
    pub(crate) free_pages_after: u64,
}

#[derive(Debug)]
struct DurableWriteRequest {
    operation: DurableWriteOperation,
    response: mpsc::Sender<Result<u64, String>>,
}

#[derive(Debug)]
enum DurableWriteOperation {
    Event {
        event_id: String,
        kind: String,
        subject_id: Option<String>,
        timestamp_ms: u64,
        payload_json: String,
    },
    Snapshot {
        sequence: u64,
        timestamp_ms: u64,
        payload_json: String,
    },
    EntityCheckpoint {
        owner_id: String,
        sequence: u64,
        timestamp_ms: u64,
        entities: Vec<DurableCheckpointEntity>,
    },
    WorkflowRuntimeTransition {
        event_id: String,
        timestamp_ms: u64,
        payload_json: String,
        owner_id: String,
        session_id: String,
        hot_entities: Vec<DurableWorkflowHotEntityWrite>,
        workflow_runs: Vec<DurableWorkflowRunWrite>,
        delivery_receipts: Vec<DurableDeliveryReceiptWrite>,
    },
    WorkflowRuntimeSessionsTransition {
        event_id: String,
        timestamp_ms: u64,
        payload_json: String,
        owner_id: String,
        sessions: Vec<DurableWorkflowSessionWrite>,
    },
    WorkflowRuntimeMigration {
        owner_id: String,
        hot_entities: Vec<DurableWorkflowHotEntityWrite>,
        workflow_runs: Vec<DurableWorkflowRunWrite>,
        delivery_receipts: Vec<DurableDeliveryReceiptWrite>,
    },
    WorkflowHistoryMigrationChunk {
        owner_id: String,
        workflow_runs: Vec<DurableWorkflowRunWrite>,
        completed: bool,
    },
    SessionDelete {
        event_id: String,
        timestamp_ms: u64,
        payload_json: String,
        owner_id: String,
        session_id: String,
    },
}

#[derive(Debug)]
struct DurableWorkflowRunWrite {
    session_id: String,
    run_id: String,
    workflow_id: String,
    status: String,
    created_at_ms: u64,
    completed_at_ms: Option<u64>,
    payload_json: String,
}

#[derive(Debug)]
struct DurableWorkflowHotEntityWrite {
    session_id: String,
    entity_kind: String,
    entity_id: String,
    payload_json: String,
}

#[derive(Debug)]
struct DurableDeliveryReceiptWrite {
    session_id: String,
    delivery_id: String,
    binding_id: String,
    expires_at_ms: u64,
    payload_json: String,
}

#[derive(Debug)]
struct DurableWorkflowSessionWrite {
    session_id: String,
    hot_entities: Vec<DurableWorkflowHotEntityWrite>,
    workflow_runs: Vec<DurableWorkflowRunWrite>,
    delivery_receipts: Vec<DurableDeliveryReceiptWrite>,
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

#[derive(Debug, Clone)]
pub(crate) struct DurableCheckpointEntity {
    pub(crate) kind: String,
    pub(crate) id: String,
    pub(crate) payload_json: String,
}

impl DurableKernelStateStore {
    pub fn open(path: PathBuf) -> Result<Self, DaemonError> {
        let initialize_incremental_vacuum = fs::metadata(&path)
            .map(|metadata| metadata.len() == 0)
            .unwrap_or(true);
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
        if initialize_incremental_vacuum {
            connection
                .pragma_update(None, "auto_vacuum", "INCREMENTAL")
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "durable_state.incremental_vacuum_mode",
                    message: error.to_string(),
                })?;
        }
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
        let writer = DurableStateWriter::start(&path)?;
        connection
            .pragma_update(None, "query_only", true)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.read_only",
                message: error.to_string(),
            })?;
        Ok(Self {
            path,
            connection: Arc::new(Mutex::new(connection)),
            writer: Arc::new(writer),
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
        let sequence = self.writer.execute(DurableWriteOperation::Event {
            event_id: event_id.clone(),
            kind: kind.clone(),
            subject_id: subject_id.clone(),
            timestamp_ms,
            payload_json,
        })?;
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

    pub fn load_events_after_batch(
        &self,
        sequence: u64,
        limit: usize,
    ) -> Result<Vec<DurableStateEvent>, DaemonError> {
        let limit = limit.clamp(1, 4_096);
        let connection = self.lock_connection("durable_state.load_events_after_batch")?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, event_id, kind, subject_id, timestamp_ms, payload_json
                 FROM durable_state_events
                 WHERE sequence > ?1
                 ORDER BY sequence ASC
                 LIMIT ?2",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_events_after_batch",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![sequence as i64, limit as i64])
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_events_after_batch",
                message: error.to_string(),
            })?;
        let mut events = Vec::with_capacity(limit);
        while let Some(row) = rows.next().map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.load_events_after_batch",
            message: error.to_string(),
        })? {
            let payload_json =
                row.get::<_, String>(5)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "durable_state.load_events_after_batch",
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
                        operation: "durable_state.decode_event_batch",
                        message: error.to_string(),
                    }
                })?,
            });
        }
        Ok(events)
    }

    pub(crate) fn load_restore_events_after_batch(
        &self,
        sequence: u64,
        limit: usize,
    ) -> Result<Vec<DurableStateEvent>, DaemonError> {
        let limit = limit.clamp(1, 4_096);
        let connection = self.lock_connection("durable_state.load_restore_events_after_batch")?;
        let mut statement = connection
            .prepare(
                "SELECT event.sequence, event.event_id, event.kind, event.subject_id,
                        event.timestamp_ms, event.payload_json
                 FROM durable_state_events event
                 WHERE event.sequence > ?1
                   AND event.kind <> 'workflow.runtime.updated'
                   AND (
                     event.kind <> 'session.updated'
                     OR event.subject_id IS NULL
                     OR event.sequence = (
                       SELECT MAX(latest.sequence)
                       FROM durable_state_events latest
                       WHERE latest.sequence > ?1
                         AND latest.subject_id = event.subject_id
                         AND latest.kind = 'session.updated'
                     )
                   )
                 ORDER BY event.sequence ASC
                 LIMIT ?2",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_restore_events_after_batch",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![sequence as i64, limit as i64])
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_restore_events_after_batch",
                message: error.to_string(),
            })?;
        let mut events = Vec::with_capacity(limit);
        while let Some(row) = rows.next().map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.load_restore_events_after_batch",
            message: error.to_string(),
        })? {
            let payload_json =
                row.get::<_, String>(5)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "durable_state.load_restore_events_after_batch",
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
                        operation: "durable_state.decode_restore_event_batch",
                        message: error.to_string(),
                    }
                })?,
            });
        }
        Ok(events)
    }

    pub fn load_events_by_kind(&self, kind: &str) -> Result<Vec<DurableStateEvent>, DaemonError> {
        let connection = self.lock_connection("durable_state.load_events_by_kind")?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, event_id, kind, subject_id, timestamp_ms, payload_json
                 FROM durable_state_events
                 WHERE kind = ?1
                 ORDER BY sequence ASC",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_events_by_kind",
                message: error.to_string(),
            })?;
        let mut rows =
            statement
                .query(params![kind])
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "durable_state.load_events_by_kind",
                    message: error.to_string(),
                })?;
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.load_events_by_kind",
            message: error.to_string(),
        })? {
            let payload_json =
                row.get::<_, String>(5)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "durable_state.load_events_by_kind",
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

    pub fn load_subject_events(
        &self,
        subject_id: &str,
        limit: usize,
    ) -> Result<Vec<DurableStateEvent>, DaemonError> {
        let limit = limit.clamp(1, 200);
        let connection = self.lock_connection("durable_state.load_subject_events")?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, event_id, kind, subject_id, timestamp_ms, payload_json
                 FROM durable_state_events
                 WHERE subject_id = ?1
                 ORDER BY sequence DESC
                 LIMIT ?2",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_subject_events",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![subject_id, limit as i64])
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_subject_events",
                message: error.to_string(),
            })?;
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.load_subject_events",
            message: error.to_string(),
        })? {
            let payload_json =
                row.get::<_, String>(5)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "durable_state.load_subject_events",
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
        events.reverse();
        Ok(events)
    }

    pub fn load_subject_events_by_kind(
        &self,
        subject_id: &str,
        kind: &str,
        limit: usize,
    ) -> Result<Vec<DurableStateEvent>, DaemonError> {
        let limit = limit.clamp(1, 200);
        let connection = self.lock_connection("durable_state.load_subject_events_by_kind")?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, event_id, kind, subject_id, timestamp_ms, payload_json
                 FROM durable_state_events
                 WHERE subject_id = ?1 AND kind = ?2
                 ORDER BY sequence DESC
                 LIMIT ?3",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_subject_events_by_kind",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![subject_id, kind, limit as i64])
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.load_subject_events_by_kind",
                message: error.to_string(),
            })?;
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.load_subject_events_by_kind",
            message: error.to_string(),
        })? {
            let payload_json =
                row.get::<_, String>(5)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "durable_state.load_subject_events_by_kind",
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
        events.reverse();
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

    pub(crate) fn event_tail_statistics(
        &self,
        after_sequence: u64,
    ) -> Result<DurableEventTailStatistics, DaemonError> {
        let connection = self.lock_connection("durable_state.event_tail_statistics")?;
        let (event_count, encoded_bytes, oldest_timestamp_ms, latest_sequence) = connection
            .query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(
                            length(CAST(event_id AS BLOB))
                          + length(CAST(kind AS BLOB))
                          + COALESCE(length(CAST(subject_id AS BLOB)), 0)
                          + length(CAST(payload_json AS BLOB))
                          + 24
                        ), 0),
                        MIN(timestamp_ms),
                        COALESCE(MAX(sequence), ?1)
                 FROM durable_state_events
                 WHERE sequence > ?1",
                params![after_sequence as i64],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.event_tail_statistics",
                message: error.to_string(),
            })?;
        Ok(DurableEventTailStatistics {
            event_count: event_count.max(0) as u64,
            encoded_bytes: encoded_bytes.max(0) as u64,
            oldest_timestamp_ms: oldest_timestamp_ms.map(|value| value.max(0) as u64),
            latest_sequence: latest_sequence.max(0) as u64,
        })
    }

    pub(crate) fn latest_checkpoint_marker_for_owner(
        &self,
        owner_id: &str,
    ) -> Result<DurableCheckpointMarker, DaemonError> {
        let connection = self.lock_connection("durable_state.latest_checkpoint_marker")?;
        let marker = connection
            .query_row(
                "SELECT sequence, timestamp_ms
                 FROM durable_state_owner_checkpoint_manifest
                 WHERE owner_id = ?1",
                params![owner_id],
                |row| {
                    Ok(DurableCheckpointMarker {
                        sequence: row.get::<_, i64>(0)?.max(0) as u64,
                        timestamp_ms: row.get::<_, i64>(1)?.max(0) as u64,
                    })
                },
            )
            .optional()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.latest_checkpoint_marker",
                message: error.to_string(),
            })?;
        drop(connection);
        if let Some(marker) = marker {
            return Ok(marker);
        }
        Ok(self
            .latest_snapshot_for_owner(owner_id)?
            .map(|snapshot| DurableCheckpointMarker {
                sequence: snapshot.sequence,
                timestamp_ms: snapshot.timestamp_ms,
            })
            .unwrap_or_default())
    }

    pub fn latest_snapshot_sequence(&self) -> Result<u64, DaemonError> {
        let connection = self.lock_connection("durable_state.latest_snapshot_sequence")?;
        let sequence = connection
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM (
                    SELECT sequence FROM durable_state_snapshots
                    UNION ALL
                    SELECT sequence FROM durable_state_checkpoint_manifest
                    UNION ALL
                    SELECT sequence FROM durable_state_owner_checkpoint_manifest
                 )",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.latest_snapshot_sequence",
                message: error.to_string(),
            })?;
        Ok(sequence.max(0) as u64)
    }

    pub fn latest_snapshot_sequence_for_owner(&self, owner_id: &str) -> Result<u64, DaemonError> {
        Ok(self
            .latest_snapshot_for_owner(owner_id)?
            .map(|snapshot| snapshot.sequence)
            .unwrap_or_default())
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
        self.writer.execute(DurableWriteOperation::Snapshot {
            sequence,
            timestamp_ms,
            payload_json,
        })?;
        Ok(DurableStateSnapshot {
            sequence,
            timestamp_ms,
            payload,
        })
    }

    pub fn latest_snapshot(&self) -> Result<Option<DurableStateSnapshot>, DaemonError> {
        Ok([
            self.latest_entity_checkpoint(None)?,
            self.latest_unscoped_entity_checkpoint()?,
            self.latest_legacy_snapshot()?,
        ]
        .into_iter()
        .flatten()
        .max_by_key(|snapshot| (snapshot.sequence, snapshot.timestamp_ms)))
    }

    fn latest_legacy_snapshot(&self) -> Result<Option<DurableStateSnapshot>, DaemonError> {
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
        let legacy = DurableStateSnapshot {
            sequence: sequence.max(0) as u64,
            timestamp_ms: timestamp_ms.max(0) as u64,
            payload: serde_json::from_str(&payload_json).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "durable_state.decode_snapshot",
                    message: error.to_string(),
                }
            })?,
        };
        Ok(Some(legacy))
    }

    pub(crate) fn save_entity_checkpoint(
        &self,
        owner_id: &str,
        sequence: u64,
        entities: Vec<DurableCheckpointEntity>,
    ) -> Result<DurableStateSnapshot, DaemonError> {
        let timestamp_ms = unix_epoch_ms();
        self.writer
            .execute(DurableWriteOperation::EntityCheckpoint {
                owner_id: owner_id.to_string(),
                sequence,
                timestamp_ms,
                entities,
            })?;
        self.latest_entity_checkpoint(Some(owner_id))?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "durable_state.load_saved_entity_checkpoint",
                message: "entity checkpoint manifest was not visible after commit".to_string(),
            })
    }

    pub(crate) fn latest_snapshot_for_owner(
        &self,
        owner_id: &str,
    ) -> Result<Option<DurableStateSnapshot>, DaemonError> {
        let legacy_checkpoint = self
            .latest_unscoped_entity_checkpoint()?
            .filter(|snapshot| snapshot_payload_has_owner(&snapshot.payload, owner_id));
        Ok([
            self.latest_entity_checkpoint(Some(owner_id))?,
            legacy_checkpoint,
            self.latest_legacy_snapshot_for_owner(owner_id)?,
        ]
        .into_iter()
        .flatten()
        .max_by_key(|snapshot| (snapshot.sequence, snapshot.timestamp_ms)))
    }

    fn latest_unscoped_entity_checkpoint(
        &self,
    ) -> Result<Option<DurableStateSnapshot>, DaemonError> {
        let connection = self.lock_connection("durable_state.latest_legacy_entity_checkpoint")?;
        let manifest = connection.query_row(
            "SELECT sequence, timestamp_ms FROM durable_state_checkpoint_manifest
             ORDER BY sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        );
        let (sequence, timestamp_ms) = match manifest {
            Ok(manifest) => manifest,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(error) => {
                return Err(DaemonError::LocalTransport {
                    operation: "durable_state.latest_legacy_entity_checkpoint",
                    message: error.to_string(),
                })
            }
        };
        let mut statement = connection
            .prepare(
                "SELECT entity_kind, payload_json
                 FROM durable_state_checkpoint_entities
                 ORDER BY entity_kind ASC, entity_id ASC",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.prepare_legacy_entity_checkpoint",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query([])
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.query_legacy_entity_checkpoint",
                message: error.to_string(),
            })?;
        let mut payload = empty_checkpoint_payload();
        while let Some(row) = rows.next().map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.read_legacy_entity_checkpoint",
            message: error.to_string(),
        })? {
            push_checkpoint_entity(&mut payload, row, "legacy")?;
        }
        Ok(Some(DurableStateSnapshot {
            sequence: sequence.max(0) as u64,
            timestamp_ms: timestamp_ms.max(0) as u64,
            payload: serde_json::Value::Object(payload),
        }))
    }

    fn latest_entity_checkpoint(
        &self,
        owner_id: Option<&str>,
    ) -> Result<Option<DurableStateSnapshot>, DaemonError> {
        let connection = self.lock_connection("durable_state.latest_entity_checkpoint")?;
        let manifest = match owner_id {
            Some(owner_id) => connection.query_row(
                "SELECT owner_id, sequence, timestamp_ms
                 FROM durable_state_owner_checkpoint_manifest
                 WHERE owner_id = ?1",
                params![owner_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            ),
            None => connection.query_row(
                "SELECT owner_id, sequence, timestamp_ms
                 FROM durable_state_owner_checkpoint_manifest
                 ORDER BY sequence DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            ),
        };
        let (checkpoint_owner_id, sequence, timestamp_ms) = match manifest {
            Ok(manifest) => manifest,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(error) => {
                return Err(DaemonError::LocalTransport {
                    operation: "durable_state.latest_entity_checkpoint",
                    message: error.to_string(),
                })
            }
        };
        let mut statement = connection
            .prepare(
                "SELECT entity_kind, payload_json
                 FROM durable_state_owner_checkpoint_entities
                 WHERE owner_id = ?1
                 ORDER BY entity_kind ASC, entity_id ASC",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.prepare_entity_checkpoint",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query(params![checkpoint_owner_id])
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.query_entity_checkpoint",
                message: error.to_string(),
            })?;
        let mut payload = [
            "sessions",
            "prompt_private_states",
            "agents",
            "slices",
            "slice_saved_states",
            "slice_backups",
            "metaagent_event_records",
            "metaagent_event_subscriptions",
        ]
        .into_iter()
        .map(|kind| (kind.to_string(), serde_json::Value::Array(Vec::new())))
        .collect::<serde_json::Map<_, _>>();
        while let Some(row) = rows.next().map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.read_entity_checkpoint",
            message: error.to_string(),
        })? {
            let kind = row
                .get::<_, String>(0)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "durable_state.decode_entity_checkpoint_kind",
                    message: error.to_string(),
                })?;
            let encoded = row
                .get::<_, String>(1)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "durable_state.decode_entity_checkpoint_payload",
                    message: error.to_string(),
                })?;
            let value =
                serde_json::from_str(&encoded).map_err(|error| DaemonError::LocalTransport {
                    operation: "durable_state.decode_entity_checkpoint_json",
                    message: error.to_string(),
                })?;
            payload
                .entry(kind)
                .or_insert_with(|| serde_json::Value::Array(Vec::new()))
                .as_array_mut()
                .expect("checkpoint entity group should be an array")
                .push(value);
        }
        Ok(Some(DurableStateSnapshot {
            sequence: sequence.max(0) as u64,
            timestamp_ms: timestamp_ms.max(0) as u64,
            payload: serde_json::Value::Object(payload),
        }))
    }

    fn latest_legacy_snapshot_for_owner(
        &self,
        owner_id: &str,
    ) -> Result<Option<DurableStateSnapshot>, DaemonError> {
        let connection = self.lock_connection("durable_state.legacy_snapshots")?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, timestamp_ms, payload_json
                 FROM durable_state_snapshots
                 ORDER BY sequence DESC, snapshot_id DESC",
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.legacy_snapshots",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query([])
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.legacy_snapshots",
                message: error.to_string(),
            })?;
        while let Some(row) = rows.next().map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.legacy_snapshots",
            message: error.to_string(),
        })? {
            let payload_json =
                row.get::<_, String>(2)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "durable_state.legacy_snapshots",
                        message: error.to_string(),
                    })?;
            let snapshot = DurableStateSnapshot {
                sequence: row.get::<_, i64>(0).unwrap_or_default().max(0) as u64,
                timestamp_ms: row.get::<_, i64>(1).unwrap_or_default().max(0) as u64,
                payload: serde_json::from_str(&payload_json).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "durable_state.decode_snapshot",
                        message: error.to_string(),
                    }
                })?,
            };
            if snapshot_payload_has_owner(&snapshot.payload, owner_id) {
                return Ok(Some(snapshot));
            }
        }
        Ok(None)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn reclaim_unused_pages_incrementally(
        &self,
        max_pages: u32,
    ) -> Result<DurableIncrementalReclaimOutcome, DaemonError> {
        let connection =
            Connection::open(&self.path).map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.open_incremental_reclaim",
                message: error.to_string(),
            })?;
        connection
            .busy_timeout(Duration::from_millis(250))
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.configure_incremental_reclaim",
                message: error.to_string(),
            })?;
        let auto_vacuum = connection
            .query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, i64>(0))
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.read_incremental_reclaim_mode",
                message: error.to_string(),
            })?;
        let free_pages_before = connection
            .query_row("PRAGMA freelist_count", [], |row| row.get::<_, i64>(0))
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.read_free_pages",
                message: error.to_string(),
            })?
            .max(0) as u64;
        if auto_vacuum != 2 || free_pages_before == 0 || max_pages == 0 {
            return Ok(DurableIncrementalReclaimOutcome {
                supported: auto_vacuum == 2,
                free_pages_before,
                free_pages_after: free_pages_before,
            });
        }
        connection
            .execute_batch(&format!("PRAGMA incremental_vacuum({max_pages});"))
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.incremental_reclaim",
                message: error.to_string(),
            })?;
        let free_pages_after = connection
            .query_row("PRAGMA freelist_count", [], |row| row.get::<_, i64>(0))
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.read_reclaimed_free_pages",
                message: error.to_string(),
            })?
            .max(0) as u64;
        Ok(DurableIncrementalReclaimOutcome {
            supported: true,
            free_pages_before,
            free_pages_after,
        })
    }

    pub(crate) fn writer_health_snapshot(&self) -> DurableWriterHealthSnapshot {
        self.writer.health_snapshot()
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

impl DurableStateWriter {
    fn start(path: &Path) -> Result<Self, DaemonError> {
        let connection = Connection::open(path).map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.open_writer",
            message: error.to_string(),
        })?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.writer_wal",
                message: error.to_string(),
            })?;
        let (sender, receiver) = mpsc::sync_channel(DURABLE_WRITE_QUEUE_CAPACITY);
        let health = Arc::new(DurableWriterHealth::default());
        let worker_health = Arc::clone(&health);
        let worker = std::thread::Builder::new()
            .name("chariox-durable-writer".to_string())
            .stack_size(512 * 1024)
            .spawn(move || run_durable_writer(connection, receiver, worker_health))
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.spawn_writer",
                message: error.to_string(),
            })?;
        Ok(Self {
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(Some(worker)),
            health,
        })
    }

    fn execute(&self, operation: DurableWriteOperation) -> Result<u64, DaemonError> {
        let (response_tx, response_rx) = mpsc::channel();
        let sender = self
            .sender
            .lock()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.lock_writer",
                message: error.to_string(),
            })?
            .as_ref()
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "durable_state.enqueue_write",
                message: "durable writer is shutting down".to_string(),
            })?;
        sender
            .send(DurableWriteRequest {
                operation,
                response: response_tx,
            })
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.enqueue_write",
                message: error.to_string(),
            })?;
        response_rx
            .recv()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.await_write",
                message: error.to_string(),
            })?
            .map_err(|message| DaemonError::LocalTransport {
                operation: "durable_state.commit_write",
                message,
            })
    }

    fn health_snapshot(&self) -> DurableWriterHealthSnapshot {
        DurableWriterHealthSnapshot {
            committed_batches: self.health.committed_batches.load(Ordering::Acquire),
            committed_records: self.health.committed_records.load(Ordering::Acquire),
            max_batch_records: self.health.max_batch_records.load(Ordering::Acquire),
        }
    }
}

impl Drop for DurableStateWriter {
    fn drop(&mut self) {
        self.sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = worker.join();
        }
    }
}

fn run_durable_writer(
    mut connection: Connection,
    receiver: Receiver<DurableWriteRequest>,
    health: Arc<DurableWriterHealth>,
) {
    while let Ok(first) = receiver.recv() {
        let mut batch = vec![first];
        let deadline = Instant::now() + DURABLE_WRITE_BATCH_WINDOW;
        while batch.len() < DURABLE_WRITE_BATCH_LIMIT {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            match receiver.recv_timeout(remaining) {
                Ok(request) => batch.push(request),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        commit_durable_write_batch(&mut connection, batch, &health);
    }
}

fn commit_durable_write_batch(
    connection: &mut Connection,
    batch: Vec<DurableWriteRequest>,
    health: &DurableWriterHealth,
) {
    let transaction = match connection.transaction() {
        Ok(transaction) => transaction,
        Err(error) => {
            send_durable_batch_error(batch, error.to_string());
            return;
        }
    };
    let mut results = Vec::with_capacity(batch.len());
    let mut failure = None;
    for request in &batch {
        let result = match &request.operation {
            DurableWriteOperation::Event {
                event_id,
                kind,
                subject_id,
                timestamp_ms,
                payload_json,
            } => transaction
                .execute(
                    "INSERT INTO durable_state_events (
                        event_id, kind, subject_id, timestamp_ms, payload_json
                    ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        event_id,
                        kind,
                        subject_id,
                        *timestamp_ms as i64,
                        payload_json
                    ],
                )
                .map(|_| transaction.last_insert_rowid().max(0) as u64),
            DurableWriteOperation::Snapshot {
                sequence,
                timestamp_ms,
                payload_json,
            } => transaction
                .execute(
                    "INSERT INTO durable_state_snapshots (sequence, timestamp_ms, payload_json)
                     VALUES (?1, ?2, ?3)",
                    params![*sequence as i64, *timestamp_ms as i64, payload_json],
                )
                .map(|_| *sequence),
            DurableWriteOperation::EntityCheckpoint {
                owner_id,
                sequence,
                timestamp_ms,
                entities,
            } => {
                write_entity_checkpoint(&transaction, owner_id, *sequence, *timestamp_ms, entities)
                    .map(|_| *sequence)
            }
            DurableWriteOperation::WorkflowRuntimeTransition {
                event_id,
                timestamp_ms,
                payload_json,
                owner_id,
                session_id,
                hot_entities,
                workflow_runs,
                delivery_receipts,
            } => workflow_runtime::write_workflow_runtime_transition(
                &transaction,
                workflow_runtime::WorkflowRuntimeTransitionWrite {
                    event_id,
                    timestamp_ms: *timestamp_ms,
                    payload_json,
                    owner_id,
                    session_id,
                    hot_entities,
                    workflow_runs,
                    delivery_receipts,
                },
            ),
            DurableWriteOperation::WorkflowRuntimeSessionsTransition {
                event_id,
                timestamp_ms,
                payload_json,
                owner_id,
                sessions,
            } => workflow_runtime::write_workflow_runtime_sessions_transition(
                &transaction,
                event_id,
                *timestamp_ms,
                payload_json,
                owner_id,
                sessions,
            ),
            DurableWriteOperation::WorkflowRuntimeMigration {
                owner_id,
                hot_entities,
                workflow_runs,
                delivery_receipts,
            } => workflow_runtime::write_workflow_runtime_migration(
                &transaction,
                owner_id,
                hot_entities,
                workflow_runs,
                delivery_receipts,
            ),
            DurableWriteOperation::WorkflowHistoryMigrationChunk {
                owner_id,
                workflow_runs,
                completed,
            } => workflow_runtime::write_workflow_history_migration_chunk(
                &transaction,
                owner_id,
                workflow_runs,
                *completed,
            ),
            DurableWriteOperation::SessionDelete {
                event_id,
                timestamp_ms,
                payload_json,
                owner_id,
                session_id,
            } => workflow_runtime::write_session_delete(
                &transaction,
                event_id,
                *timestamp_ms,
                payload_json,
                owner_id,
                session_id,
            ),
        };
        match result {
            Ok(sequence) => results.push(sequence),
            Err(error) => {
                failure = Some(error.to_string());
                break;
            }
        }
    }
    if let Some(message) = failure {
        drop(transaction);
        send_durable_batch_error(batch, message);
        return;
    }
    if let Err(error) = transaction.commit() {
        send_durable_batch_error(batch, error.to_string());
        return;
    }
    let batch_len = batch.len() as u64;
    health.committed_batches.fetch_add(1, Ordering::AcqRel);
    health
        .committed_records
        .fetch_add(batch_len, Ordering::AcqRel);
    health
        .max_batch_records
        .fetch_max(batch_len, Ordering::AcqRel);
    for (request, sequence) in batch.into_iter().zip(results) {
        let _ = request.response.send(Ok(sequence));
    }
}

fn send_durable_batch_error(batch: Vec<DurableWriteRequest>, message: String) {
    for request in batch {
        let _ = request.response.send(Err(message.clone()));
    }
}

fn write_entity_checkpoint(
    transaction: &rusqlite::Transaction<'_>,
    owner_id: &str,
    sequence: u64,
    timestamp_ms: u64,
    entities: &[DurableCheckpointEntity],
) -> Result<(), rusqlite::Error> {
    transaction.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS durable_checkpoint_current_keys (
            entity_kind TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            PRIMARY KEY(entity_kind, entity_id)
         );
         DELETE FROM durable_checkpoint_current_keys;",
    )?;
    {
        let mut key_statement = transaction.prepare(
            "INSERT INTO durable_checkpoint_current_keys (entity_kind, entity_id) VALUES (?1, ?2)",
        )?;
        let mut entity_statement = transaction.prepare(
            "INSERT INTO durable_state_owner_checkpoint_entities (
                owner_id, entity_kind, entity_id, checkpoint_sequence, payload_json
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(owner_id, entity_kind, entity_id) DO UPDATE SET
                checkpoint_sequence = excluded.checkpoint_sequence,
                payload_json = excluded.payload_json
             WHERE durable_state_owner_checkpoint_entities.payload_json <> excluded.payload_json",
        )?;
        for entity in entities {
            key_statement.execute(params![entity.kind, entity.id])?;
            entity_statement.execute(params![
                owner_id,
                entity.kind,
                entity.id,
                sequence as i64,
                entity.payload_json,
            ])?;
        }
    }
    transaction.execute(
        "DELETE FROM durable_state_owner_checkpoint_entities
         WHERE owner_id = ?1 AND NOT EXISTS (
            SELECT 1 FROM durable_checkpoint_current_keys current
            WHERE current.entity_kind = durable_state_owner_checkpoint_entities.entity_kind
              AND current.entity_id = durable_state_owner_checkpoint_entities.entity_id
         )",
        params![owner_id],
    )?;
    transaction.execute(
        "INSERT INTO durable_state_owner_checkpoint_manifest (owner_id, sequence, timestamp_ms)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(owner_id) DO UPDATE SET
            sequence = excluded.sequence,
            timestamp_ms = excluded.timestamp_ms",
        params![owner_id, sequence as i64, timestamp_ms as i64],
    )?;
    transaction.execute(
        "DELETE FROM durable_state_events
         WHERE sequence <= ?1
           AND kind IN (
               'session.updated',
               'workflow.runtime.updated',
               'session.prompt_state.updated'
           )
           AND (
               (
                   kind = 'workflow.runtime.updated'
                   AND json_extract(payload_json, '$.owner_id') = ?2
               )
               OR EXISTS (
                   SELECT 1 FROM durable_checkpoint_current_keys current
                   WHERE current.entity_kind = 'sessions'
                     AND current.entity_id = durable_state_events.subject_id
               )
           )
           AND EXISTS (
               SELECT 1 FROM durable_state_metadata migration
               WHERE migration.owner_id = ?2
                 AND migration.metadata_key = 'workflow_history_migration_status'
                 AND migration.metadata_value = 'verified'
           )",
        params![sequence as i64, owner_id],
    )?;
    transaction.execute(
        "DELETE FROM durable_state_snapshots
         WHERE sequence <= ?1
           AND (
               EXISTS (
                   SELECT 1 FROM json_each(payload_json, '$.sessions') session
                   WHERE json_extract(session.value, '$.host_daemon_id') = ?2
               )
               OR EXISTS (
                   SELECT 1 FROM json_each(payload_json, '$.slices') slice
                   WHERE json_extract(slice.value, '$.owner_kernel_id') = ?2
               )
           )
           AND EXISTS (
               SELECT 1 FROM durable_state_metadata migration
               WHERE migration.owner_id = ?2
                 AND migration.metadata_key = 'workflow_history_migration_status'
                 AND migration.metadata_value = 'verified'
           )",
        params![sequence as i64, owner_id],
    )?;
    Ok(())
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

CREATE TABLE IF NOT EXISTS durable_state_checkpoint_manifest (
    sequence INTEGER PRIMARY KEY,
    timestamp_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS durable_state_checkpoint_entities (
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    checkpoint_sequence INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY(entity_kind, entity_id)
);

CREATE INDEX IF NOT EXISTS idx_durable_checkpoint_entities_sequence
    ON durable_state_checkpoint_entities(checkpoint_sequence, entity_kind);

CREATE TABLE IF NOT EXISTS durable_state_owner_checkpoint_manifest (
    owner_id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL,
    timestamp_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS durable_state_owner_checkpoint_entities (
    owner_id TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    checkpoint_sequence INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY(owner_id, entity_kind, entity_id)
);

CREATE INDEX IF NOT EXISTS idx_durable_owner_checkpoint_entities_sequence
    ON durable_state_owner_checkpoint_entities(owner_id, checkpoint_sequence, entity_kind);

CREATE TABLE IF NOT EXISTS durable_workflow_hot_entities (
    owner_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY(owner_id, session_id, entity_kind, entity_id)
);

CREATE INDEX IF NOT EXISTS idx_durable_workflow_hot_entities_session
    ON durable_workflow_hot_entities(owner_id, session_id, entity_kind, entity_id);

CREATE TABLE IF NOT EXISTS durable_workflow_runs (
    owner_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    workflow_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY(owner_id, session_id, run_id)
);

CREATE INDEX IF NOT EXISTS idx_durable_workflow_runs_session_created
    ON durable_workflow_runs(owner_id, session_id, created_at_ms DESC, run_id DESC);

CREATE INDEX IF NOT EXISTS idx_durable_workflow_runs_workflow_created
    ON durable_workflow_runs(owner_id, session_id, workflow_id, created_at_ms DESC, run_id DESC);

CREATE INDEX IF NOT EXISTS idx_durable_workflow_runs_status
    ON durable_workflow_runs(owner_id, status, session_id);

CREATE INDEX IF NOT EXISTS idx_durable_workflow_runs_active
    ON durable_workflow_runs(owner_id, session_id, created_at_ms, run_id)
    WHERE status NOT IN ('Completed', 'Failed', 'Stopped');

CREATE TABLE IF NOT EXISTS durable_event_delivery_receipts (
    owner_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    delivery_id TEXT NOT NULL,
    binding_id TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY(owner_id, session_id, delivery_id)
);

CREATE INDEX IF NOT EXISTS idx_durable_delivery_receipts_expiry
    ON durable_event_delivery_receipts(owner_id, expires_at_ms);

CREATE TABLE IF NOT EXISTS durable_state_metadata (
    owner_id TEXT NOT NULL,
    metadata_key TEXT NOT NULL,
    metadata_value TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(owner_id, metadata_key)
);
"#;

fn snapshot_payload_has_owner(payload: &serde_json::Value, owner_id: &str) -> bool {
    [
        ("sessions", "host_daemon_id"),
        ("slices", "owner_kernel_id"),
    ]
    .into_iter()
    .any(|(collection, field)| {
        payload
            .get(collection)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .any(|entity| entity.get(field).and_then(serde_json::Value::as_str) == Some(owner_id))
    })
}

fn empty_checkpoint_payload() -> serde_json::Map<String, serde_json::Value> {
    [
        "sessions",
        "prompt_private_states",
        "agents",
        "slices",
        "slice_saved_states",
        "slice_backups",
        "metaagent_event_records",
        "metaagent_event_subscriptions",
    ]
    .into_iter()
    .map(|kind| (kind.to_string(), serde_json::Value::Array(Vec::new())))
    .collect()
}

fn push_checkpoint_entity(
    payload: &mut serde_json::Map<String, serde_json::Value>,
    row: &rusqlite::Row<'_>,
    _source: &'static str,
) -> Result<(), DaemonError> {
    let kind = row
        .get::<_, String>(0)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.decode_entity_checkpoint_kind",
            message: error.to_string(),
        })?;
    let encoded = row
        .get::<_, String>(1)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.decode_entity_checkpoint_payload",
            message: error.to_string(),
        })?;
    let value = serde_json::from_str(&encoded).map_err(|error| DaemonError::LocalTransport {
        operation: "durable_state.decode_entity_checkpoint_json",
        message: error.to_string(),
    })?;
    payload
        .entry(kind)
        .or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array_mut()
        .expect("checkpoint entity group should be an array")
        .push(value);
    Ok(())
}

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
    fn durable_writer_groups_concurrent_acknowledged_events() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-batch-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        let barrier = Arc::new(std::sync::Barrier::new(33));
        let handles = (0..32)
            .map(|index| {
                let store = store.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    store
                        .append_event(
                            "agent.updated",
                            Some(format!("agent-{index}")),
                            serde_json::json!({"index": index}),
                        )
                        .expect("batched event should commit")
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let mut sequences = handles
            .into_iter()
            .map(|handle| handle.join().expect("append thread should join").sequence)
            .collect::<Vec<_>>();
        sequences.sort_unstable();
        sequences.dedup();
        assert_eq!(sequences.len(), 32);
        let health = store.writer_health_snapshot();
        assert_eq!(health.committed_records, 32);
        assert!(health.committed_batches < health.committed_records);
        assert!(health.max_batch_records > 1);

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn durable_writer_does_not_wait_for_read_connection_mutex() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-read-isolation-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        let read_guard = store
            .connection
            .lock()
            .expect("read connection should lock");
        store
            .append_event("session.updated", None, serde_json::json!({"ok": true}))
            .expect("writer should commit while reader mutex is held");
        drop(read_guard);

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn new_store_enables_bounded_incremental_reclamation() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-incremental-reclaim-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        let outcome = store
            .reclaim_unused_pages_incrementally(32)
            .expect("incremental reclaim should be safe");
        assert!(outcome.supported);
        assert_eq!(outcome.free_pages_before, outcome.free_pages_after);

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn entity_checkpoints_are_incremental_atomic_and_supersede_legacy_snapshots() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-entity-checkpoint-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        store
            .save_snapshot(
                1,
                serde_json::json!({
                    "sessions": [{"id": "legacy", "host_daemon_id": "owner-1"}]
                }),
            )
            .expect("legacy snapshot should save");
        store
            .migrate_legacy_workflow_history_chunk("owner-1", &[], true)
            .expect("history migration should be verified before legacy compaction");
        let session = DurableCheckpointEntity {
            kind: "sessions".to_string(),
            id: "session-1".to_string(),
            payload_json: serde_json::json!({"id": "session-1"}).to_string(),
        };
        store
            .save_entity_checkpoint("owner-1", 2, vec![session.clone()])
            .expect("entity checkpoint should save");
        store
            .save_entity_checkpoint(
                "owner-1",
                3,
                vec![
                    session,
                    DurableCheckpointEntity {
                        kind: "agents".to_string(),
                        id: "agent-1".to_string(),
                        payload_json: serde_json::json!({"id": "agent-1"}).to_string(),
                    },
                ],
            )
            .expect("incremental entity checkpoint should save");
        let snapshot = store
            .latest_snapshot()
            .expect("checkpoint should load")
            .expect("checkpoint should exist");
        assert_eq!(snapshot.sequence, 3);
        assert_eq!(snapshot.payload["sessions"][0]["id"], "session-1");
        assert_eq!(snapshot.payload["agents"][0]["id"], "agent-1");
        assert_eq!(
            store
                .latest_snapshot_sequence()
                .expect("sequence should load"),
            3
        );
        let connection = store
            .connection
            .lock()
            .expect("read connection should lock");
        let session_changed_at = connection
            .query_row(
                "SELECT checkpoint_sequence FROM durable_state_owner_checkpoint_entities
                 WHERE owner_id = 'owner-1'
                   AND entity_kind = 'sessions' AND entity_id = 'session-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("session checkpoint sequence should load");
        let legacy_snapshot_count = connection
            .query_row("SELECT COUNT(*) FROM durable_state_snapshots", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("legacy snapshot count should load");
        assert_eq!(
            session_changed_at, 2,
            "unchanged entities must not be rewritten"
        );
        assert_eq!(
            legacy_snapshot_count, 0,
            "writer-thread compaction should remove superseded snapshots"
        );
        drop(connection);

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn checkpoint_preserves_legacy_replay_until_history_migration_is_verified() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-migration-gate-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        let sequence = store
            .append_event(
                "session.updated",
                Some("session-1".to_string()),
                serde_json::json!({"id": "session-1", "host_daemon_id": "owner-1"}),
            )
            .expect("legacy session event should append")
            .sequence;
        let entities = vec![DurableCheckpointEntity {
            kind: "sessions".to_string(),
            id: "session-1".to_string(),
            payload_json: serde_json::json!({"id": "session-1"}).to_string(),
        }];

        store
            .save_entity_checkpoint("owner-1", sequence, entities.clone())
            .expect("checkpoint should save");
        assert_eq!(
            store
                .load_events_by_kind("session.updated")
                .expect("legacy event should load")
                .len(),
            1,
            "rollback source must remain before migration verification",
        );

        store
            .migrate_legacy_workflow_history_chunk("owner-1", &[], true)
            .expect("empty history migration should verify");
        store
            .save_entity_checkpoint("owner-1", sequence, entities)
            .expect("verified checkpoint should save");
        assert!(store
            .load_events_by_kind("session.updated")
            .expect("compacted events should load")
            .is_empty());

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn entity_checkpoints_are_isolated_by_kernel_owner() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-owner-checkpoint-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        let owner_checkpoint = |owner_id: &str, session_id: &str| DurableCheckpointEntity {
            kind: "sessions".to_string(),
            id: session_id.to_string(),
            payload_json: serde_json::json!({
                "id": session_id,
                "host_daemon_id": owner_id,
            })
            .to_string(),
        };

        store
            .append_event(
                "session.updated",
                Some("session-a".to_string()),
                serde_json::json!({"session": {"id": "session-a"}}),
            )
            .expect("first owner update should append");
        store
            .append_event(
                "session.updated",
                Some("session-b".to_string()),
                serde_json::json!({"session": {"id": "session-b"}}),
            )
            .expect("second owner update should append");
        store
            .migrate_legacy_workflow_history_chunk("kernel-a", &[], true)
            .expect("first owner history migration should verify");
        store
            .migrate_legacy_workflow_history_chunk("kernel-b", &[], true)
            .expect("second owner history migration should verify");

        store
            .save_entity_checkpoint(
                "kernel-a",
                10,
                vec![owner_checkpoint("kernel-a", "session-a")],
            )
            .expect("first owner checkpoint should save");
        assert_eq!(
            store
                .load_subject_events_by_kind("session-b", "session.updated", 10)
                .expect("other owner update should remain")
                .len(),
            1
        );
        store
            .save_entity_checkpoint(
                "kernel-b",
                20,
                vec![owner_checkpoint("kernel-b", "session-b")],
            )
            .expect("second owner checkpoint should save");

        let first = store
            .latest_snapshot_for_owner("kernel-a")
            .expect("first owner checkpoint should load")
            .expect("first owner checkpoint should exist");
        let second = store
            .latest_snapshot_for_owner("kernel-b")
            .expect("second owner checkpoint should load")
            .expect("second owner checkpoint should exist");
        assert_eq!(first.sequence, 10);
        assert_eq!(first.payload["sessions"][0]["id"], "session-a");
        assert_eq!(second.sequence, 20);
        assert_eq!(second.payload["sessions"][0]["id"], "session-b");
        assert!(store
            .load_subject_events_by_kind("session-a", "session.updated", 10)
            .expect("first owner updates should load")
            .is_empty());
        assert!(store
            .load_subject_events_by_kind("session-b", "session.updated", 10)
            .expect("second owner updates should load")
            .is_empty());

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn owner_restore_reuses_compatible_unscoped_checkpoint_during_migration() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-legacy-checkpoint-migration-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        {
            let connection = Connection::open(&path).expect("migration fixture should open");
            connection
                .execute(
                    "INSERT INTO durable_state_checkpoint_manifest (sequence, timestamp_ms)
                     VALUES (42, 1000)",
                    [],
                )
                .expect("legacy manifest should insert");
            connection
                .execute(
                    "INSERT INTO durable_state_checkpoint_entities (
                        entity_kind, entity_id, checkpoint_sequence, payload_json
                     ) VALUES ('sessions', 'session-1', 42, ?1)",
                    params![serde_json::json!({
                        "id": "session-1",
                        "host_daemon_id": "kernel-a",
                    })
                    .to_string()],
                )
                .expect("legacy entity should insert");
        }

        let compatible = store
            .latest_snapshot_for_owner("kernel-a")
            .expect("compatible checkpoint should load")
            .expect("compatible checkpoint should exist");
        assert_eq!(compatible.sequence, 42);
        assert_eq!(compatible.payload["sessions"][0]["id"], "session-1");
        assert!(store
            .latest_snapshot_for_owner("kernel-b")
            .expect("foreign lookup should succeed")
            .is_none());

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn owner_restore_uses_compatible_snapshot_when_foreign_checkpoint_is_newer() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-compatible-snapshot-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        store
            .save_snapshot(
                10,
                serde_json::json!({
                    "sessions": [{"id": "session-a", "host_daemon_id": "kernel-a"}],
                    "agents": [],
                }),
            )
            .expect("compatible legacy snapshot should save");
        store
            .save_entity_checkpoint(
                "kernel-b",
                20,
                vec![DurableCheckpointEntity {
                    kind: "sessions".to_string(),
                    id: "session-b".to_string(),
                    payload_json: serde_json::json!({
                        "id": "session-b",
                        "host_daemon_id": "kernel-b",
                    })
                    .to_string(),
                }],
            )
            .expect("foreign checkpoint should save");

        let snapshot = store
            .latest_snapshot_for_owner("kernel-a")
            .expect("compatible snapshot lookup should succeed")
            .expect("compatible snapshot should exist");
        assert_eq!(snapshot.sequence, 10);
        assert_eq!(snapshot.payload["sessions"][0]["id"], "session-a");

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn newer_legacy_snapshot_supersedes_older_entity_checkpoint() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-newer-legacy-snapshot-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        store
            .save_entity_checkpoint(
                "owner-1",
                1,
                vec![DurableCheckpointEntity {
                    kind: "sessions".to_string(),
                    id: "foreign-session".to_string(),
                    payload_json: serde_json::json!({"id": "foreign-session"}).to_string(),
                }],
            )
            .expect("older checkpoint should save");
        store
            .save_snapshot(
                2,
                serde_json::json!({"sessions": [{"id": "current-session"}]}),
            )
            .expect("newer snapshot should save");

        let latest = store
            .latest_snapshot()
            .expect("latest snapshot should load")
            .expect("snapshot should exist");
        assert_eq!(latest.sequence, 2);
        assert_eq!(latest.payload["sessions"][0]["id"], "current-session");

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn entity_checkpoint_recovers_before_during_and_after_transaction_commit() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-checkpoint-crash-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        let before = DurableCheckpointEntity {
            kind: "sessions".to_string(),
            id: "session-1".to_string(),
            payload_json: serde_json::json!({"id": "session-1", "revision": 1}).to_string(),
        };
        store
            .save_entity_checkpoint("owner-1", 1, vec![before])
            .expect("checkpoint before crash should commit");

        {
            let mut connection =
                rusqlite::Connection::open(&path).expect("crash-simulation connection should open");
            let transaction = connection.transaction().expect("transaction should begin");
            transaction
                .execute(
                    "UPDATE durable_state_owner_checkpoint_entities
                     SET checkpoint_sequence = 2, payload_json = ?1
                     WHERE owner_id = 'owner-1'
                       AND entity_kind = 'sessions' AND entity_id = 'session-1'",
                    [serde_json::json!({"id": "session-1", "revision": 2}).to_string()],
                )
                .expect("in-flight entity update should apply");
            transaction
                .execute(
                    "UPDATE durable_state_owner_checkpoint_manifest
                     SET sequence = 2, timestamp_ms = ?1 WHERE owner_id = 'owner-1'",
                    [unix_epoch_ms() as i64],
                )
                .expect("in-flight manifest should apply");
            // Dropping an uncommitted transaction models process loss during the batch.
        }
        let recovered = store
            .latest_snapshot()
            .expect("checkpoint should load after rollback")
            .expect("checkpoint should exist");
        assert_eq!(recovered.sequence, 1);
        assert_eq!(recovered.payload["sessions"][0]["revision"], 1);

        store
            .save_entity_checkpoint(
                "owner-1",
                2,
                vec![DurableCheckpointEntity {
                    kind: "sessions".to_string(),
                    id: "session-1".to_string(),
                    payload_json: serde_json::json!({"id": "session-1", "revision": 2}).to_string(),
                }],
            )
            .expect("checkpoint after restart should commit");
        drop(store);

        let reopened = DurableKernelStateStore::open(path.clone()).expect("store should reopen");
        let after = reopened
            .latest_snapshot()
            .expect("committed checkpoint should load")
            .expect("committed checkpoint should exist");
        assert_eq!(after.sequence, 2);
        assert_eq!(after.payload["sessions"][0]["revision"], 2);
        drop(reopened);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn durable_state_store_appends_events_and_loads_latest_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-{}-{}.db",
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

    #[test]
    fn durable_state_store_loads_event_replay_in_bounded_batches() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-batched-replay-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        for index in 0..3 {
            store
                .append_event(
                    "session.updated",
                    Some(format!("session-{index}")),
                    serde_json::json!({"index": index}),
                )
                .expect("event should append");
        }

        let first_batch = store
            .load_events_after_batch(0, 2)
            .expect("first replay batch should load");
        assert_eq!(first_batch.len(), 2);
        let second_batch = store
            .load_events_after_batch(first_batch[1].sequence, 2)
            .expect("second replay batch should load");
        assert_eq!(second_batch.len(), 1);
        assert!(second_batch[0].sequence > first_batch[1].sequence);

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn durable_state_store_loads_only_events_of_requested_kind() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-event-kind-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");

        let first = store
            .append_event(
                "workspace_live_sync.change_recorded",
                Some("session-1".to_string()),
                serde_json::json!({"value": 1}),
            )
            .expect("first matching event should append");
        store
            .append_event(
                "session.updated",
                Some("session-1".to_string()),
                serde_json::json!({"large_unrelated_value": "x".repeat(1_000_000)}),
            )
            .expect("unrelated event should append");
        let second = store
            .append_event(
                "workspace_live_sync.change_recorded",
                Some("session-2".to_string()),
                serde_json::json!({"value": 2}),
            )
            .expect("second matching event should append");

        assert_eq!(
            store
                .load_events_by_kind("workspace_live_sync.change_recorded")
                .expect("matching events should load"),
            vec![first, second]
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn durable_state_store_loads_subject_events_by_kind() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-kind-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");

        store
            .append_event(
                "slice.updated",
                Some("slice-1".to_string()),
                serde_json::json!({"status": "starting"}),
            )
            .expect("state event should append");
        let audit = store
            .append_event(
                "slice.audit",
                Some("slice-1".to_string()),
                serde_json::json!({"action": "start"}),
            )
            .expect("audit event should append");
        store
            .append_event(
                "slice.audit",
                Some("slice-2".to_string()),
                serde_json::json!({"action": "other"}),
            )
            .expect("other subject audit event should append");

        let events = store
            .load_subject_events_by_kind("slice-1", "slice.audit", 10)
            .expect("audit events should load");

        assert_eq!(events, vec![audit]);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn restore_event_loading_skips_superseded_full_session_snapshots() {
        let path = std::env::temp_dir().join(format!(
            "chariox-durable-state-restore-compaction-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        store
            .append_event(
                "session.updated",
                Some("session-1".to_string()),
                serde_json::json!({"revision": 1, "payload": "x".repeat(1_000_000)}),
            )
            .expect("first session snapshot should append");
        let unrelated = store
            .append_event(
                "agent.updated",
                Some("agent-1".to_string()),
                serde_json::json!({"revision": 1}),
            )
            .expect("unrelated event should append");
        store
            .append_event(
                "workflow.runtime.updated",
                Some("session-1".to_string()),
                serde_json::json!({"revision": 2}),
            )
            .expect("latest session snapshot should append");

        let restored = store
            .load_restore_events_after_batch(0, 10)
            .expect("restore events should load");
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].kind, "session.updated");
        assert_eq!(restored[1], unrelated);

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
