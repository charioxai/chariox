//! Atomic App occurrence -> existing workflow queue handoff. Runtime callers hold
//! the workflow transition mutex and SessionService write guard until completion.
mod preparation;
pub(super) mod workflow_completion;
pub(crate) use workflow_completion::PreparedNotification;

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
    #[error("app_event_automation_changed")]
    AutomationChanged,
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
        super::storage_full::observe(&error);
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
    /// Corroborate the source-neutral durable admission before steering. Client
    /// publication envelopes alone never authorize notification injection.
    pub(crate) fn notification_queue_is_owned(
        &self,
        session: &crate::session::RuntimeSession,
        queued: &crate::session::WorkflowQueuedPrompt,
    ) -> std::result::Result<bool, DaemonError> {
        let Some(inv) = queued.publication_invocation() else {
            return Ok(false);
        };
        let connection = self.lock_connection("notification.delivery.authority")?;
        connection.query_row("SELECT EXISTS(SELECT 1 FROM app_outbox WHERE receipt_id=?1 AND owner_id=?2 AND queued_session_id=?3 AND queued_prompt_id=?4 AND state='queued' AND source_kind IN ('app_event','workflow_completion'))",
            rusqlite::params![inv.invocation_id,inv.caller.get("owner_id").and_then(serde_json::Value::as_str).unwrap_or(""),session.id(),queued.id()], |r| r.get(0))
            .map_err(|e| super::workflow_notifications::error(e.to_string()))
    }

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
        // A full disk rolled the commit back: nothing to reconcile.
        Err(error) if super::storage_full::observe(&error) => Err(error.into()),
        Err(error) => reconcile_commit(connection, prepared, error),
    }
}
fn write(tx: &Transaction<'_>, prepared: &PreparedAppEvent, recovery: bool) -> Result<()> {
    write_queue_state_in(
        tx,
        &prepared.after,
        &prepared.encoded,
        if recovery {
            "app_event_queue_reconciled"
        } else {
            "app_event_queued"
        },
        &prepared.candidate.receipt.receipt_id,
    )?;
    Ok(())
}
/// One ordinary queue persistence path. Source adapters retain their receipt,
/// ownership and CAS checks and call this inside the same writer transaction.
pub(super) fn write_queue_state_in(
    tx: &Transaction<'_>,
    after: &crate::session::RuntimeSession,
    encoded: &DurableWorkflowSessionWrite,
    reason: &str,
    receipt_id: &str,
) -> std::result::Result<(), DaemonError> {
    let now = crate::session::unix_epoch_ms();
    let metadata=serde_json::json!({"owner_id":after.host_daemon_id(),"session_id":after.id(),"reason":reason,"receipt_id":receipt_id}).to_string();
    write_workflow_runtime_transition(
        tx,
        WorkflowRuntimeTransitionWrite {
            event_id: &format!("state_evt_{now}_{}", super::rand_suffix()),
            event_kind: "workflow.runtime.updated",
            timestamp_ms: now,
            payload_json: &metadata,
            owner_id: after.host_daemon_id(),
            source_owner_id: after.owner_user_id(),
            session_id: after.id(),
            hot_entities: &encoded.hot_entities,
            workflow_runs: &encoded.workflow_runs,
            delivery_receipts: &encoded.delivery_receipts,
            prompt_state_jsons: &[],
        },
    )
    .map(|_| ())
    .map_err(|e| DaemonError::LocalTransport {
        operation: "notification queue commit",
        message: e.to_string(),
    })
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
    if super::workflow_runtime::hot_state_matches(tx, owner, expected)? {
        Ok(())
    } else {
        Err(AppEventDeliveryError::Conflict)
    }
}

#[cfg(test)]
mod tests;
