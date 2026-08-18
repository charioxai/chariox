use rusqlite::{params, OptionalExtension, Transaction};

use crate::error::DaemonError;
use crate::session::{RuntimeSession, WorkflowEventDeliveryReceipt, WorkflowRun};

use super::{
    unix_epoch_ms, DurableDeliveryReceiptWrite, DurableKernelStateStore, DurableWorkflowRunWrite,
    DurableWriteOperation,
};

const WORKFLOW_RUNTIME_STORAGE_VERSION: &str = "1";
const WORKFLOW_RUNTIME_STORAGE_VERSION_KEY: &str = "workflow_runtime_storage_version";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DurableWorkflowRunPage {
    pub(crate) workflow_runs: Vec<WorkflowRun>,
    pub(crate) next_cursor: Option<(u64, String)>,
}

impl DurableKernelStateStore {
    pub(crate) fn persist_workflow_runtime_transition(
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
            operation: "durable_state.encode_workflow_runtime_transition",
            message: error.to_string(),
        })?;
        let workflow_runs = session
            .workflow_runs()
            .iter()
            .map(|workflow_run| encode_workflow_run(session.id(), workflow_run))
            .collect::<Result<Vec<_>, _>>()?;
        let delivery_receipts = session
            .workflow_event_delivery_receipts()
            .values()
            .map(|receipt| encode_delivery_receipt(session.id(), receipt))
            .collect::<Result<Vec<_>, _>>()?;
        self.writer
            .execute(DurableWriteOperation::WorkflowRuntimeTransition {
                event_id,
                timestamp_ms,
                payload_json,
                owner_id: session.host_daemon_id().to_string(),
                session_id: session.id().to_string(),
                workflow_runs,
                delivery_receipts,
            })
    }

    pub(crate) fn migrate_legacy_workflow_runtime(
        &self,
        owner_id: &str,
        sessions: &[RuntimeSession],
    ) -> Result<(), DaemonError> {
        let workflow_runs = sessions
            .iter()
            .flat_map(|session| {
                session
                    .workflow_runs()
                    .iter()
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
                workflow_runs,
                delivery_receipts,
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
                 FROM durable_workflow_runs
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

pub(super) fn write_workflow_runtime_transition(
    transaction: &Transaction<'_>,
    event_id: &str,
    timestamp_ms: u64,
    payload_json: &str,
    owner_id: &str,
    session_id: &str,
    workflow_runs: &[DurableWorkflowRunWrite],
    delivery_receipts: &[DurableDeliveryReceiptWrite],
) -> Result<u64, rusqlite::Error> {
    transaction.execute(
        "INSERT INTO durable_state_events (
            event_id, kind, subject_id, timestamp_ms, payload_json
         ) VALUES (?1, 'workflow.runtime.updated', ?2, ?3, ?4)",
        params![event_id, session_id, timestamp_ms as i64, payload_json],
    )?;
    let sequence = transaction.last_insert_rowid().max(0) as u64;
    write_workflow_runs(transaction, owner_id, timestamp_ms, workflow_runs, true)?;
    write_delivery_receipts(transaction, owner_id, timestamp_ms, delivery_receipts, true)?;
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
    workflow_runs: &[DurableWorkflowRunWrite],
    delivery_receipts: &[DurableDeliveryReceiptWrite],
) -> Result<u64, rusqlite::Error> {
    let timestamp_ms = unix_epoch_ms();
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
            payload_json = excluded.payload_json"
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
            payload_json = excluded.payload_json"
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
    use std::time::{SystemTime, UNIX_EPOCH};

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
        session
    }

    #[test]
    fn workflow_runtime_transition_separates_hot_state_from_run_history_and_receipts() {
        let (store, path) = temp_store("workflow-runtime-separation");
        let session = session_with_runs();

        store
            .persist_workflow_runtime_transition(&session, "test")
            .expect("workflow transition should persist");

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

        let event = store
            .load_events_by_kind("workflow.runtime.updated")
            .expect("workflow event should load")
            .pop()
            .expect("workflow event should exist");
        let hot_session: RuntimeSession = serde_json::from_value(event.payload["session"].clone())
            .expect("hot session should decode");
        assert_eq!(hot_session.workflow_runs().len(), 1);
        assert_eq!(hot_session.workflow_runs()[0].id(), "run-active");
        assert!(hot_session.workflow_event_delivery_receipts().is_empty());

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
}
