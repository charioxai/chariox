//! Generic durable queue -> Ready run transition, before provider scheduling.
use super::workflow_runtime::{
    encode_workflow_session, hot_state_matches, write_workflow_runtime_transition,
    WorkflowRuntimeTransitionWrite,
};
use super::{DurableKernelStateStore, DurableWorkflowSessionWrite, DurableWriterRequest};
use crate::{error::DaemonError, session::PreparedWorkflowQueueRun};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::sync::{mpsc, Arc};
#[derive(Debug, thiserror::Error)]
pub(crate) enum WorkflowQueueStartError {
    #[error(transparent)]
    Storage(#[from] DaemonError),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("workflow_queue_snapshot_conflict")]
    Conflict,
    #[error("workflow_queue_commit_unknown")]
    CommitUnknown,
}
type Result<T> = std::result::Result<T, WorkflowQueueStartError>;
struct Encoded {
    prepared: Arc<PreparedWorkflowQueueRun>,
    before: DurableWorkflowSessionWrite,
    after: DurableWorkflowSessionWrite,
}
pub(super) struct WorkflowQueueStartRequest {
    encoded: Encoded,
    response: mpsc::Sender<Result<()>>,
}
impl std::fmt::Debug for WorkflowQueueStartRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WorkflowQueueStartRequest(..)")
    }
}
impl DurableKernelStateStore {
    pub(crate) fn commit_workflow_queue_start(
        &self,
        prepared: Arc<PreparedWorkflowQueueRun>,
    ) -> Result<()> {
        let before = encode_workflow_session(prepared.before())?;
        let after = encode_workflow_session(prepared.after())?;
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::WorkflowQueueStart(Box::new(
                WorkflowQueueStartRequest {
                    encoded: Encoded {
                        prepared,
                        before,
                        after,
                    },
                    response,
                },
            )))?;
        receiver
            .recv()
            .map_err(|_| WorkflowQueueStartError::CommitUnknown)?
    }
}
pub(super) fn execute(
    connection: &mut Connection,
    request: WorkflowQueueStartRequest,
) -> super::app_event_delivery::WriterDisposition {
    let result = apply(connection, &request.encoded);
    let disposition = if matches!(&result, Err(WorkflowQueueStartError::CommitUnknown)) {
        super::app_event_delivery::WriterDisposition::Stop
    } else {
        super::app_event_delivery::WriterDisposition::Continue
    };
    let _ = request.response.send(result);
    disposition
}
fn apply(connection: &mut Connection, encoded: &Encoded) -> Result<()> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let owner = encoded.prepared.before().host_daemon_id();
    if !hot_state_matches(&tx, owner, &encoded.before)? {
        return Err(WorkflowQueueStartError::Conflict);
    }
    write(&tx, encoded)?;
    match tx.commit() {
        Ok(()) => Ok(()),
        Err(original) => reconcile(connection, encoded, original),
    }
}
fn write(tx: &Transaction<'_>, encoded: &Encoded) -> Result<()> {
    let session = encoded.prepared.after();
    let now = crate::session::unix_epoch_ms();
    if let Some((queued, run, _, _)) = encoded.prepared.next() {
        super::workflow_dispatch_intents::insert_in(
            tx,
            session.host_daemon_id(),
            session.id(),
            queued,
            run,
        )?;
    }
    write_workflow_runtime_transition(tx,WorkflowRuntimeTransitionWrite {
        event_id:&format!("workflow_queue_{}_{:016x}",now,rand::random::<u64>()),event_kind:"workflow.runtime.updated",timestamp_ms:now,
        payload_json:&serde_json::json!({"owner_id":session.host_daemon_id(),"session_id":session.id(),"reason":"workflow_queue_run_created"}).to_string(),
        owner_id:session.host_daemon_id(),session_id:session.id(),hot_entities:&encoded.after.hot_entities,workflow_runs:&encoded.after.workflow_runs,delivery_receipts:&encoded.after.delivery_receipts,
    })?;
    Ok(())
}
fn reconcile(
    connection: &mut Connection,
    encoded: &Encoded,
    original: rusqlite::Error,
) -> Result<()> {
    let result = (|| -> Result<bool> {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner = encoded.prepared.after().host_daemon_id();
        if hot_state_matches(&tx, owner, &encoded.after)? {
            // Visible pages alone do not prove the failed commit was durable.
            write(&tx, encoded)?;
            tx.commit()?;
            return Ok(true);
        }
        if hot_state_matches(&tx, owner, &encoded.before)? {
            tx.commit()?;
            return Ok(false);
        }
        Err(WorkflowQueueStartError::CommitUnknown)
    })();
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(original.into()),
        Err(_) => Err(WorkflowQueueStartError::CommitUnknown),
    }
}
