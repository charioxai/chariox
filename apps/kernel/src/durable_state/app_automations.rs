//! App automation configuration on the existing durable writer. Workflow targets
//! remain existing SessionService/normalized-workflow entities, not App metadata.

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped};
use crate::{
    error::DaemonError,
    session::{SessionService, WORKFLOW_PUBLICATION_KIND_EVENT_BASED},
};
use chariox_app_runtime::app_outbox::{
    AppOutbox, AutomationConfiguration, AutomationStatus, AutomationTarget, EventCatalog,
    OutboxError,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::sync::{mpsc, Arc};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppAutomationError {
    #[error(transparent)]
    Stopped(#[from] AppOperationStopped),
    #[error(transparent)]
    Outbox(#[from] OutboxError),
    #[error(transparent)]
    Storage(#[from] DaemonError),
    #[error("app_automation_target_changed")]
    TargetChanged,
    #[error("app_automation_target_invalid")]
    InvalidTarget,
    #[error("app_automation_not_owner")]
    NotOwner,
}
impl From<rusqlite::Error> for AppAutomationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Outbox(OutboxError::Database(error))
    }
}

/// Built only by resolving existing workflow assets for the authenticated owner.
/// The runtime keeps its SessionService read guard and workflow transition mutex
/// until the writer replies; the writer also checks the exact durable entities.
#[derive(Debug)]
pub(crate) struct WorkflowAutomationTarget {
    owner: String,
    durable_owner: String,
    target: AutomationTarget,
    entities: Vec<(&'static str, String, String)>,
}
impl WorkflowAutomationTarget {
    pub(crate) fn resolve(
        sessions: &SessionService,
        trusted_owner: &str,
        session_id: &str,
        publication_ref: &str,
        queue_ref: Option<&str>,
    ) -> Result<Self, AppAutomationError> {
        let session = sessions.get_session(session_id)?;
        let publication = sessions.resolve_workflow_publication_ref(session_id, publication_ref)?;
        if publication.created_by_user_id() != trusted_owner {
            return Err(AppAutomationError::NotOwner);
        }
        if !publication.enabled() || publication.kind() != WORKFLOW_PUBLICATION_KIND_EVENT_BASED {
            return Err(AppAutomationError::InvalidTarget);
        }
        let workflow = sessions.resolve_workflow_ref(session_id, publication.workflow_id())?;
        let endpoint = sessions.resolve_workflow_endpoint_ref(
            session_id,
            workflow.id(),
            publication.endpoint_id(),
        )?;
        if endpoint.owner_user_id() != trusted_owner {
            return Err(AppAutomationError::NotOwner);
        }
        sessions.validate_workflow_runnable(session_id, &workflow, &endpoint)?;
        let queue_id = sessions.resolve_workflow_prompt_queue_ref(
            session_id,
            workflow.id(),
            queue_ref.or(publication.queue_ref()).unwrap_or("default"),
        )?;
        let queue = session
            .workflow_prompt_queues()
            .iter()
            .find(|queue| queue.id() == queue_id)
            .ok_or(AppAutomationError::InvalidTarget)?;
        let entities = vec![
            (
                "publication",
                publication.id().to_owned(),
                encode(&publication)?,
            ),
            ("workflow", workflow.id().to_owned(), encode(&workflow)?),
            ("queue", queue.id().to_owned(), encode(queue)?),
        ];
        Ok(Self {
            owner: trusted_owner.into(),
            durable_owner: session.host_daemon_id().into(),
            target: AutomationTarget {
                session_id: session.id().into(),
                publication_id: publication.id().into(),
                endpoint_id: endpoint.id().into(),
                queue_id,
            },
            entities,
        })
    }
    fn require_current(&self, tx: &Transaction<'_>, owner: &str) -> Result<(), AppAutomationError> {
        if self.owner != owner {
            return Err(AppAutomationError::NotOwner);
        }
        for (kind, id, payload) in &self.entities {
            let current:Option<String>=tx.query_row("SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind=?3 AND entity_id=?4",
                rusqlite::params![self.durable_owner,self.target.session_id,kind,id],|row|row.get(0)).optional()?;
            if current.as_deref() != Some(payload) {
                return Err(AppAutomationError::TargetChanged);
            }
        }
        Ok(())
    }
}
fn encode(value: &impl serde::Serialize) -> Result<String, AppAutomationError> {
    let mut output = TargetEncoding {
        bytes: Vec::new(),
        limited: false,
    };
    if serde_json::to_writer(&mut output, value).is_err() {
        return Err(if output.limited {
            OutboxError::Limit.into()
        } else {
            AppAutomationError::InvalidTarget
        });
    }
    String::from_utf8(output.bytes).map_err(|_| AppAutomationError::InvalidTarget)
}
struct TargetEncoding {
    bytes: Vec<u8>,
    limited: bool,
}
impl std::io::Write for TargetEncoding {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|length| length > 1024 * 1024)
        {
            self.limited = true;
            return Err(std::io::Error::other(
                "App automation target exceeds encoded limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) enum AppAutomationMutation {
    Configure {
        automation_id: String,
        expected_revision: u64,
        event_name: String,
        target: WorkflowAutomationTarget,
        scheduled: bool,
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
        } => {
            target.require_current(&tx, owner)?;
            budget.check()?;
            AppAutomationOutcome::Configured(AppOutbox::configure_in(
                &tx,
                catalog,
                owner,
                &automation_id,
                expected_revision,
                &event_name,
                &target.target,
                scheduled,
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

#[cfg(test)]
mod tests;
