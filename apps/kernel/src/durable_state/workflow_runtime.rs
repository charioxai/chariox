use rusqlite::{params, OptionalExtension, Transaction};

use crate::error::DaemonError;
use crate::session::{
    DurableWorkflowHotState, RuntimeSession, WorkflowConsole, WorkflowDefinition,
    WorkflowEndpointRuntimeInstance, WorkflowEventBinding, WorkflowEventDeliveryReceipt,
    WorkflowPromptQueueDefinition, WorkflowPublicationDefinition, WorkflowPublicationSnapshot,
    WorkflowQueuedPrompt, WorkflowRun, WorkflowScheduleDefinition,
};

use super::{
    unix_epoch_ms, DurableDeliveryReceiptWrite, DurableKernelStateStore,
    DurableWorkflowHotEntityWrite, DurableWorkflowRunWrite, DurableWorkflowSessionWrite,
    DurableWriteOperation,
};

const WORKFLOW_RUNTIME_STORAGE_VERSION: &str = "2";
const WORKFLOW_RUNTIME_STORAGE_VERSION_KEY: &str = "workflow_runtime_storage_version";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DurableWorkflowRunPage {
    pub(crate) workflow_runs: Vec<WorkflowRun>,
    pub(crate) next_cursor: Option<(u64, String)>,
}

impl DurableKernelStateStore {
    pub(crate) fn with_workflow_runtime_transition_lock<T>(
        &self,
        transition: impl FnOnce() -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let _guard = self
            .workflow_runtime_transition_lock
            .lock()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.lock_workflow_runtime_transition",
                message: error.to_string(),
            })?;
        transition()
    }

    pub(crate) fn persist_workflow_runtime_transition(
        &self,
        session: &RuntimeSession,
        reason: &str,
    ) -> Result<u64, DaemonError> {
        let timestamp_ms = unix_epoch_ms();
        let event_id = format!("state_evt_{timestamp_ms}_{}", super::rand_suffix());
        let payload_json = serde_json::to_string(&serde_json::json!({
            "owner_id": session.host_daemon_id(),
            "session_id": session.id(),
            "reason": reason,
        }))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.encode_workflow_runtime_transition",
            message: error.to_string(),
        })?;
        let encoded = encode_workflow_session(session)?;
        self.writer
            .execute(DurableWriteOperation::WorkflowRuntimeTransition {
                event_id,
                timestamp_ms,
                payload_json,
                owner_id: session.host_daemon_id().to_string(),
                session_id: session.id().to_string(),
                hot_entities: encoded.hot_entities,
                workflow_runs: encoded.workflow_runs,
                delivery_receipts: encoded.delivery_receipts,
            })
    }

    pub(crate) fn persist_workflow_runtime_sessions_transition(
        &self,
        sessions: &[RuntimeSession],
        reason: &str,
    ) -> Result<u64, DaemonError> {
        let Some(first) = sessions.first() else {
            return Err(DaemonError::LocalTransport {
                operation: "durable_state.persist_workflow_runtime_sessions_transition",
                message: "workflow runtime transition requires at least one session".to_string(),
            });
        };
        let owner_id = first.host_daemon_id();
        if sessions
            .iter()
            .any(|session| session.host_daemon_id() != owner_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "durable_state.persist_workflow_runtime_sessions_transition",
                message: "workflow runtime transition cannot span kernel owners".to_string(),
            });
        }
        let timestamp_ms = unix_epoch_ms();
        let event_id = format!("state_evt_{timestamp_ms}_{}", super::rand_suffix());
        let session_ids = sessions
            .iter()
            .map(|session| session.id())
            .collect::<Vec<_>>();
        let payload_json = serde_json::to_string(&serde_json::json!({
            "owner_id": owner_id,
            "session_ids": session_ids,
            "reason": reason,
        }))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.encode_workflow_runtime_sessions_transition",
            message: error.to_string(),
        })?;
        let sessions = sessions
            .iter()
            .map(encode_workflow_session)
            .collect::<Result<Vec<_>, _>>()?;
        self.writer
            .execute(DurableWriteOperation::WorkflowRuntimeSessionsTransition {
                event_id,
                timestamp_ms,
                payload_json,
                owner_id: owner_id.to_string(),
                sessions,
            })
    }

    pub(crate) fn migrate_legacy_workflow_runtime(
        &self,
        owner_id: &str,
        sessions: &[RuntimeSession],
    ) -> Result<(), DaemonError> {
        let hot_entities = sessions
            .iter()
            .map(encode_workflow_hot_entities)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        let workflow_runs = sessions
            .iter()
            .flat_map(|session| {
                session
                    .workflow_runs()
                    .iter()
                    .filter(|run| !run.status().is_terminal())
                    .map(move |run| encode_workflow_run(session.id(), run))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let delivery_receipts = sessions
            .iter()
            .flat_map(|session| {
                session
                    .workflow_event_delivery_receipts()
                    .values()
                    .map(move |receipt| encode_delivery_receipt(session.id(), receipt))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.writer
            .execute(DurableWriteOperation::WorkflowRuntimeMigration {
                owner_id: owner_id.to_string(),
                hot_entities,
                workflow_runs,
                delivery_receipts,
            })?;
        Ok(())
    }

    pub(crate) fn migrate_legacy_workflow_history_chunk(
        &self,
        owner_id: &str,
        workflow_runs: &[(String, WorkflowRun)],
        completed: bool,
    ) -> Result<(), DaemonError> {
        let workflow_runs = workflow_runs
            .iter()
            .map(|(session_id, run)| encode_workflow_run(session_id, run))
            .collect::<Result<Vec<_>, _>>()?;
        self.writer
            .execute(DurableWriteOperation::WorkflowHistoryMigrationChunk {
                owner_id: owner_id.to_string(),
                workflow_runs,
                completed,
            })?;
        Ok(())
    }

    pub(crate) fn persist_session_deleted(
        &self,
        session: &RuntimeSession,
        reason: &str,
    ) -> Result<u64, DaemonError> {
        let timestamp_ms = unix_epoch_ms();
        let event_id = format!("state_evt_{timestamp_ms}_{}", super::rand_suffix());
        let hot_session = session.durable_runtime_snapshot();
        let payload_json = serde_json::to_string(&serde_json::json!({
            "session": hot_session,
            "reason": reason,
        }))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.encode_session_deleted",
            message: error.to_string(),
        })?;
        self.writer.execute(DurableWriteOperation::SessionDelete {
            event_id,
            timestamp_ms,
            payload_json,
            owner_id: session.host_daemon_id().to_string(),
            session_id: session.id().to_string(),
        })
    }

    pub(crate) fn load_active_workflow_runs(
        &self,
        owner_id: &str,
    ) -> Result<Vec<(String, WorkflowRun)>, DaemonError> {
        let connection = self.lock_connection("durable_state.load_active_workflow_runs")?;
        let mut statement = connection
            .prepare(
                "SELECT session_id, payload_json
                 FROM durable_workflow_runs INDEXED BY idx_durable_workflow_runs_active
                 WHERE owner_id = ?1
                   AND status NOT IN ('Completed', 'Failed', 'Stopped')
                 ORDER BY session_id ASC, created_at_ms ASC, run_id ASC",
            )
            .map_err(|error| storage_error("prepare active workflow runs", error))?;
        let rows = statement
            .query_map(params![owner_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| storage_error("query active workflow runs", error))?;
        decode_workflow_run_rows(rows, "read active workflow runs")
    }

    pub(crate) fn load_workflow_hot_states(
        &self,
        owner_id: &str,
    ) -> Result<Vec<(String, DurableWorkflowHotState)>, DaemonError> {
        let connection = self.lock_connection("durable_state.load_workflow_hot_states")?;
        let mut statement = connection
            .prepare(
                "SELECT session_id, entity_kind, entity_id, payload_json
                 FROM durable_workflow_hot_entities
                 WHERE owner_id = ?1
                 ORDER BY session_id ASC, entity_kind ASC, entity_id ASC",
            )
            .map_err(|error| storage_error("prepare workflow hot states", error))?;
        let rows = statement
            .query_map(params![owner_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| storage_error("query workflow hot states", error))?;
        let mut states = std::collections::BTreeMap::<String, DurableWorkflowHotState>::new();
        for row in rows {
            let (session_id, entity_kind, entity_id, payload_json) =
                row.map_err(|error| storage_error("read workflow hot state", error))?;
            let state = states.entry(session_id).or_default();
            decode_workflow_hot_entity(state, &entity_kind, &entity_id, &payload_json)?;
        }
        Ok(states.into_iter().collect())
    }

    pub(crate) fn load_active_delivery_receipts(
        &self,
        owner_id: &str,
        now_ms: u64,
    ) -> Result<Vec<(String, WorkflowEventDeliveryReceipt)>, DaemonError> {
        let connection = self.lock_connection("durable_state.load_active_delivery_receipts")?;
        let mut statement = connection
            .prepare(
                "SELECT session_id, payload_json
                 FROM durable_event_delivery_receipts
                 WHERE owner_id = ?1 AND expires_at_ms > ?2
                 ORDER BY session_id ASC, delivery_id ASC",
            )
            .map_err(|error| storage_error("prepare delivery receipts", error))?;
        let rows = statement
            .query_map(params![owner_id, now_ms as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| storage_error("query delivery receipts", error))?;
        let mut receipts = Vec::new();
        for row in rows {
            let (session_id, payload_json) =
                row.map_err(|error| storage_error("read delivery receipt", error))?;
            let receipt = serde_json::from_str(&payload_json).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "durable_state.decode_delivery_receipt",
                    message: error.to_string(),
                }
            })?;
            receipts.push((session_id, receipt));
        }
        Ok(receipts)
    }

    pub(crate) fn list_workflow_runs_page(
        &self,
        owner_id: &str,
        session_id: &str,
        workflow_id: Option<&str>,
        before: Option<(u64, &str)>,
        limit: usize,
    ) -> Result<DurableWorkflowRunPage, DaemonError> {
        let limit = limit.clamp(1, 500);
        let connection = self.lock_connection("durable_state.list_workflow_runs_page")?;
        let (before_created_at_ms, before_run_id) = before
            .map(|(created_at_ms, run_id)| (created_at_ms as i64, run_id))
            .unwrap_or((i64::MAX, "\u{10ffff}"));
        let mut statement = connection
            .prepare(
                "SELECT created_at_ms, run_id, payload_json
                 FROM durable_workflow_runs
                 WHERE owner_id = ?1 AND session_id = ?2
                   AND (?3 IS NULL OR workflow_id = ?3)
                   AND (created_at_ms < ?4 OR (created_at_ms = ?4 AND run_id < ?5))
                 ORDER BY created_at_ms DESC, run_id DESC
                 LIMIT ?6",
            )
            .map_err(|error| storage_error("prepare workflow run page", error))?;
        let rows = statement
            .query_map(
                params![
                    owner_id,
                    session_id,
                    workflow_id,
                    before_created_at_ms,
                    before_run_id,
                    (limit + 1) as i64
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?.max(0) as u64,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|error| storage_error("query workflow run page", error))?;
        let mut decoded = Vec::new();
        for row in rows {
            let (created_at_ms, run_id, payload_json) =
                row.map_err(|error| storage_error("read workflow run page", error))?;
            let workflow_run = decode_workflow_run(&payload_json)?;
            decoded.push((created_at_ms, run_id, workflow_run));
        }
        let next_cursor = (decoded.len() > limit)
            .then(|| decoded[limit - 1].clone())
            .map(|(created_at_ms, run_id, _)| (created_at_ms, run_id));
        decoded.truncate(limit);
        Ok(DurableWorkflowRunPage {
            workflow_runs: decoded.into_iter().map(|(_, _, run)| run).collect(),
            next_cursor,
        })
    }

    pub(crate) fn resolve_workflow_run(
        &self,
        owner_id: &str,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<Option<WorkflowRun>, DaemonError> {
        let normalized_ref = workflow_run_ref.trim().to_ascii_lowercase();
        let connection = self.lock_connection("durable_state.resolve_workflow_run")?;
        if let Some(payload_json) = connection
            .query_row(
                "SELECT payload_json FROM durable_workflow_runs
                 WHERE owner_id = ?1 AND session_id = ?2 AND run_id = ?3",
                params![owner_id, session_id, normalized_ref],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| storage_error("resolve exact workflow run", error))?
        {
            return decode_workflow_run(&payload_json).map(Some);
        }
        let like = format!("{}%", normalized_ref.replace(['%', '_'], ""));
        let mut statement = connection
            .prepare(
                "SELECT payload_json FROM durable_workflow_runs
                 WHERE owner_id = ?1 AND session_id = ?2 AND run_id LIKE ?3
                 ORDER BY run_id ASC LIMIT 2",
            )
            .map_err(|error| storage_error("prepare workflow run prefix", error))?;
        let rows = statement
            .query_map(params![owner_id, session_id, like], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| storage_error("query workflow run prefix", error))?;
        let mut matches = Vec::new();
        for row in rows {
            matches.push(decode_workflow_run(&row.map_err(|error| {
                storage_error("read workflow run prefix", error)
            })?)?);
        }
        Ok((matches.len() == 1).then(|| matches.remove(0)))
    }
}

pub(super) struct WorkflowRuntimeTransitionWrite<'a> {
    pub(super) event_id: &'a str,
    pub(super) timestamp_ms: u64,
    pub(super) payload_json: &'a str,
    pub(super) owner_id: &'a str,
    pub(super) session_id: &'a str,
    pub(super) hot_entities: &'a [DurableWorkflowHotEntityWrite],
    pub(super) workflow_runs: &'a [DurableWorkflowRunWrite],
    pub(super) delivery_receipts: &'a [DurableDeliveryReceiptWrite],
}

pub(super) fn write_workflow_runtime_transition(
    transaction: &Transaction<'_>,
    write: WorkflowRuntimeTransitionWrite<'_>,
) -> Result<u64, rusqlite::Error> {
    transaction.execute(
        "INSERT INTO durable_state_events (
            event_id, kind, subject_id, timestamp_ms, payload_json
         ) VALUES (?1, 'workflow.runtime.updated', ?2, ?3, ?4)",
        params![
            write.event_id,
            write.session_id,
            write.timestamp_ms as i64,
            write.payload_json
        ],
    )?;
    let sequence = transaction.last_insert_rowid().max(0) as u64;
    write_workflow_hot_entities(
        transaction,
        write.owner_id,
        write.session_id,
        write.timestamp_ms,
        write.hot_entities,
        true,
    )?;
    write_workflow_runs(
        transaction,
        write.owner_id,
        write.timestamp_ms,
        write.workflow_runs,
        true,
    )?;
    write_delivery_receipts(
        transaction,
        write.owner_id,
        write.timestamp_ms,
        write.delivery_receipts,
        true,
    )?;
    delete_missing_active_workflow_runs(
        transaction,
        write.owner_id,
        write.session_id,
        write.workflow_runs,
    )?;
    delete_missing_delivery_receipts(
        transaction,
        write.owner_id,
        write.session_id,
        write.delivery_receipts,
    )?;
    Ok(sequence)
}

pub(super) fn write_workflow_runtime_sessions_transition(
    transaction: &Transaction<'_>,
    event_id: &str,
    timestamp_ms: u64,
    payload_json: &str,
    owner_id: &str,
    sessions: &[DurableWorkflowSessionWrite],
) -> Result<u64, rusqlite::Error> {
    transaction.execute(
        "INSERT INTO durable_state_events (
            event_id, kind, subject_id, timestamp_ms, payload_json
         ) VALUES (?1, 'workflow.runtime.updated', NULL, ?2, ?3)",
        params![event_id, timestamp_ms as i64, payload_json],
    )?;
    let sequence = transaction.last_insert_rowid().max(0) as u64;
    for session in sessions {
        write_workflow_hot_entities(
            transaction,
            owner_id,
            &session.session_id,
            timestamp_ms,
            &session.hot_entities,
            true,
        )?;
        write_workflow_runs(
            transaction,
            owner_id,
            timestamp_ms,
            &session.workflow_runs,
            true,
        )?;
        write_delivery_receipts(
            transaction,
            owner_id,
            timestamp_ms,
            &session.delivery_receipts,
            true,
        )?;
        delete_missing_active_workflow_runs(
            transaction,
            owner_id,
            &session.session_id,
            &session.workflow_runs,
        )?;
        delete_missing_delivery_receipts(
            transaction,
            owner_id,
            &session.session_id,
            &session.delivery_receipts,
        )?;
    }
    Ok(sequence)
}

pub(super) fn write_session_delete(
    transaction: &Transaction<'_>,
    event_id: &str,
    timestamp_ms: u64,
    payload_json: &str,
    owner_id: &str,
    session_id: &str,
) -> Result<u64, rusqlite::Error> {
    transaction.execute(
        "INSERT INTO durable_state_events (
            event_id, kind, subject_id, timestamp_ms, payload_json
         ) VALUES (?1, 'session.deleted', ?2, ?3, ?4)",
        params![event_id, session_id, timestamp_ms as i64, payload_json],
    )?;
    let sequence = transaction.last_insert_rowid().max(0) as u64;
    transaction.execute(
        "DELETE FROM durable_workflow_hot_entities
         WHERE owner_id = ?1 AND session_id = ?2",
        params![owner_id, session_id],
    )?;
    transaction.execute(
        "DELETE FROM durable_workflow_runs
         WHERE owner_id = ?1 AND session_id = ?2",
        params![owner_id, session_id],
    )?;
    transaction.execute(
        "DELETE FROM durable_event_delivery_receipts
         WHERE owner_id = ?1 AND session_id = ?2",
        params![owner_id, session_id],
    )?;
    Ok(sequence)
}

pub(super) fn write_workflow_runtime_migration(
    transaction: &Transaction<'_>,
    owner_id: &str,
    hot_entities: &[DurableWorkflowHotEntityWrite],
    workflow_runs: &[DurableWorkflowRunWrite],
    delivery_receipts: &[DurableDeliveryReceiptWrite],
) -> Result<u64, rusqlite::Error> {
    let timestamp_ms = unix_epoch_ms();
    write_workflow_hot_entities(transaction, owner_id, "", timestamp_ms, hot_entities, false)?;
    write_workflow_runs(transaction, owner_id, timestamp_ms, workflow_runs, false)?;
    write_delivery_receipts(
        transaction,
        owner_id,
        timestamp_ms,
        delivery_receipts,
        false,
    )?;
    transaction.execute(
        "INSERT INTO durable_state_metadata (
            owner_id, metadata_key, metadata_value, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(owner_id, metadata_key) DO UPDATE SET
            metadata_value = excluded.metadata_value,
            updated_at_ms = excluded.updated_at_ms",
        params![
            owner_id,
            WORKFLOW_RUNTIME_STORAGE_VERSION_KEY,
            WORKFLOW_RUNTIME_STORAGE_VERSION,
            timestamp_ms as i64
        ],
    )?;
    Ok(0)
}

pub(super) fn write_workflow_history_migration_chunk(
    transaction: &Transaction<'_>,
    owner_id: &str,
    workflow_runs: &[DurableWorkflowRunWrite],
    completed: bool,
) -> Result<u64, rusqlite::Error> {
    let timestamp_ms = unix_epoch_ms();
    write_workflow_runs(transaction, owner_id, timestamp_ms, workflow_runs, false)?;
    transaction.execute(
        "INSERT INTO durable_state_metadata (
            owner_id, metadata_key, metadata_value, updated_at_ms
         ) VALUES (?1, 'workflow_history_migration_status', ?2, ?3)
         ON CONFLICT(owner_id, metadata_key) DO UPDATE SET
            metadata_value = excluded.metadata_value,
            updated_at_ms = excluded.updated_at_ms",
        params![
            owner_id,
            if completed { "verified" } else { "in_progress" },
            timestamp_ms as i64,
        ],
    )?;
    Ok(0)
}

fn write_workflow_hot_entities(
    transaction: &Transaction<'_>,
    owner_id: &str,
    session_id: &str,
    timestamp_ms: u64,
    hot_entities: &[DurableWorkflowHotEntityWrite],
    replace_session: bool,
) -> Result<(), rusqlite::Error> {
    if replace_session {
        transaction.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS durable_workflow_hot_current_keys (
                entity_kind TEXT NOT NULL,
                entity_id TEXT NOT NULL,
                PRIMARY KEY(entity_kind, entity_id)
             );
             DELETE FROM durable_workflow_hot_current_keys;",
        )?;
    }
    let mut key_statement = if replace_session {
        Some(transaction.prepare(
            "INSERT INTO durable_workflow_hot_current_keys (entity_kind, entity_id)
             VALUES (?1, ?2)",
        )?)
    } else {
        None
    };
    let conflict_clause = if replace_session {
        "ON CONFLICT(owner_id, session_id, entity_kind, entity_id) DO UPDATE SET
            updated_at_ms = excluded.updated_at_ms,
            payload_json = excluded.payload_json
         WHERE durable_workflow_hot_entities.payload_json <> excluded.payload_json"
    } else {
        "ON CONFLICT(owner_id, session_id, entity_kind, entity_id) DO NOTHING"
    };
    let sql = format!(
        "INSERT INTO durable_workflow_hot_entities (
            owner_id, session_id, entity_kind, entity_id, updated_at_ms, payload_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         {conflict_clause}"
    );
    let mut entity_statement = transaction.prepare(&sql)?;
    for entity in hot_entities {
        if let Some(statement) = key_statement.as_mut() {
            statement.execute(params![entity.entity_kind, entity.entity_id])?;
        }
        entity_statement.execute(params![
            owner_id,
            entity.session_id,
            entity.entity_kind,
            entity.entity_id,
            timestamp_ms as i64,
            entity.payload_json,
        ])?;
    }
    drop(entity_statement);
    drop(key_statement);
    if replace_session {
        transaction.execute(
            "DELETE FROM durable_workflow_hot_entities
             WHERE owner_id = ?1 AND session_id = ?2
               AND NOT EXISTS (
                 SELECT 1 FROM durable_workflow_hot_current_keys current
                 WHERE current.entity_kind = durable_workflow_hot_entities.entity_kind
                   AND current.entity_id = durable_workflow_hot_entities.entity_id
               )",
            params![owner_id, session_id],
        )?;
    }
    Ok(())
}

fn write_workflow_runs(
    transaction: &Transaction<'_>,
    owner_id: &str,
    timestamp_ms: u64,
    workflow_runs: &[DurableWorkflowRunWrite],
    update_existing: bool,
) -> Result<(), rusqlite::Error> {
    let conflict_clause = if update_existing {
        "ON CONFLICT(owner_id, session_id, run_id) DO UPDATE SET
            workflow_id = excluded.workflow_id,
            status = excluded.status,
            created_at_ms = excluded.created_at_ms,
            completed_at_ms = excluded.completed_at_ms,
            updated_at_ms = excluded.updated_at_ms,
            payload_json = excluded.payload_json
         WHERE durable_workflow_runs.payload_json <> excluded.payload_json"
    } else {
        "ON CONFLICT(owner_id, session_id, run_id) DO NOTHING"
    };
    let sql = format!(
        "INSERT INTO durable_workflow_runs (
            owner_id, session_id, run_id, workflow_id, status, created_at_ms,
            completed_at_ms, updated_at_ms, payload_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         {conflict_clause}"
    );
    let mut statement = transaction.prepare(&sql)?;
    for workflow_run in workflow_runs {
        statement.execute(params![
            owner_id,
            workflow_run.session_id,
            workflow_run.run_id,
            workflow_run.workflow_id,
            workflow_run.status,
            workflow_run.created_at_ms as i64,
            workflow_run.completed_at_ms.map(|value| value as i64),
            timestamp_ms as i64,
            workflow_run.payload_json,
        ])?;
    }
    Ok(())
}

fn write_delivery_receipts(
    transaction: &Transaction<'_>,
    owner_id: &str,
    now_ms: u64,
    delivery_receipts: &[DurableDeliveryReceiptWrite],
    update_existing: bool,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "DELETE FROM durable_event_delivery_receipts
         WHERE owner_id = ?1 AND expires_at_ms <= ?2",
        params![owner_id, now_ms as i64],
    )?;
    let conflict_clause = if update_existing {
        "ON CONFLICT(owner_id, session_id, delivery_id) DO UPDATE SET
            binding_id = excluded.binding_id,
            expires_at_ms = excluded.expires_at_ms,
            payload_json = excluded.payload_json
         WHERE durable_event_delivery_receipts.payload_json <> excluded.payload_json"
    } else {
        "ON CONFLICT(owner_id, session_id, delivery_id) DO NOTHING"
    };
    let sql = format!(
        "INSERT INTO durable_event_delivery_receipts (
            owner_id, session_id, delivery_id, binding_id, expires_at_ms, payload_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         {conflict_clause}"
    );
    let mut statement = transaction.prepare(&sql)?;
    for receipt in delivery_receipts {
        statement.execute(params![
            owner_id,
            receipt.session_id,
            receipt.delivery_id,
            receipt.binding_id,
            receipt.expires_at_ms as i64,
            receipt.payload_json,
        ])?;
    }
    Ok(())
}

fn delete_missing_active_workflow_runs(
    transaction: &Transaction<'_>,
    owner_id: &str,
    session_id: &str,
    workflow_runs: &[DurableWorkflowRunWrite],
) -> Result<(), rusqlite::Error> {
    transaction.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS durable_workflow_current_run_ids (
            run_id TEXT PRIMARY KEY
         );
         DELETE FROM durable_workflow_current_run_ids;",
    )?;
    {
        let mut statement = transaction
            .prepare("INSERT INTO durable_workflow_current_run_ids (run_id) VALUES (?1)")?;
        for run in workflow_runs {
            statement.execute([&run.run_id])?;
        }
    }
    transaction.execute(
        "DELETE FROM durable_workflow_runs
         WHERE owner_id = ?1 AND session_id = ?2
           AND status NOT IN ('Completed', 'Failed', 'Stopped')
           AND NOT EXISTS (
               SELECT 1 FROM durable_workflow_current_run_ids current
               WHERE current.run_id = durable_workflow_runs.run_id
           )",
        params![owner_id, session_id],
    )?;
    Ok(())
}

fn delete_missing_delivery_receipts(
    transaction: &Transaction<'_>,
    owner_id: &str,
    session_id: &str,
    delivery_receipts: &[DurableDeliveryReceiptWrite],
) -> Result<(), rusqlite::Error> {
    transaction.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS durable_workflow_current_delivery_ids (
            delivery_id TEXT PRIMARY KEY
         );
         DELETE FROM durable_workflow_current_delivery_ids;",
    )?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO durable_workflow_current_delivery_ids (delivery_id) VALUES (?1)",
        )?;
        for receipt in delivery_receipts {
            statement.execute([&receipt.delivery_id])?;
        }
    }
    transaction.execute(
        "DELETE FROM durable_event_delivery_receipts
         WHERE owner_id = ?1 AND session_id = ?2
           AND NOT EXISTS (
               SELECT 1 FROM durable_workflow_current_delivery_ids current
               WHERE current.delivery_id = durable_event_delivery_receipts.delivery_id
           )",
        params![owner_id, session_id],
    )?;
    Ok(())
}

fn encode_workflow_session(
    session: &RuntimeSession,
) -> Result<DurableWorkflowSessionWrite, DaemonError> {
    Ok(DurableWorkflowSessionWrite {
        session_id: session.id().to_string(),
        hot_entities: encode_workflow_hot_entities(session)?,
        workflow_runs: session
            .workflow_runs()
            .iter()
            .map(|workflow_run| encode_workflow_run(session.id(), workflow_run))
            .collect::<Result<Vec<_>, _>>()?,
        delivery_receipts: session
            .workflow_event_delivery_receipts()
            .values()
            .map(|receipt| encode_delivery_receipt(session.id(), receipt))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn encode_workflow_hot_entities(
    session: &RuntimeSession,
) -> Result<Vec<DurableWorkflowHotEntityWrite>, DaemonError> {
    let session_id = session.id();
    let state = session.durable_workflow_hot_state();
    let mut entities = vec![encode_workflow_hot_entity(
        session_id,
        "state_marker",
        "state",
        &serde_json::json!({}),
    )?];
    for workflow in state.workflows {
        entities.push(encode_workflow_hot_entity(
            session_id,
            "workflow",
            workflow.id(),
            &workflow,
        )?);
    }
    for queue in state.workflow_prompt_queues {
        entities.push(encode_workflow_hot_entity(
            session_id,
            "queue",
            queue.id(),
            &queue,
        )?);
    }
    for prompt in state.workflow_queued_prompts {
        entities.push(encode_workflow_hot_entity(
            session_id,
            "queued_prompt",
            prompt.id(),
            &prompt,
        )?);
    }
    for instance in state.workflow_runtime_instances {
        entities.push(encode_workflow_hot_entity(
            session_id,
            "runtime_instance",
            instance.id(),
            &instance,
        )?);
    }
    for schedule in state.workflow_schedules {
        entities.push(encode_workflow_hot_entity(
            session_id,
            "schedule",
            schedule.id(),
            &schedule,
        )?);
    }
    for console in state.workflow_consoles {
        entities.push(encode_workflow_hot_entity(
            session_id,
            "console",
            console.workflow_id(),
            &console,
        )?);
    }
    for publication in state.workflow_publications {
        entities.push(encode_workflow_hot_entity(
            session_id,
            "publication",
            publication.id(),
            &publication,
        )?);
    }
    for (publication_id, snapshot) in state.workflow_publication_snapshots {
        entities.push(encode_workflow_hot_entity(
            session_id,
            "publication_snapshot",
            &publication_id,
            &snapshot,
        )?);
    }
    for binding in state.workflow_event_bindings {
        entities.push(encode_workflow_hot_entity(
            session_id,
            "event_binding",
            &binding.id,
            &binding,
        )?);
    }
    Ok(entities)
}

fn encode_workflow_hot_entity(
    session_id: &str,
    entity_kind: &str,
    entity_id: &str,
    payload: &impl serde::Serialize,
) -> Result<DurableWorkflowHotEntityWrite, DaemonError> {
    Ok(DurableWorkflowHotEntityWrite {
        session_id: session_id.to_string(),
        entity_kind: entity_kind.to_string(),
        entity_id: entity_id.to_string(),
        payload_json: serde_json::to_string(payload).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "durable_state.encode_workflow_hot_entity",
                message: error.to_string(),
            }
        })?,
    })
}

fn decode_workflow_hot_entity(
    state: &mut DurableWorkflowHotState,
    entity_kind: &str,
    entity_id: &str,
    payload_json: &str,
) -> Result<(), DaemonError> {
    macro_rules! decode {
        ($type:ty) => {
            serde_json::from_str::<$type>(payload_json).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "durable_state.decode_workflow_hot_entity",
                    message: format!("invalid {entity_kind} `{entity_id}`: {error}"),
                }
            })?
        };
    }
    match entity_kind {
        "state_marker" => {}
        "workflow" => state.workflows.push(decode!(WorkflowDefinition)),
        "queue" => state
            .workflow_prompt_queues
            .push(decode!(WorkflowPromptQueueDefinition)),
        "queued_prompt" => state
            .workflow_queued_prompts
            .push_back(decode!(WorkflowQueuedPrompt)),
        "runtime_instance" => state
            .workflow_runtime_instances
            .push(decode!(WorkflowEndpointRuntimeInstance)),
        "schedule" => state
            .workflow_schedules
            .push(decode!(WorkflowScheduleDefinition)),
        "console" => state.workflow_consoles.push(decode!(WorkflowConsole)),
        "publication" => state
            .workflow_publications
            .push(decode!(WorkflowPublicationDefinition)),
        "publication_snapshot" => {
            state
                .workflow_publication_snapshots
                .insert(entity_id.to_string(), decode!(WorkflowPublicationSnapshot));
        }
        "event_binding" => state
            .workflow_event_bindings
            .push(decode!(WorkflowEventBinding)),
        unsupported => {
            return Err(DaemonError::LocalTransport {
                operation: "durable_state.decode_workflow_hot_entity",
                message: format!("unsupported workflow hot entity kind `{unsupported}`"),
            });
        }
    }
    Ok(())
}

fn encode_workflow_run(
    session_id: &str,
    workflow_run: &WorkflowRun,
) -> Result<DurableWorkflowRunWrite, DaemonError> {
    Ok(DurableWorkflowRunWrite {
        session_id: session_id.to_string(),
        run_id: workflow_run.id().to_string(),
        workflow_id: workflow_run.workflow_id().to_string(),
        status: format!("{:?}", workflow_run.status()),
        created_at_ms: workflow_run.created_at_ms(),
        completed_at_ms: workflow_run.completed_at_ms(),
        payload_json: serde_json::to_string(workflow_run).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "durable_state.encode_workflow_run",
                message: error.to_string(),
            }
        })?,
    })
}

fn encode_delivery_receipt(
    session_id: &str,
    receipt: &WorkflowEventDeliveryReceipt,
) -> Result<DurableDeliveryReceiptWrite, DaemonError> {
    Ok(DurableDeliveryReceiptWrite {
        session_id: session_id.to_string(),
        delivery_id: receipt.delivery_id.clone(),
        binding_id: receipt.binding_id.clone(),
        expires_at_ms: receipt.expires_at_ms,
        payload_json: serde_json::to_string(receipt).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "durable_state.encode_delivery_receipt",
                message: error.to_string(),
            }
        })?,
    })
}

fn decode_workflow_run(payload_json: &str) -> Result<WorkflowRun, DaemonError> {
    serde_json::from_str(payload_json).map_err(|error| DaemonError::LocalTransport {
        operation: "durable_state.decode_workflow_run",
        message: error.to_string(),
    })
}

fn decode_workflow_run_rows(
    rows: impl Iterator<Item = rusqlite::Result<(String, String)>>,
    operation: &'static str,
) -> Result<Vec<(String, WorkflowRun)>, DaemonError> {
    let mut workflow_runs = Vec::new();
    for row in rows {
        let (session_id, payload_json) = row.map_err(|error| storage_error(operation, error))?;
        workflow_runs.push((session_id, decode_workflow_run(&payload_json)?));
    }
    Ok(workflow_runs)
}

fn storage_error(operation: &'static str, error: rusqlite::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::WorkflowRunStatus;
    use std::collections::BTreeMap;
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    fn temp_store(label: &str) -> (DurableKernelStateStore, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "chariox-{label}-{}-{}.db",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should follow epoch")
                .as_nanos()
        ));
        (
            DurableKernelStateStore::open(path.clone()).expect("store should open"),
            path,
        )
    }

    fn session_with_runs() -> RuntimeSession {
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "/workspace",
            "/workspace",
            "machine-1",
            "kernel-1",
        );
        let active = WorkflowRun::new(
            "run-active",
            "workflow-1",
            "endpoint-1",
            "node-1",
            Some("active".to_string()),
            None,
            Vec::new(),
            Vec::new(),
        );
        let mut completed = WorkflowRun::new(
            "run-completed",
            "workflow-1",
            "endpoint-1",
            "node-1",
            Some("completed".to_string()),
            None,
            Vec::new(),
            Vec::new(),
        );
        completed.set_status(WorkflowRunStatus::Completed);
        session.create_workflow_run(active);
        session.create_workflow_run(completed);
        session.record_workflow_event_delivery_receipt(WorkflowEventDeliveryReceipt {
            delivery_id: "delivery-1".to_string(),
            binding_id: "binding-1".to_string(),
            occurrence_id: "occurrence-1".to_string(),
            queued_prompt_id: "queued-1".to_string(),
            accepted_at_ms: unix_epoch_ms(),
            expires_at_ms: unix_epoch_ms().saturating_add(60_000),
        });
        session.add_workflow_runtime_instance(WorkflowEndpointRuntimeInstance::new(
            "instance-1",
            "workflow-1",
            "endpoint-1",
            1,
            1,
            true,
            BTreeMap::from([("node-1".to_string(), "agent-1".to_string())]),
            "/workspace",
        ));
        session
    }

    #[test]
    fn workflow_runtime_transition_separates_hot_state_from_run_history_and_receipts() {
        let (store, path) = temp_store("workflow-runtime-separation");
        let session = session_with_runs();

        store
            .persist_workflow_runtime_transition(&session, "test")
            .expect("workflow transition should persist");
        drop(store);
        let store = DurableKernelStateStore::open(path.clone())
            .expect("workflow runtime store should reopen after restart");

        let hot_states = store
            .load_workflow_hot_states("kernel-1")
            .expect("workflow hot state should load");
        assert_eq!(hot_states.len(), 1);
        assert_eq!(hot_states[0].1.workflow_runtime_instances.len(), 1);
        assert_eq!(
            hot_states[0].1.workflow_runtime_instances[0].id(),
            "instance-1"
        );

        let active = store
            .load_active_workflow_runs("kernel-1")
            .expect("active runs should load");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].1.id(), "run-active");

        let page = store
            .list_workflow_runs_page("kernel-1", "session-1", None, None, 10)
            .expect("run page should load");
        assert_eq!(page.workflow_runs.len(), 2);
        assert!(page.next_cursor.is_none());

        let receipts = store
            .load_active_delivery_receipts("kernel-1", unix_epoch_ms())
            .expect("receipts should load");
        assert_eq!(receipts.len(), 1);

        let mut settled = session.clone();
        settled
            .workflow_run_mut("run-active")
            .expect("active run should exist")
            .set_status(WorkflowRunStatus::Completed);
        store
            .persist_workflow_runtime_transition(&settled, "run-completed")
            .expect("terminal run should persist before archival");
        settled.archive_terminal_workflow_runs();
        settled.prune_expired_workflow_event_delivery_receipts(u64::MAX);
        store
            .persist_workflow_runtime_transition(&settled, "hot-state-pruned")
            .expect("pruned hot state should persist");
        assert!(store
            .load_active_workflow_runs("kernel-1")
            .expect("pruned active runs should load")
            .is_empty());
        assert!(store
            .load_active_delivery_receipts("kernel-1", unix_epoch_ms())
            .expect("pruned receipts should load")
            .is_empty());
        assert_eq!(
            store
                .list_workflow_runs_page("kernel-1", "session-1", None, None, 10)
                .expect("terminal history should remain")
                .workflow_runs
                .len(),
            2,
            "pruning hot active state must retain terminal run history",
        );

        let event = store
            .load_events_by_kind("workflow.runtime.updated")
            .expect("workflow event should load")
            .pop()
            .expect("workflow event should exist");
        assert_eq!(event.payload["session_id"], "session-1");
        assert!(event.payload.get("session").is_none());
        assert!(serde_json::to_vec(&event.payload).unwrap().len() < 128);
        assert_eq!(
            store
                .load_workflow_hot_states("kernel-1")
                .expect("workflow hot state should load")
                .len(),
            1
        );

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn workflow_runtime_transition_lock_serializes_snapshot_capture_and_commit() {
        let (store, path) = temp_store("workflow-runtime-transition-order");
        let session = Arc::new(Mutex::new(session_with_runs()));
        let delayed_store = store.clone();
        let delayed_session = Arc::clone(&session);
        let (start_delayed_tx, start_delayed_rx) = mpsc::channel();
        let (delayed_attempt_tx, delayed_attempt_rx) = mpsc::channel();

        session
            .lock()
            .expect("session should lock")
            .workflow_run_mut("run-active")
            .expect("active run should exist")
            .set_status(WorkflowRunStatus::Completed);

        let delayed = thread::spawn(move || {
            start_delayed_rx
                .recv()
                .expect("newer transition should release delayed transition");
            delayed_attempt_tx
                .send(())
                .expect("delayed transition should report its attempt");
            delayed_store
                .with_workflow_runtime_transition_lock(|| {
                    let snapshot = delayed_session.lock().expect("session should lock").clone();
                    delayed_store
                        .persist_workflow_runtime_transition(&snapshot, "delayed_transition")
                })
                .expect("delayed transition should persist");
        });

        store
            .with_workflow_runtime_transition_lock(|| {
                start_delayed_tx
                    .send(())
                    .expect("delayed transition should start");
                delayed_attempt_rx
                    .recv()
                    .expect("delayed transition should wait on the shared lock");
                let snapshot = session.lock().expect("session should lock").clone();
                store.persist_workflow_runtime_transition(&snapshot, "newer_transition")?;
                session
                    .lock()
                    .expect("session should lock")
                    .archive_terminal_workflow_runs();
                Ok(())
            })
            .expect("newer transition should persist");
        delayed.join().expect("delayed transition should join");

        assert!(store
            .load_active_workflow_runs("kernel-1")
            .expect("active runs should load")
            .is_empty());
        assert_eq!(
            store
                .resolve_workflow_run("kernel-1", "session-1", "run-active")
                .expect("run should resolve")
                .expect("run history should remain")
                .status(),
            WorkflowRunStatus::Completed,
            "a delayed transition must snapshot current state after the newer commit",
        );

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn normalized_active_runs_replace_stale_legacy_nonterminal_runs() {
        let (store, path) = temp_store("workflow-runtime-active-replacement");
        let mut stale_session = session_with_runs();
        let mut settled_session = stale_session.clone();
        settled_session
            .workflow_run_mut("run-active")
            .expect("active run should exist")
            .set_status(WorkflowRunStatus::Completed);
        store
            .persist_workflow_runtime_transition(&settled_session, "settled")
            .expect("settled state should persist");

        let normalized_active_runs = store
            .load_active_workflow_runs("kernel-1")
            .expect("active runs should load")
            .into_iter()
            .map(|(_, workflow_run)| workflow_run)
            .collect();
        stale_session.restore_active_workflow_runs(normalized_active_runs);

        assert!(stale_session.workflow_run("run-active").is_none());
        assert_eq!(
            stale_session
                .workflow_run("run-completed")
                .expect("terminal run awaiting archival should remain")
                .status(),
            WorkflowRunStatus::Completed,
        );

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn workflow_runtime_transition_deletes_removed_runtime_instances() {
        let (store, path) = temp_store("workflow-runtime-instance-deletion");
        let mut session = session_with_runs();
        store
            .persist_workflow_runtime_transition(&session, "instance_created")
            .expect("runtime instance should persist");
        assert_eq!(
            store
                .load_workflow_hot_states("kernel-1")
                .expect("workflow hot state should load")[0]
                .1
                .workflow_runtime_instances
                .len(),
            1,
        );

        session
            .remove_workflow_runtime_instance("instance-1")
            .expect("runtime instance should exist");
        store
            .persist_workflow_runtime_transition(&session, "instance_removed")
            .expect("runtime instance removal should persist");
        assert!(store
            .load_workflow_hot_states("kernel-1")
            .expect("workflow hot state should load")[0]
            .1
            .workflow_runtime_instances
            .is_empty());

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn multi_session_workflow_transition_is_atomic_and_never_writes_session_aggregates() {
        let (store, path) = temp_store("workflow-runtime-multi-session");
        let first = session_with_runs();
        let mut second = RuntimeSession::new(
            "session-2",
            None,
            "/workspace",
            "/workspace",
            "machine-1",
            "kernel-1",
        );
        second.create_workflow_run(WorkflowRun::new(
            "run-second",
            "workflow-2",
            "endpoint-2",
            "node-2",
            Some("second".to_string()),
            None,
            Vec::new(),
            Vec::new(),
        ));

        store
            .persist_workflow_runtime_sessions_transition(&[first, second], "binding_transferred")
            .expect("multi-session transition should persist");

        let hot_states = store
            .load_workflow_hot_states("kernel-1")
            .expect("hot states should load");
        assert_eq!(hot_states.len(), 2);
        assert_eq!(
            store
                .load_active_workflow_runs("kernel-1")
                .expect("active runs should load")
                .len(),
            2
        );
        let events = store
            .load_events_by_kind("workflow.runtime.updated")
            .expect("workflow events should load");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["reason"], "binding_transferred");
        assert_eq!(
            events[0].payload["session_ids"]
                .as_array()
                .expect("session ids should be an array")
                .len(),
            2
        );
        assert!(events[0].payload.get("sessions").is_none());
        assert!(store
            .load_events_by_kind("sessions.updated")
            .expect("aggregate events should load")
            .is_empty());

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_workflow_runtime_migration_is_idempotent_and_paginated() {
        let (store, path) = temp_store("workflow-runtime-migration");
        let session = session_with_runs();

        store
            .migrate_legacy_workflow_runtime("kernel-1", std::slice::from_ref(&session))
            .expect("first migration should succeed");
        let mut newer = session.clone();
        newer
            .workflow_run_mut("run-active")
            .expect("active run should exist")
            .set_status(WorkflowRunStatus::Running);
        store
            .persist_workflow_runtime_transition(&newer, "newer-state")
            .expect("newer workflow state should persist");
        store
            .migrate_legacy_workflow_runtime("kernel-1", std::slice::from_ref(&session))
            .expect("second migration should succeed");

        assert_eq!(
            store
                .resolve_workflow_run("kernel-1", "session-1", "run-active")
                .expect("active run should resolve")
                .expect("active run should exist")
                .status(),
            WorkflowRunStatus::Running,
            "legacy migration must not overwrite normalized runtime state",
        );

        let first = store
            .list_workflow_runs_page("kernel-1", "session-1", None, None, 1)
            .expect("first page should load");
        assert_eq!(first.workflow_runs.len(), 1);
        let cursor = first.next_cursor.expect("first page should have cursor");
        let second = store
            .list_workflow_runs_page(
                "kernel-1",
                "session-1",
                None,
                Some((cursor.0, cursor.1.as_str())),
                1,
            )
            .expect("second page should load");
        assert_eq!(second.workflow_runs.len(), 1);
        assert_ne!(first.workflow_runs[0].id(), second.workflow_runs[0].id());

        let connection = store
            .lock_connection("test workflow runtime migration")
            .expect("connection should lock");
        let count = connection
            .query_row(
                "SELECT COUNT(*) FROM durable_workflow_runs WHERE owner_id = 'kernel-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("run count should load");
        assert_eq!(count, 2);
        drop(connection);

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_history_migration_resumes_idempotently_and_marks_verified() {
        let (store, path) = temp_store("workflow-history-migration-resume");
        let session = session_with_runs();
        let archived = session
            .workflow_run("run-completed")
            .expect("completed run should exist")
            .clone();

        store
            .migrate_legacy_workflow_runtime("kernel-1", std::slice::from_ref(&session))
            .expect("hot migration should succeed");
        assert_eq!(
            store
                .list_workflow_runs_page("kernel-1", "session-1", None, None, 10)
                .expect("hot run page should load")
                .workflow_runs
                .len(),
            1,
            "synchronous migration must leave terminal history for the background worker",
        );

        store
            .migrate_legacy_workflow_history_chunk(
                "kernel-1",
                &[("session-1".to_string(), archived.clone())],
                false,
            )
            .expect("first history chunk should persist");
        drop(store);

        let resumed = DurableKernelStateStore::open(path.clone()).expect("store should reopen");
        resumed
            .migrate_legacy_workflow_history_chunk(
                "kernel-1",
                &[("session-1".to_string(), archived)],
                true,
            )
            .expect("replayed history chunk should complete");
        assert_eq!(
            resumed
                .list_workflow_runs_page("kernel-1", "session-1", None, None, 10)
                .expect("migrated run page should load")
                .workflow_runs
                .len(),
            2,
            "replaying a chunk after interruption must not duplicate history",
        );
        let connection = resumed
            .lock_connection("test history migration status")
            .expect("connection should lock");
        let status = connection
            .query_row(
                "SELECT metadata_value FROM durable_state_metadata
                 WHERE owner_id = 'kernel-1'
                   AND metadata_key = 'workflow_history_migration_status'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("migration status should load");
        assert_eq!(status, "verified");
        drop(connection);
        drop(resumed);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn session_delete_atomically_removes_normalized_workflow_state() {
        let (store, path) = temp_store("workflow-runtime-session-delete");
        let session = session_with_runs();

        let workflow_sequence = store
            .persist_workflow_runtime_transition(&session, "test")
            .expect("workflow transition should persist");
        let delete_sequence = store
            .persist_session_deleted(&session, "test_delete")
            .expect("session deletion should persist");
        assert!(delete_sequence > workflow_sequence);

        assert!(store
            .list_workflow_runs_page("kernel-1", "session-1", None, None, 10)
            .expect("run page should load")
            .workflow_runs
            .is_empty());
        assert!(store
            .load_active_delivery_receipts("kernel-1", unix_epoch_ms())
            .expect("receipts should load")
            .is_empty());
        let deleted = store
            .load_events_by_kind("session.deleted")
            .expect("delete event should load");
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].sequence, delete_sequence);

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn large_completed_history_keeps_hot_restore_and_transition_tail_bounded() {
        const COMPLETED_RUNS: usize = 10_000;
        const HOT_TRANSITIONS: usize = 50;
        let (store, path) = temp_store("workflow-runtime-large-history");
        let mut hot_session = RuntimeSession::new(
            "session-scale",
            None,
            "/workspace",
            "/workspace",
            "machine-1",
            "kernel-1",
        );
        hot_session.create_workflow_run(WorkflowRun::new(
            "run-active",
            "workflow-1",
            "endpoint-1",
            "node-1",
            Some("active".to_string()),
            None,
            Vec::new(),
            Vec::new(),
        ));
        let completed = (0..COMPLETED_RUNS)
            .map(|index| {
                let mut run = WorkflowRun::new(
                    format!("run-completed-{index:05}"),
                    "workflow-1",
                    "endpoint-1",
                    "node-1",
                    Some(format!("completed prompt {index}")),
                    None,
                    Vec::new(),
                    Vec::new(),
                );
                run.set_status(WorkflowRunStatus::Completed);
                ("session-scale".to_string(), run)
            })
            .collect::<Vec<_>>();
        let mut legacy_session = hot_session.clone();
        for (_, run) in &completed {
            legacy_session.create_workflow_run(run.clone());
        }
        let legacy_payload = serde_json::to_vec(&legacy_session)
            .expect("legacy aggregate should encode for the baseline");
        let legacy_decode_started = Instant::now();
        let decoded_legacy: RuntimeSession = serde_json::from_slice(&legacy_payload)
            .expect("legacy aggregate should decode for the baseline");
        let legacy_decode_ms = legacy_decode_started.elapsed().as_millis();
        assert_eq!(decoded_legacy.workflow_runs().len(), COMPLETED_RUNS + 1);
        drop(decoded_legacy);
        drop(legacy_session);
        for (index, chunk) in completed.chunks(256).enumerate() {
            store
                .migrate_legacy_workflow_history_chunk(
                    "kernel-1",
                    chunk,
                    (index + 1) * 256 >= completed.len(),
                )
                .expect("history chunk should migrate");
        }
        for index in 0..HOT_TRANSITIONS {
            store
                .persist_workflow_runtime_transition(&hot_session, &format!("scale-{index}"))
                .expect("bounded hot transition should persist");
        }

        let hot_restore_started = Instant::now();
        let hot_states = store
            .load_workflow_hot_states("kernel-1")
            .expect("hot states should load");
        let active = store
            .load_active_workflow_runs("kernel-1")
            .expect("active runs should load");
        let hot_restore_ms = hot_restore_started.elapsed().as_millis();
        assert_eq!(hot_states.len(), 1);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].1.id(), "run-active");
        assert!(
            hot_restore_ms < 2_000,
            "bounded hot restore exceeded the local two-second readiness budget"
        );
        let history_page_started = Instant::now();
        let first_page = store
            .list_workflow_runs_page("kernel-1", "session-scale", None, None, 50)
            .expect("history page should load");
        let history_page_ms = history_page_started.elapsed().as_millis();
        assert_eq!(first_page.workflow_runs.len(), 50);
        assert!(first_page.next_cursor.is_some());
        let tail = store
            .event_tail_statistics(0)
            .expect("event tail should measure");
        assert_eq!(tail.event_count, HOT_TRANSITIONS as u64);
        assert!(
            tail.encoded_bytes < 32 * 1024,
            "hot transition journal grew to {} bytes with {COMPLETED_RUNS} historical runs",
            tail.encoded_bytes,
        );
        let connection = store
            .lock_connection("test large workflow history")
            .expect("connection should lock");
        let history_count = connection
            .query_row(
                "SELECT COUNT(*) FROM durable_workflow_runs
                 WHERE owner_id = 'kernel-1' AND session_id = 'session-scale'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("history count should load");
        assert_eq!(history_count, COMPLETED_RUNS as i64 + 1);
        let max_transition_bytes = connection
            .query_row(
                "SELECT COALESCE(MAX(length(CAST(payload_json AS BLOB))), 0)
                 FROM durable_state_events
                 WHERE kind = 'workflow.runtime.updated'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("maximum transition size should load");
        assert!(max_transition_bytes < 512);
        let active_query_plan = connection
            .query_row(
                "EXPLAIN QUERY PLAN
                 SELECT session_id, payload_json
                 FROM durable_workflow_runs INDEXED BY idx_durable_workflow_runs_active
                 WHERE owner_id = 'kernel-1'
                   AND status NOT IN ('Completed', 'Failed', 'Stopped')
                 ORDER BY session_id ASC, created_at_ms ASC, run_id ASC",
                [],
                |row| row.get::<_, String>(3),
            )
            .expect("active run query plan should load");
        assert!(
            active_query_plan.contains("idx_durable_workflow_runs_active"),
            "active restore must use the terminal-history-independent partial index: {active_query_plan}"
        );
        drop(connection);

        let database_bytes = std::fs::metadata(&path)
            .expect("scale database metadata should load")
            .len();
        eprintln!(
            "{}",
            serde_json::json!({
                "completed_runs": COMPLETED_RUNS,
                "active_runs": active.len(),
                "legacy_aggregate_bytes": legacy_payload.len(),
                "legacy_aggregate_decode_ms": legacy_decode_ms,
                "normalized_hot_restore_ms": hot_restore_ms,
                "history_page_ms": history_page_ms,
                "transition_count": tail.event_count,
                "transition_tail_bytes": tail.encoded_bytes,
                "maximum_transition_payload_bytes": max_transition_bytes,
                "active_query_plan": active_query_plan,
                "database_bytes": database_bytes,
            })
        );

        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
