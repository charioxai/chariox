//! App automation configuration on the existing durable writer. Workflow targets
//! remain existing SessionService/normalized-workflow entities, not App metadata.

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;
use crate::runtime::app_operation_budget::AppOperationBudget;
use chariox_app_runtime::app_outbox::{
    AppOutbox, AutomationConfiguration, AutomationStatus, EventCatalog, OutboxError,
};
use rusqlite::{Connection, TransactionBehavior};
use std::sync::{mpsc, Arc};

pub(crate) use super::notification_target::NotificationTargetError as AppAutomationError;

pub(crate) use super::notification_target::WorkflowNotificationTarget as WorkflowAutomationTarget;

#[derive(Debug)]
pub(crate) enum AppAutomationMutation {
    Configure {
        automation_id: String,
        expected_revision: u64,
        event_name: String,
        target: WorkflowAutomationTarget,
        scheduled: bool,
        delivery_mode: crate::local::NotificationDeliveryMode,
    },
    Deactivate {
        automation_id: String,
        expected_revision: u64,
        status: AutomationStatus,
    },
    List,
}
#[derive(Debug)]
pub(crate) enum AppAutomationOutcome {
    Configured(AutomationConfiguration),
    Listed(Vec<AutomationConfiguration>),
}
pub(super) struct AppAutomationRequest {
    owner: String,
    catalog: Arc<EventCatalog>,
    mutation: AppAutomationMutation,
    budget: AppOperationBudget,
    response: mpsc::Sender<Result<AppAutomationOutcome, AppAutomationError>>,
}
impl std::fmt::Debug for AppAutomationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppAutomationRequest")
            .finish_non_exhaustive()
    }
}
impl DurableKernelStateStore {
    /// Blocking. Configure callers must keep the workflow target guards through
    /// this response; the writer never accesses SessionService or reacquires them.
    /// Inputs are an explicit authenticated kernel operation after existing asset
    /// permission policy, not an App request or an automatic permission grant.
    pub(crate) fn mutate_app_automation(
        &self,
        trusted_owner: &str,
        catalog: Arc<EventCatalog>,
        mutation: AppAutomationMutation,
        budget: AppOperationBudget,
    ) -> Result<AppAutomationOutcome, AppAutomationError> {
        budget.check()?;
        if trusted_owner.is_empty()
            || trusted_owner.len() > 128
            || trusted_owner
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '\u{feff}')
        {
            return Err(OutboxError::Invalid.into());
        }
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppAutomation(Box::new(
                AppAutomationRequest {
                    owner: trusted_owner.into(),
                    catalog,
                    mutation,
                    budget,
                    response,
                },
            )))?;
        receiver.recv().map_err(|_| DaemonError::LocalTransport {
            operation: "durable_state.await_app_automation",
            message: "App automation operation did not complete".into(),
        })?
    }
}
pub(super) fn execute(connection: &mut Connection, request: AppAutomationRequest) {
    let result = apply(
        connection,
        &request.owner,
        &request.catalog,
        request.mutation,
        &request.budget,
    );
    let _ = request.response.send(result);
}
pub(super) fn initialize(connection: &mut Connection) -> Result<(), DaemonError> {
    AppOutbox::initialize(connection).map_err(|error| DaemonError::LocalTransport {
        operation: "durable_state.migrate_app_automations",
        message: error.to_string(),
    })
}
fn apply(
    connection: &mut Connection,
    owner: &str,
    catalog: &EventCatalog,
    mutation: AppAutomationMutation,
    budget: &AppOperationBudget,
) -> Result<AppAutomationOutcome, AppAutomationError> {
    budget.check()?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // The FIFO and BEGIN IMMEDIATE may both wait. Admission is checked after
    // those waits; cancellation after admitted work does not undo its commit.
    budget.check()?;
    let result = match mutation {
        AppAutomationMutation::Configure {
            automation_id,
            expected_revision,
            event_name,
            target,
            scheduled,
            delivery_mode,
        } => {
            target.require_current(&tx, owner)?;
            budget.check()?;
            AppOutbox::configure_in(
                &tx,
                catalog,
                owner,
                &automation_id,
                expected_revision,
                &event_name,
                target.target(),
                scheduled,
            )?;
            tx.execute("UPDATE app_automations SET delivery_mode=?4 WHERE owner_id=?1 AND installation_id=?2 AND automation_id=?3",rusqlite::params![owner,catalog.installation_id(),automation_id,delivery_mode.name()])?;
            AppAutomationOutcome::Configured(AppOutbox::configuration_in(
                &tx,
                catalog,
                owner,
                &automation_id,
            )?)
        }
        AppAutomationMutation::Deactivate {
            automation_id,
            expected_revision,
            status,
        } => AppAutomationOutcome::Configured(AppOutbox::deactivate_in(
            &tx,
            catalog,
            owner,
            &automation_id,
            expected_revision,
            status,
        )?),
        AppAutomationMutation::List => {
            AppAutomationOutcome::Listed(AppOutbox::configurations_in(&tx, catalog, owner)?)
        }
    };
    tx.commit()?;
    Ok(result)
}

impl DurableKernelStateStore {
    /// Protocol 366: the owner's installations whose active automations feed a
    /// publication.
    pub(crate) fn app_installations_feeding_publication(
        &self,
        owner: &str,
        session_id: &str,
        publication_id: &str,
    ) -> Result<Vec<String>, DaemonError> {
        let connection = self.lock_connection("durable_state.app_automation_targets")?;
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT installation_id FROM app_automations WHERE owner_id=?1
                 AND session_id=?2 AND publication_id=?3 AND status='active'
                 ORDER BY installation_id",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map(
                rusqlite::params![owner, session_id, publication_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }
}

fn storage(error: rusqlite::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "read App automation targets",
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
use super::notification_target::encode;

#[cfg(test)]
use crate::{runtime::app_operation_budget::AppOperationStopped, session::SessionService};
