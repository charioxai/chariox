//! Explicit App automation configuration retains Room and publication authority
//! through the durable writer's exact-target and revision fences.

use super::KernelRuntimeOwnedState;
use crate::durable_state::app_automations::{
    AppAutomationError, AppAutomationMutation, AppAutomationOutcome, WorkflowAutomationTarget,
};
use crate::runtime::app_operation_budget::AppOperationBudget;
use chariox_app_runtime::app_outbox::{
    AutomationConfiguration, AutomationStatus, EventCatalog, OutboxError,
};
use std::sync::Arc;

pub(super) struct ConfigureAppAutomation {
    pub caller_user_id: String,
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
    ) -> Result<AutomationConfiguration, AppAutomationError> {
        budget.check()?;
        self.durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                Ok((|| {
                    budget.check()?;
                    // Holding only the transition mutex is insufficient: several
                    // current workflow mutations acquire SessionService directly.
                    // Retain this read guard until the writer's exact-target check
                    // and config CAS commit.
                    let sessions = self.session_store.read();
                    let target = WorkflowAutomationTarget::resolve(
                        &sessions,
                        trusted_owner,
                        &request.session_id,
                        &request.publication_ref,
                        request.queue_ref.as_deref(),
                    )?;
                    // The read guard protects both targets through the writer's CAS.
                    // A caller replacing a binding must also own its current target.
                    let previous_target = if request.caller_user_id != trusted_owner {
                        let room = sessions.get_session(&request.session_id)?;
                        let publication = sessions.resolve_workflow_publication_ref(
                            &request.session_id,
                            &request.publication_ref,
                        )?;
                        if !room.has_member(&request.caller_user_id)
                            || room.status() == crate::session::SessionStatus::Ended
                            || publication.created_by_user_id() != request.caller_user_id
                        {
                            return Err(AppAutomationError::Unauthorized);
                        }
                        let previous = self
                            .list_app_automations(
                                trusted_owner,
                                catalog.clone(),
                                budget.fork(|| false),
                            )?
                            .into_iter()
                            .find(|value| value.automation_id == request.automation_id);
                        match previous {
                            Some(previous) => Some(WorkflowAutomationTarget::authorize_existing(
                                &sessions,
                                trusted_owner,
                                &request.caller_user_id,
                                previous.target,
                            )?),
                            None if request.expected_revision != 0 => {
                                return Err(AppAutomationError::Unauthorized)
                            }
                            None => None,
                        }
                    } else {
                        None
                    };
                    let result = self.durable_state_store.mutate_app_automation(
                        trusted_owner,
                        catalog,
                        AppAutomationMutation::Configure {
                            automation_id: request.automation_id,
                            expected_revision: request.expected_revision,
                            event_name: request.event_name,
                            target,
                            scheduled: request.scheduled,
                            previous_target,
                        },
                        budget,
                    )?;
                    drop(sessions);
                    configured(result)
                })())
            })
            .map_err(AppAutomationError::Storage)?
    }
    pub(super) fn deactivate_app_automation(
        &self,
        trusted_owner: &str,
        catalog: Arc<EventCatalog>,
        automation_id: String,
        expected_revision: u64,
        status: AutomationStatus,
        budget: AppOperationBudget,
    ) -> Result<AutomationConfiguration, AppAutomationError> {
        configured(self.durable_state_store.mutate_app_automation(
            trusted_owner,
            catalog,
            AppAutomationMutation::Deactivate {
                automation_id,
                expected_revision,
                status,
            },
            budget,
        )?)
    }
    pub(super) fn list_app_automations(
        &self,
        trusted_owner: &str,
        catalog: Arc<EventCatalog>,
        budget: AppOperationBudget,
    ) -> Result<Vec<AutomationConfiguration>, AppAutomationError> {
        match self.durable_state_store.mutate_app_automation(
            trusted_owner,
            catalog,
            AppAutomationMutation::List,
            budget,
        )? {
            AppAutomationOutcome::Listed(values) => Ok(values),
            _ => Err(OutboxError::Invalid.into()),
        }
    }
}
fn configured(
    outcome: AppAutomationOutcome,
) -> Result<AutomationConfiguration, AppAutomationError> {
    match outcome {
        AppAutomationOutcome::Configured(value) => Ok(value),
        _ => Err(OutboxError::Invalid.into()),
    }
}
