//! Explicit App automation configuration through existing workflow ownership.
//! This has no wire/SDK endpoint yet. A future App asset request must pass the
//! existing Ask/YOLO permission path before invoking these kernel operations.

use super::KernelRuntimeOwnedState;
use crate::runtime::app_operation_budget::AppOperationBudget;
use crate::{
    durable_state::app_automations::{
        AppAutomationError, AppAutomationMutation, AppAutomationOutcome, WorkflowAutomationTarget,
    },
    error::DaemonError,
};
use chariox_app_runtime::app_outbox::{AutomationConfiguration, AutomationStatus, EventCatalog};
use std::sync::Arc;

pub(super) struct ConfigureAppAutomation {
    pub automation_id: String,
    pub expected_revision: u64,
    pub event_name: String,
    pub session_id: String,
    pub publication_ref: String,
    pub queue_ref: Option<String>,
    pub scheduled: bool,
}
impl KernelRuntimeOwnedState {
    /// Blocking ownership operation; execute on the existing bounded kernel
    /// service. The authenticated caller is supplied separately from request data.
    pub(super) fn configure_app_automation(
        &self,
        trusted_owner: &str,
        catalog: Arc<EventCatalog>,
        request: ConfigureAppAutomation,
        budget: AppOperationBudget,
    ) -> Result<AutomationConfiguration, DaemonError> {
        budget.check().map_err(|e| error(e.into()))?;
        self.durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                budget.check().map_err(|e| error(e.into()))?;
                // Holding only the transition mutex is insufficient: several current
                // workflow mutations acquire SessionService directly. Retain this read
                // guard until the writer's exact-target check and config CAS commit.
                let sessions = self.session_store.read();
                let target = WorkflowAutomationTarget::resolve(
                    &sessions,
                    trusted_owner,
                    &request.session_id,
                    &request.publication_ref,
                    request.queue_ref.as_deref(),
                )
                .map_err(error)?;
                let result = self
                    .durable_state_store
                    .mutate_app_automation(
                        trusted_owner,
                        catalog,
                        AppAutomationMutation::Configure {
                            automation_id: request.automation_id,
                            expected_revision: request.expected_revision,
                            event_name: request.event_name,
                            target,
                            scheduled: request.scheduled,
                        },
                        budget,
                    )
                    .map_err(error)?;
                drop(sessions);
                match result {
                    AppAutomationOutcome::Configured(value) => Ok(value),
                    _ => Err(invalid_response()),
                }
            })
    }
    pub(super) fn deactivate_app_automation(
        &self,
        trusted_owner: &str,
        catalog: Arc<EventCatalog>,
        automation_id: String,
        expected_revision: u64,
        status: AutomationStatus,
        budget: AppOperationBudget,
    ) -> Result<AutomationConfiguration, DaemonError> {
        let result = self
            .durable_state_store
            .mutate_app_automation(
                trusted_owner,
                catalog,
                AppAutomationMutation::Deactivate {
                    automation_id,
                    expected_revision,
                    status,
                },
                budget,
            )
            .map_err(error)?;
        match result {
            AppAutomationOutcome::Configured(value) => Ok(value),
            _ => Err(invalid_response()),
        }
    }
    pub(super) fn list_app_automations(
        &self,
        trusted_owner: &str,
        catalog: Arc<EventCatalog>,
        budget: AppOperationBudget,
    ) -> Result<Vec<AutomationConfiguration>, DaemonError> {
        match self
            .durable_state_store
            .mutate_app_automation(trusted_owner, catalog, AppAutomationMutation::List, budget)
            .map_err(error)?
        {
            AppAutomationOutcome::Listed(values) => Ok(values),
            _ => Err(invalid_response()),
        }
    }
}
fn error(error: AppAutomationError) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "app.automation",
        message: error.to_string(),
    }
}
fn invalid_response() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "app.automation",
        message: "Unexpected App automation response".into(),
    }
}
