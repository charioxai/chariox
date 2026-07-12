use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

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
        sequence: u64,
        timestamp_ms: u64,
        entities: Vec<DurableCheckpointEntity>,
    },
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

    pub fn latest_snapshot_sequence(&self) -> Result<u64, DaemonError> {
        let connection = self.lock_connection("durable_state.latest_snapshot_sequence")?;
        let sequence = connection
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM (
                    SELECT sequence FROM durable_state_snapshots
                    UNION ALL
                    SELECT sequence FROM durable_state_checkpoint_manifest
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
        if let Some(snapshot) = self.latest_entity_checkpoint()? {
            return Ok(Some(snapshot));
        }
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

    pub(crate) fn save_entity_checkpoint(
        &self,
        sequence: u64,
        entities: Vec<DurableCheckpointEntity>,
    ) -> Result<DurableStateSnapshot, DaemonError> {
        let timestamp_ms = unix_epoch_ms();
        self.writer
            .execute(DurableWriteOperation::EntityCheckpoint {
                sequence,
                timestamp_ms,
                entities,
            })?;
        self.latest_entity_checkpoint()?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "durable_state.load_saved_entity_checkpoint",
                message: "entity checkpoint manifest was not visible after commit".to_string(),
            })
    }

    fn latest_entity_checkpoint(&self) -> Result<Option<DurableStateSnapshot>, DaemonError> {
        let connection = self.lock_connection("durable_state.latest_entity_checkpoint")?;
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
                    operation: "durable_state.latest_entity_checkpoint",
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
                operation: "durable_state.prepare_entity_checkpoint",
                message: error.to_string(),
            })?;
        let mut rows = statement
            .query([])
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

    pub fn path(&self) -> &Path {
        &self.path
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
            .name("arroba-durable-writer".to_string())
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
                sequence,
                timestamp_ms,
                entities,
            } => write_entity_checkpoint(&transaction, *sequence, *timestamp_ms, entities)
                .map(|_| *sequence),
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
            "INSERT INTO durable_state_checkpoint_entities (
                entity_kind, entity_id, checkpoint_sequence, payload_json
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(entity_kind, entity_id) DO UPDATE SET
                checkpoint_sequence = excluded.checkpoint_sequence,
                payload_json = excluded.payload_json
             WHERE durable_state_checkpoint_entities.payload_json <> excluded.payload_json",
        )?;
        for entity in entities {
            key_statement.execute(params![entity.kind, entity.id])?;
            entity_statement.execute(params![
                entity.kind,
                entity.id,
                sequence as i64,
                entity.payload_json,
            ])?;
        }
    }
    transaction.execute(
        "DELETE FROM durable_state_checkpoint_entities
         WHERE NOT EXISTS (
            SELECT 1 FROM durable_checkpoint_current_keys current
            WHERE current.entity_kind = durable_state_checkpoint_entities.entity_kind
              AND current.entity_id = durable_state_checkpoint_entities.entity_id
         )",
        [],
    )?;
    transaction.execute(
        "INSERT INTO durable_state_checkpoint_manifest (sequence, timestamp_ms)
         VALUES (?1, ?2)
         ON CONFLICT(sequence) DO UPDATE SET timestamp_ms = excluded.timestamp_ms",
        params![sequence as i64, timestamp_ms as i64],
    )?;
    transaction.execute(
        "DELETE FROM durable_state_checkpoint_manifest WHERE sequence <> ?1",
        params![sequence as i64],
    )?;
    transaction.execute(
        "DELETE FROM durable_state_snapshots WHERE sequence <= ?1",
        params![sequence as i64],
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
    fn durable_writer_groups_concurrent_acknowledged_events() {
        let path = std::env::temp_dir().join(format!(
            "arroba-durable-state-batch-{}-{}.db",
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
            "arroba-durable-state-read-isolation-{}-{}.db",
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
    fn entity_checkpoints_are_incremental_atomic_and_supersede_legacy_snapshots() {
        let path = std::env::temp_dir().join(format!(
            "arroba-durable-state-entity-checkpoint-{}-{}.db",
            std::process::id(),
            unix_epoch_ms()
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("store should open");
        store
            .save_snapshot(1, serde_json::json!({"sessions": [{"id": "legacy"}]}))
            .expect("legacy snapshot should save");
        let session = DurableCheckpointEntity {
            kind: "sessions".to_string(),
            id: "session-1".to_string(),
            payload_json: serde_json::json!({"id": "session-1"}).to_string(),
        };
        store
            .save_entity_checkpoint(2, vec![session.clone()])
            .expect("entity checkpoint should save");
        store
            .save_entity_checkpoint(
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
                "SELECT checkpoint_sequence FROM durable_state_checkpoint_entities
                 WHERE entity_kind = 'sessions' AND entity_id = 'session-1'",
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
    fn entity_checkpoint_recovers_before_during_and_after_transaction_commit() {
        let path = std::env::temp_dir().join(format!(
            "arroba-durable-state-checkpoint-crash-{}-{}.db",
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
            .save_entity_checkpoint(1, vec![before])
            .expect("checkpoint before crash should commit");

        {
            let mut connection =
                rusqlite::Connection::open(&path).expect("crash-simulation connection should open");
            let transaction = connection.transaction().expect("transaction should begin");
            transaction
                .execute(
                    "UPDATE durable_state_checkpoint_entities
                     SET checkpoint_sequence = 2, payload_json = ?1
                     WHERE entity_kind = 'sessions' AND entity_id = 'session-1'",
                    [serde_json::json!({"id": "session-1", "revision": 2}).to_string()],
                )
                .expect("in-flight entity update should apply");
            transaction
                .execute(
                    "INSERT INTO durable_state_checkpoint_manifest (sequence, timestamp_ms)
                     VALUES (2, ?1)",
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

    #[test]
    fn durable_state_store_loads_only_events_of_requested_kind() {
        let path = std::env::temp_dir().join(format!(
            "arroba-durable-state-event-kind-{}-{}.db",
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
            "arroba-durable-state-kind-{}-{}.db",
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
}
