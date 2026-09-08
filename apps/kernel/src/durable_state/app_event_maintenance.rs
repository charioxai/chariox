//! Timer-owned bounded receipt maintenance and classification on the sole writer.
mod workflow;
pub(super) use workflow::{record_queue_removals_in, record_workflow_transition_in};

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::{
    error::DaemonError,
    runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped},
};
use chariox_app_runtime::app_outbox::{
    AppOutbox, EventCatalog, Maintenance, OutboxError, Receipt, ReceiptState, VerifiedAutomation,
    MAX_ATTEMPTS,
};
use rusqlite::{params, Connection, TransactionBehavior};
use std::sync::{mpsc, Arc};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppEventMaintenanceError {
    #[error(transparent)]
    Stopped(#[from] AppOperationStopped),
    #[error(transparent)]
    Outbox(#[from] OutboxError),
    #[error(transparent)]
    Storage(#[from] DaemonError),
}
impl From<rusqlite::Error> for AppEventMaintenanceError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Outbox(value.into())
    }
}
type Result<T> = std::result::Result<T, AppEventMaintenanceError>;

pub(crate) enum AppEventClassification {
    Retryable,
    Failed,
    Expired,
}
pub(crate) enum AppEventMaintenanceOperation {
    Sweep {
        owner: String,
        installation: String,
        durable_owner: String,
    },
    Classify {
        owner: String,
        catalog: Arc<EventCatalog>,
        receipt: Receipt,
        classification: AppEventClassification,
    },
}
#[derive(Debug)]
pub(crate) enum AppEventMaintenanceOutcome {
    Swept {
        maintenance: Maintenance,
        queued_sessions: Vec<String>,
    },
    Classified(Receipt),
}
pub(super) struct AppEventMaintenanceRequest {
    operation: AppEventMaintenanceOperation,
    budget: AppOperationBudget,
    response: mpsc::Sender<Result<AppEventMaintenanceOutcome>>,
}
impl std::fmt::Debug for AppEventMaintenanceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppEventMaintenanceRequest(..)")
    }
}
impl DurableKernelStateStore {
    pub(crate) fn maintain_app_events(
        &self,
        operation: AppEventMaintenanceOperation,
        budget: AppOperationBudget,
    ) -> Result<AppEventMaintenanceOutcome> {
        budget.check()?;
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppEventMaintenance(Box::new(
                AppEventMaintenanceRequest {
                    operation,
                    budget,
                    response,
                },
            )))?;
        receiver
            .recv()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "app.events.maintenance",
                message: error.to_string(),
            })?
    }
    /// Kernel-only discovery; page carries no effect authority. Includes inactive
    /// installations so uninstall/revocation cannot disable expiry or reclamation.
    pub(crate) fn app_event_installations_page(
        &self,
        after: Option<(&str, &str)>,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        if !(1..=16).contains(&limit) {
            return Err(OutboxError::Limit.into());
        }
        let connection = self.lock_connection("app.events.installations")?;
        let mut statement = connection.prepare(
            "SELECT owner_id,installation_id FROM app_installations
          WHERE (owner_id,installation_id)>(?1,?2) ORDER BY owner_id,installation_id LIMIT ?3",
        )?;
        let (owner, installation) = after.unwrap_or(("", ""));
        let rows = statement.query_map(params![owner, installation, limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub(crate) fn pending_app_events(
        &self,
        owner: &str,
        catalog: &EventCatalog,
        now: u64,
        limit: usize,
    ) -> Result<Vec<Receipt>> {
        let mut connection = self.lock_connection("app.events.pending")?;
        let tx = connection.transaction()?;
        Ok(AppOutbox::pending_in(&tx, catalog, owner, now, limit)?)
    }
}
pub(super) fn execute(connection: &mut Connection, request: AppEventMaintenanceRequest) {
    let result = apply(
        connection,
        request.operation,
        &request.budget,
        crate::session::unix_epoch_ms(),
    );
    let _ = request.response.send(result);
}
fn apply(
    connection: &mut Connection,
    operation: AppEventMaintenanceOperation,
    budget: &AppOperationBudget,
    now: u64,
) -> Result<AppEventMaintenanceOutcome> {
    budget.check()?;
    let mut tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    budget.check()?;
    let outcome = match operation {
        AppEventMaintenanceOperation::Sweep {
            owner,
            installation,
            durable_owner,
        } => {
            let maintenance = AppOutbox::maintain_in(&mut tx, &owner, &installation, now)?;
            let queued_sessions =
                workflow::reconcile_queued_in(&tx, &durable_owner, &owner, &installation)?;
            AppEventMaintenanceOutcome::Swept {
                maintenance,
                queued_sessions,
            }
        }
        AppEventMaintenanceOperation::Classify {
            owner,
            catalog,
            receipt,
            classification,
        } => {
            let value = match classification {
                AppEventClassification::Retryable => {
                    let automation = VerifiedAutomation::load_in(
                        &tx,
                        catalog.clone(),
                        &owner,
                        &receipt.automation_id,
                        receipt.automation_revision,
                    )?;
                    let delay = 1000u64
                        .saturating_mul(1u64 << receipt.attempts.min(6))
                        .min(60_000);
                    let retry_at = now.saturating_add(delay);
                    if receipt.attempts + 1 >= MAX_ATTEMPTS || retry_at >= receipt.expires_at_ms {
                        AppOutbox::mark_failed_attempt_in(
                            &tx,
                            &automation,
                            &receipt.receipt_id,
                            receipt.revision,
                            now,
                        )?
                    } else {
                        AppOutbox::mark_retryable_in(
                            &tx,
                            &automation,
                            &receipt.receipt_id,
                            receipt.revision,
                            retry_at,
                            now,
                        )?
                    }
                }
                AppEventClassification::Failed => AppOutbox::settle_in(
                    &tx,
                    &catalog,
                    &owner,
                    &receipt.receipt_id,
                    receipt.revision,
                    ReceiptState::Failed,
                    now,
                )?,
                AppEventClassification::Expired => AppOutbox::settle_in(
                    &tx,
                    &catalog,
                    &owner,
                    &receipt.receipt_id,
                    receipt.revision,
                    ReceiptState::Expired,
                    now,
                )?,
            };
            AppEventMaintenanceOutcome::Classified(value)
        }
    };
    budget.check()?;
    tx.commit()?;
    Ok(outcome)
}
