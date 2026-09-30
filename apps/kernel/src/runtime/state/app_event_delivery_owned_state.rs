//! App outbox queue preparation uses the existing workflow authority and writer.
//! The session projection is published here after a durable commit; the caller
//! dispatches only after this operation returns success.
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
    /// Like any workflow queue admission, a new queued prompt is published to the
    /// session read projection: a paused queue or busy endpoint dispatches nothing
    /// that would publish it later.
    pub(super) fn queue_app_event(
        &self,
        owner: &str,
        catalog: Arc<EventCatalog>,
        receipt_id: &str,
        budget: AppOperationBudget,
    ) -> Result<Receipt, AppEventDeliveryError> {
        budget.check()?;
        let activity_mutation = self.begin_managed_activity_mutation();
        let (receipt, queued_session) = self
            .durable_state_store
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
                            AppStateOutcome::Receipt(receipt) => Ok((receipt, None)),
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
                    Ok((receipt, Some(prepared.session().id().to_owned())))
                })())
            })??;
        if let Some(session_id) = queued_session {
            activity_mutation.record();
            // Publication follows the durable commit and activity capture. The
            // receipt is already queued, so a failed publication does not undo it.
            let _ = self.session_snapshot(&session_id);
        }
        Ok(receipt)
    }
}
