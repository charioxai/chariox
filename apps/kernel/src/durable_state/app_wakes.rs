//! Due-wake reads and delivery outcomes on the sole durable writer. Wake
//! registration itself composes with App state in `app_state`.
use super::{DurableKernelStateStore, DurableWriterRequest};
use chariox_app_runtime::managed_state::{self, DueWake, StateError};
use rusqlite::Connection;
use std::sync::mpsc;

pub(crate) enum AppWakeOperation {
    Due { now_ms: u64, limit: usize },
    Delivered(DueWake),
    Failed { wake: DueWake, now_ms: u64 },
    NextDue,
}

#[derive(Debug, PartialEq)]
pub(crate) enum AppWakeOutcome {
    Due(Vec<DueWake>),
    Recorded,
    NextDue(Option<u64>),
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
    pub(crate) fn app_wakes(&self, operation: AppWakeOperation) -> Result<AppWakeOutcome, StateError> {
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
        AppWakeOperation::Failed { wake, now_ms } => {
            managed_state::defer_wake(connection, &wake, now_ms).map(|_| AppWakeOutcome::Recorded)
        }
        AppWakeOperation::NextDue => managed_state::next_wake_at(connection).map(AppWakeOutcome::NextDue),
    };
    let _ = request.response.send(result);
}
