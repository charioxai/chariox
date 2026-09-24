//! App outbox queue preparation uses the existing workflow authority and writer.
//! Caller dispatch/projection happens only after this operation returns success.
use super::KernelRuntimeOwnedState;
use crate::{
    durable_state::{
        app_event_delivery::{AppEventDeliveryError, PreparedAppEvent},
        app_state::{AppStateOperation, AppStateOutcome},
    },
    runtime::app_operation_budget::AppOperationBudget,
};
use chariox_app_runtime::app_outbox::{EventCatalog, Receipt, ReceiptState};
use std::sync::Arc;

impl KernelRuntimeOwnedState {
    /// Blocking, retained by the existing bounded App service. A cancelled
    /// async request must retain that blocking ownership until the writer replies.
    /// The eventual delivery pump calls normal workflow dispatch after success.
    pub(super) fn queue_app_event(
        &self,
        owner: &str,
        catalog: Arc<EventCatalog>,
        receipt_id: &str,
        budget: AppOperationBudget,
    ) -> Result<Receipt, AppEventDeliveryError> {
        budget.check()?;
        self.durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                Ok((|| {
                    budget.check()?;
                    let mut sessions = self.session_store.write();
                    let candidate = self.durable_state_store.app_event_candidate(
                        owner,
                        catalog.clone(),
                        receipt_id,
                    )?;
                    if matches!(
                        candidate.receipt().state,
                        ReceiptState::Queued | ReceiptState::Delivered
                    ) {
                        // A query-only snapshot cannot acknowledge visible data from
                        // an uncertain commit. Round-trip the existing writer; its
                        // STOP fence rejects this path until authoritative restart.
                        return match self
                            .durable_state_store
                            .execute_app_state(
                                owner,
                                catalog,
                                AppStateOperation::Status {
                                    receipt_id: receipt_id.into(),
                                },
                                budget,
                            )
                            .map_err(|_| AppEventDeliveryError::CommitUnknown)?
                        {
                            AppStateOutcome::Receipt(receipt) => Ok(receipt),
                            _ => Err(AppEventDeliveryError::CommitUnknown),
                        };
                    }
                    let prepared = Arc::new(PreparedAppEvent::prepare(&mut sessions, candidate)?);
                    let receipt = self
                        .durable_state_store
                        .commit_app_event_queue(prepared.clone(), budget)?;
                    // Only a durably committed receipt permits the queue clone to become
                    // visible. Ordinary errors leave the original session untouched.
                    sessions.restore_session(prepared.session().clone());
                    Ok(receipt)
                })())
            })?
    }
}
