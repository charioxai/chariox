//! Due-wake reads and delivery outcomes on the sole durable writer. Wake
//! registration itself composes with App state in `app_state`.
use super::{DurableKernelStateStore, DurableWriterRequest};
use chariox_app_runtime::managed_state::{self, DueWake, StateError};
use rusqlite::Connection;
use std::sync::mpsc;

pub(crate) enum AppWakeOperation {
    Due {
        now_ms: u64,
        limit: usize,
    },
    Delivered(DueWake),
    /// Retried later, or dropped after the last attempt; the App's log
    /// records why either way.
    Failed {
        wake: DueWake,
        now_ms: u64,
        reason: String,
    },
    Postponed {
        wake: DueWake,
        until_ms: u64,
    },
}

#[derive(Debug, PartialEq)]
pub(crate) enum AppWakeOutcome {
    Due(Vec<DueWake>),
    Recorded,
}

pub(super) struct AppWakeRequest {
    operation: AppWakeOperation,
    response: mpsc::Sender<Result<AppWakeOutcome, StateError>>,
}
impl std::fmt::Debug for AppWakeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppWakeRequest(..)")
    }
}

impl DurableKernelStateStore {
    /// Due work keeps a worker busy through delivery/settlement, including the
    /// interval before its lease is touched. Fail closed if storage is unhealthy.
    pub(crate) fn has_due_app_wakes(&self, owner: &str, installation: &str, now_ms: u64) -> bool {
        (|| {
            self.require_writer_healthy().ok()?;
            let connection = self.lock_connection("durable_state.busy_app_wake").ok()?;
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM app_wakes WHERE owner_id=?1
                 AND installation_id=?2 AND next_attempt_at_ms<=?3)",
                    rusqlite::params![owner, installation, now_ms.min(i64::MAX as u64) as i64],
                    |row| row.get::<_, bool>(0),
                )
                .ok()
        })()
        .unwrap_or(true)
    }

    /// Wait for the durable deadline or a committed schedule change. The one
    /// scheduler reads again after notifications and periodically rechecks wall
    /// time so a clock correction cannot strand a monotonic sleep (#764).
    pub(crate) async fn wait_for_app_wake(&self) {
        loop {
            let store = self.clone();
            let next = tokio::task::spawn_blocking(move || {
                store
                    .require_writer_healthy()
                    .map_err(|_| StateError::Corrupt)?;
                let connection = store
                    .lock_connection("durable_state.next_app_wake")
                    .map_err(|_| StateError::Corrupt)?;
                managed_state::next_wake_at_ms(&connection, crate::session::unix_epoch_ms())
            })
            .await;
            let now = crate::session::unix_epoch_ms();
            let delay_ms = match next {
                Ok(Ok(Some(due))) if due <= now => return,
                Ok(Ok(Some(due))) => due.saturating_sub(now).min(1_000),
                _ => 1_000,
            };
            tokio::select! {
                _ = self.app_wake_changed.notified() => {},
                _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => {},
            }
        }
    }

    pub(crate) fn app_wakes(
        &self,
        operation: AppWakeOperation,
    ) -> Result<AppWakeOutcome, StateError> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppWake(Box::new(AppWakeRequest {
                operation,
                response,
            })))
            .map_err(|_| StateError::Corrupt)?;
        receiver.recv().map_err(|_| StateError::Corrupt)?
    }
}

pub(super) fn execute(connection: &mut Connection, request: AppWakeRequest) {
    let result = match request.operation {
        AppWakeOperation::Due { now_ms, limit } => {
            managed_state::due_wakes(connection, now_ms, limit).map(AppWakeOutcome::Due)
        }
        AppWakeOperation::Delivered(wake) => {
            managed_state::complete_wake(connection, &wake).map(|()| AppWakeOutcome::Recorded)
        }
        AppWakeOperation::Failed {
            wake,
            now_ms,
            reason,
        } => failed(connection, &wake, now_ms, &reason),
        AppWakeOperation::Postponed { wake, until_ms } => {
            managed_state::postpone_wake(connection, &wake, until_ms)
                .map(|()| AppWakeOutcome::Recorded)
        }
    };
    let _ = request.response.send(result);
}

fn failed(
    connection: &mut Connection,
    wake: &DueWake,
    now_ms: u64,
    reason: &str,
) -> Result<AppWakeOutcome, StateError> {
    let transaction = connection.transaction()?;
    let retried = managed_state::defer_wake(&transaction, wake, now_ms)?;
    let mut fields = serde_json::Map::new();
    fields.insert("wake_id".into(), wake.wake.id.clone().into());
    fields.insert("attempt".into(), (wake.attempts + 1).into());
    fields.insert("reason".into(), reason.into());
    super::app_logs::append_kernel_notice_in(
        &transaction,
        &wake.owner_id,
        &wake.installation_id,
        now_ms,
        if retried {
            "A due wake failed; it will be retried"
        } else {
            "A due wake failed too often and was dropped"
        },
        fields,
    )?;
    transaction.commit()?;
    Ok(AppWakeOutcome::Recorded)
}
