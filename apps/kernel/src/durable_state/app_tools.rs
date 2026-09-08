//! Ordinary App tools use the same durable writer for final signer/generation
//! admission. No durable effect intent is created here; critical effects remain
//! unavailable until their shared human-validation broker can record one.

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped};
use crate::{
    error::DaemonError,
    runtime::app_worker::{
        AppCallSlot, AppToolError, AppToolReply, AppToolResponse, AppWorkerError,
    },
};
use chariox_app_runtime::{
    app_catalog::{CallerContext, CatalogError},
    app_outbox::EventCatalog,
};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::Value;
use std::sync::{mpsc, Arc};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppToolsError {
    #[error(transparent)]
    Stopped(#[from] AppOperationStopped),
    #[error(transparent)]
    Tool(#[from] AppToolError),
    #[error(transparent)]
    Worker(#[from] AppWorkerError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error("app_tools_database")]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Storage(#[from] DaemonError),
}
type Result<T> = std::result::Result<T, AppToolsError>;

pub(super) struct AppToolsRequest {
    slot: AppCallSlot,
    name: String,
    input: Value,
    caller: CallerContext,
    budget: AppOperationBudget,
    response: mpsc::Sender<Result<AppToolResponse>>,
}
impl std::fmt::Debug for AppToolsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppToolsRequest").finish_non_exhaustive()
    }
}

impl DurableKernelStateStore {
    /// Blocking; caller retains the shared App admission permit and the existing
    /// AgentStore binding read guard through this response, then releases the
    /// guard before waiting for App code. The slot owns its original deadline,
    /// exact worker/catalog, and transport reservation; request data cannot pick
    /// any of those identities. A disconnected response cancels through its Drop.
    pub(crate) fn enqueue_app_tool(
        &self,
        slot: AppCallSlot,
        name: &str,
        input: Value,
        caller: CallerContext,
        budget: AppOperationBudget,
    ) -> Result<AppToolResponse> {
        budget.check()?;
        slot.validate_input(name, &input)?;
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppTools(Box::new(AppToolsRequest {
                slot,
                name: name.into(),
                input,
                caller,
                budget,
                response,
            })))?;
        receiver.recv().map_err(|_| DaemonError::LocalTransport {
            operation: "app.tools.enqueue",
            message: "App tool admission did not complete".into(),
        })?
    }

    /// Result projection repeats current trust/output validation. This read does
    /// not attest that cancellation or a discarded result undid an external effect.
    pub(crate) fn accept_app_tool_reply(&self, reply: AppToolReply) -> Result<Value> {
        let mut connection = self.lock_connection("app.tools.result")?;
        let tx = connection.transaction()?;
        Ok(reply.accept(&tx, crate::session::unix_epoch_ms())?)
    }

    /// Bounded discovery only. A caller supplies catalogs from actual live
    /// worker leases and intersects its current App grants before publication.
    pub(crate) fn current_app_catalogs(
        &self,
        owner: &str,
        catalogs: &[Arc<EventCatalog>],
    ) -> Result<Vec<Arc<EventCatalog>>> {
        if catalogs.len() > 64 {
            return Err(CatalogError::Limit.into());
        }
        let mut connection = self.lock_connection("app.tools.catalog")?;
        let tx = connection.transaction()?;
        Ok(catalogs
            .iter()
            .filter(|catalog| catalog.app_catalog().require_current(&tx, owner).is_ok())
            .cloned()
            .collect())
    }
}

pub(super) fn execute(connection: &mut Connection, request: AppToolsRequest) {
    let result = (|| {
        request.budget.check()?;
        // IMMEDIATE reserves the existing writer's installation/trust order until
        // synchronous enqueue. This transaction deliberately has no SQL mutation:
        // dropping it rolls back only the read reservation, never App execution.
        // There is no fallible commit after enqueue and no automatic effect retry.
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        request.budget.check()?;
        let call = request.slot.prepare(
            &tx,
            &request.name,
            request.input,
            &request.caller,
            crate::session::unix_epoch_ms(),
        )?;
        request.budget.check()?;
        Ok(call.enqueue()?)
    })();
    let _ = request.response.send(result);
}
