//! Shared validated invocation transport. Runtime MCP and future App view action
//! dispatch use this after their existing authenticated operation-policy check.

use super::{AppWorkerError, AppWorkerLease, LiveWorker, Phase};
use chariox_app_runtime::{
    app_catalog::{CallerContext, CatalogError, OwnedValidatedToolCall},
    wire::Message,
    worker_peer::{CallResponse, PeerError, RequestSlot},
};
use rusqlite::Transaction;
use serde_json::Value;
use std::{sync::Arc, time::Duration};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppToolError {
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error("app_critical_validation_unavailable")]
    CriticalValidationUnavailable,
}

pub(crate) struct AppCallSlot {
    live: Arc<LiveWorker>,
    slot: RequestSlot,
}

/// A schema-checked request with its exact peer reservation and catalog. No
/// caller may substitute an identity, payload, deadline or other worker here.
pub(crate) struct PreparedAppToolCall {
    live: Arc<LiveWorker>,
    slot: RequestSlot,
    call: OwnedValidatedToolCall,
}

pub(crate) struct AppToolResponse {
    live: Arc<LiveWorker>,
    call: OwnedValidatedToolCall,
    response: CallResponse,
}

/// After transport completes, the existing durable writer consumes this to
/// validate current signer/generation and output schema before exposing a result.
pub(crate) struct AppToolReply {
    live: Arc<LiveWorker>,
    call: OwnedValidatedToolCall,
    response: Message,
}

impl AppWorkerLease {
    pub(crate) fn reserve_call(&self, timeout: Duration) -> Result<AppCallSlot, AppWorkerError> {
        self.0.available()?;
        let slot = self.0.peer.reserve(timeout).map_err(peer_error)?;
        Ok(AppCallSlot {
            live: self.0.clone(),
            slot,
        })
    }
}

impl AppCallSlot {
    /// Preflight only: authority and schema are checked again on the writer.
    pub(crate) fn validate_input(&self, name: &str, input: &Value) -> Result<(), AppToolError> {
        self.live
            .catalog
            .app_catalog()
            .validate_tool_input(name, input)?;
        Ok(())
    }

    /// Run on the existing writer while common binding/lifecycle admission is
    /// retained. It checks trust and schema; it creates no authorization grant.
    pub(crate) fn prepare(
        self,
        tx: &Transaction<'_>,
        name: &str,
        input: Value,
        caller: &CallerContext,
        now_ms: u64,
    ) -> Result<PreparedAppToolCall, AppToolError> {
        let catalog = self.live.catalog.app_catalog();
        // Critical effects must wait for the shared exact-effect approval broker.
        // An App binding is never equivalent to human validation of an operation.
        if catalog
            .tool(name)
            .and_then(|tool| tool.action.as_ref())
            .and_then(|action| action.critical_validation.as_ref())
            .is_some()
        {
            return Err(AppToolError::CriticalValidationUnavailable);
        }
        let call = catalog
            .prepare(
                tx,
                &self.live.owner,
                name,
                input,
                caller,
                self.slot.id(),
                now_ms,
                self.slot.deadline_ms(),
            )?
            .into_owned(catalog.clone())?;
        Ok(PreparedAppToolCall {
            live: self.live,
            slot: self.slot,
            call,
        })
    }
}

impl PreparedAppToolCall {
    /// The caller retains its existing binding-policy guard through this call,
    /// then releases it before awaiting the App. Owner stop shares this gate.
    pub(crate) fn enqueue(self) -> Result<AppToolResponse, AppWorkerError> {
        let phase = self
            .live
            .admission
            .phase
            .lock()
            .map_err(|_| AppWorkerError::Unavailable)?;
        if *phase != Phase::Active || self.live.peer.is_closed() {
            return Err(AppWorkerError::Unavailable);
        }
        let response = self
            .slot
            .submit(self.call.request().clone())
            .map_err(peer_error)?;
        drop(phase);
        Ok(AppToolResponse {
            live: self.live,
            call: self.call,
            response,
        })
    }
}

impl AppToolResponse {
    /// Dropping this future closes its response receiver and asks the one peer
    /// to cancel. This does not attest that a remote effect was rolled back.
    pub(crate) async fn receive(self) -> Result<AppToolReply, AppWorkerError> {
        let response = self.response.await.map_err(peer_error)?;
        self.live.available()?;
        Ok(AppToolReply {
            live: self.live,
            call: self.call,
            response,
        })
    }
}

impl AppToolReply {
    pub(crate) fn accept(self, tx: &Transaction<'_>, now_ms: u64) -> Result<Value, CatalogError> {
        self.live.available().map_err(|_| CatalogError::Stale)?;
        self.call
            .accept(tx, &self.live.owner, self.response, now_ms)
    }
}

fn peer_error(error: PeerError) -> AppWorkerError {
    match error {
        PeerError::Busy => AppWorkerError::Busy,
        PeerError::Deadline => AppWorkerError::Deadline,
        PeerError::Invalid => AppWorkerError::Invalid,
        _ => AppWorkerError::Unavailable,
    }
}
