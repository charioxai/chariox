//! Atomic App occurrence -> existing workflow queue handoff. Runtime callers hold
//! the workflow transition mutex and SessionService write guard until completion.
mod preparation;

use super::workflow_runtime::{write_workflow_runtime_transition, WorkflowRuntimeTransitionWrite};
use super::{DurableKernelStateStore, DurableWorkflowSessionWrite, DurableWriterRequest};
use crate::{
    error::DaemonError,
    runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped},
};
use chariox_app_runtime::app_outbox::{
    AppOutbox, AutomationConfiguration, EventCatalog, OutboxError, Receipt, ReceiptState,
    VerifiedAutomation,
};
pub(crate) use preparation::PreparedAppEvent;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::sync::{mpsc, Arc};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppEventDeliveryError {
    #[error(transparent)]
    Stopped(#[from] AppOperationStopped),
    #[error(transparent)]
    Outbox(#[from] OutboxError),
    #[error(transparent)]
    Workflow(#[from] DaemonError),
    #[error(transparent)]
    Target(#[from] super::app_automations::AppAutomationError),
    #[error("app_event_queue_conflict")]
    Conflict,
    #[error("app_event_queue_limit")]
    Limit,
    /// The caller must not claim rollback or dispatch; durable writer recovery
    /// must succeed before this kernel can continue the affected transition.
    #[error("app_event_queue_commit_unknown")]
    CommitUnknown,
}
impl From<rusqlite::Error> for AppEventDeliveryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Outbox(error.into())
    }
}
type Result<T> = std::result::Result<T, AppEventDeliveryError>;

pub(crate) struct AppEventCandidate {
    owner: String,
    catalog: Arc<EventCatalog>,
    receipt: Receipt,
    configuration: AutomationConfiguration,
}
impl AppEventCandidate {
    pub(crate) fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    pub(crate) fn session_id(&self) -> &str {
        &self.configuration.target.session_id
    }
}

pub(super) struct AppEventQueueRequest {
    prepared: Arc<PreparedAppEvent>,
    budget: AppOperationBudget,
    response: mpsc::Sender<Result<Receipt>>,
}
impl std::fmt::Debug for AppEventQueueRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppEventQueueRequest")
            .finish_non_exhaustive()
    }
}
impl DurableKernelStateStore {
    /// Scoped, provisional discovery on the existing query connection. Queueing
    /// repeats every receipt, automation, target and signer check on the writer.
    pub(crate) fn app_event_candidate(
        &self,
        owner: &str,
        catalog: Arc<EventCatalog>,
        receipt_id: &str,
    ) -> Result<AppEventCandidate> {
        let mut connection = self.lock_connection("durable_state.app_event_candidate")?;
        let tx = connection.transaction()?;
        let receipt = AppOutbox::status_in(&tx, &catalog, owner, receipt_id)?;
        let configuration =
            AppOutbox::configuration_in(&tx, &catalog, owner, &receipt.automation_id)?;
        Ok(AppEventCandidate {
            owner: owner.into(),
            catalog,
            receipt,
            configuration,
        })
    }

    /// Blocking. The ownership guards and admission reservation must outlive this
    /// reply, including cancellation of an outer async request. No dispatch occurs.
    pub(crate) fn commit_app_event_queue(
        &self,
        prepared: Arc<PreparedAppEvent>,
        budget: AppOperationBudget,
    ) -> Result<Receipt> {
        budget.check()?;
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppEventQueue(Box::new(
                AppEventQueueRequest {
                    prepared,
                    budget,
                    response,
                },
            )))?;
        // A disappeared writer cannot attest fsync merely from a visible read.
        // Normal SQLite commit errors are reconciled by execute before replying.
        receiver
            .recv()
            .map_err(|_| AppEventDeliveryError::CommitUnknown)?
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WriterDisposition {
    Continue,
    Stop,
}

/// Stop the sole writer on irreconcilable uncertainty. Dropping its receiver
/// rejects pending/future ordinary writes before they can overwrite the queue.
pub(super) fn execute(
    connection: &mut Connection,
    request: AppEventQueueRequest,
) -> WriterDisposition {
    let result = apply(connection, &request.prepared, &request.budget);
    let disposition = if matches!(&result, Err(AppEventDeliveryError::CommitUnknown)) {
        WriterDisposition::Stop
    } else {
        WriterDisposition::Continue
    };
    let _ = request.response.send(result);
    disposition
}

fn apply(
    connection: &mut Connection,
    prepared: &PreparedAppEvent,
    budget: &AppOperationBudget,
) -> Result<Receipt> {
    budget.check()?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    budget.check()?;
    let candidate = &prepared.candidate;
    prepared.target.require_current(&tx, &candidate.owner)?;
    require_hot_state(&tx, prepared.after.host_daemon_id(), &prepared.before)?;
    let automation = VerifiedAutomation::load_in(
        &tx,
        candidate.catalog.clone(),
        &candidate.owner,
        &candidate.receipt.automation_id,
        candidate.receipt.automation_revision,
    )?;
    // Exact receipt revision + automation CAS are checked by mark_queued_in.
    let receipt = AppOutbox::mark_queued_in(
        &tx,
        &automation,
        &candidate.receipt.receipt_id,
        candidate.receipt.revision,
        &prepared.queued_id,
        crate::session::unix_epoch_ms(),
    )?;
    budget.check()?;
    write(&tx, prepared, false)?;
    match tx.commit() {
        Ok(()) => {
            #[cfg(test)]
            if let Some(fault) = &prepared.fault_after_commit {
                fault.entered.send(()).unwrap();
                fault
                    .release
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                return Err(AppEventDeliveryError::CommitUnknown);
            }
            Ok(receipt)
        }
        Err(error) => reconcile_commit(connection, prepared, error),
    }
}
fn write(tx: &Transaction<'_>, prepared: &PreparedAppEvent, recovery: bool) -> Result<()> {
    let now = crate::session::unix_epoch_ms();
    let event_id = format!("state_evt_{now}_{}", super::rand_suffix());
    let metadata=serde_json::json!({"owner_id":prepared.after.host_daemon_id(),"session_id":prepared.after.id(),
        "reason":if recovery {"app_event_queue_reconciled"} else {"app_event_queued"},
        "receipt_id":prepared.candidate.receipt.receipt_id}).to_string();
    write_workflow_runtime_transition(
        tx,
        WorkflowRuntimeTransitionWrite {
            event_id: &event_id,
            event_kind: "workflow.runtime.updated",
            timestamp_ms: now,
            payload_json: &metadata,
            owner_id: prepared.after.host_daemon_id(),
            session_id: prepared.after.id(),
            hot_entities: &prepared.encoded.hot_entities,
            workflow_runs: &prepared.encoded.workflow_runs,
            delivery_receipts: &prepared.encoded.delivery_receipts,
        },
    )?;
    Ok(())
}
fn reconcile_commit(
    connection: &mut Connection,
    prepared: &PreparedAppEvent,
    original: rusqlite::Error,
) -> Result<Receipt> {
    // A successful second FULL-synchronous writer commit is the durability fence;
    // reading a possibly-visible WAL frame alone never authorizes projection.
    let mut attempt = || -> Result<Option<Receipt>> {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidate = &prepared.candidate;
        let receipt = AppOutbox::status_in(
            &tx,
            &candidate.catalog,
            &candidate.owner,
            &candidate.receipt.receipt_id,
        )?;
        if receipt.state != ReceiptState::Queued
            || receipt.queued_prompt_id.as_deref() != Some(&prepared.queued_id)
        {
            if receipt == candidate.receipt {
                tx.commit()?;
                return Ok(None);
            }
            return Err(AppEventDeliveryError::CommitUnknown);
        }
        require_hot_state(&tx, prepared.after.host_daemon_id(), &prepared.encoded)?;
        write(&tx, prepared, true)?;
        tx.commit()?;
        Ok(Some(receipt))
    };
    match attempt() {
        Ok(Some(receipt)) => Ok(receipt),
        Ok(None) => Err(original.into()),
        Err(_) => Err(AppEventDeliveryError::CommitUnknown),
    }
}
fn require_hot_state(
    tx: &Transaction<'_>,
    owner: &str,
    expected: &DurableWorkflowSessionWrite,
) -> Result<()> {
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2",
        rusqlite::params![owner, expected.session_id],
        |row| row.get(0),
    )?;
    if usize::try_from(count).ok() != Some(expected.hot_entities.len()) {
        return Err(AppEventDeliveryError::Conflict);
    }
    for entity in &expected.hot_entities {
        let current:Option<String>=tx.query_row("SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind=?3 AND entity_id=?4",
            rusqlite::params![owner,expected.session_id,entity.entity_kind,entity.entity_id],|row|row.get(0)).optional()?;
        if current.as_deref() != Some(&entity.payload_json) {
            return Err(AppEventDeliveryError::Conflict);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
