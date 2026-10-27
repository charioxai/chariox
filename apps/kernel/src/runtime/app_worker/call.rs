//! Shared validated invocation transport. Runtime MCP and future App view action
//! dispatch use this after their existing authenticated operation-policy check.

use super::{AppWorkerError, AppWorkerLease, DeliveryError, LiveWorker, Phase};
use chariox_app_runtime::{
    app_catalog::{CallerContext, CatalogError, OwnedValidatedToolCall},
    wire::Message,
    worker_peer::{CallResponse, PeerError, RequestSlot},
};
use rusqlite::Transaction;
use serde_json::{json, Value};
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
        self.touch();
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
            return Err(self.live.admission.closed_error());
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
        let response = match self.response.await {
            Ok(response) => response,
            Err(error) => return Err(self.live.lost(peer_error(error)).await),
        };
        self.live.available()?;
        Ok(AppToolReply {
            live: self.live,
            call: self.call,
            response,
        })
    }
}

impl AppToolReply {
    /// Time left before the call's deadline; after it the answer is refused.
    pub(crate) fn remaining(&self, now_ms: u64) -> Duration {
        match self.call.request() {
            Message::Request { deadline_ms, .. } => {
                Duration::from_millis(deadline_ms.saturating_sub(now_ms))
            }
            _ => Duration::ZERO,
        }
    }
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

/// The handler's own error, bounded: it may reach the App's log.
fn handler_error(error: &chariox_app_runtime::wire::RemoteError) -> DeliveryError {
    let text = format!("{}: {}", error.code, error.message);
    DeliveryError::Handler(text.chars().take(240).collect())
}

impl AppWorkerLease {
    /// Deliver one kernel-owned wake. Success means the App's wake handler
    /// returned; delivery is at least once and never implies an external effect.
    /// Only a wake armed during a tool call or an inbound event is use; after
    /// one the App armed itself, a worker with no other use may stop.
    pub(crate) async fn deliver_wake(
        &self,
        wake: &chariox_app_runtime::managed_state::Wake,
        overdue: bool,
        counts_as_use: bool,
        timeout: Duration,
    ) -> Result<(), DeliveryError> {
        let _delivering = self
            .0
            .admission
            .admit_delivery()
            .map_err(|_| DeliveryError::NotAdmitted)?;
        if self.0.peer.is_closed() {
            return Err(DeliveryError::NotAdmitted);
        }
        if counts_as_use {
            self.touch();
        }
        let slot = self.0.peer.reserve(timeout).map_err(peer_error)?;
        let (params, context) = wake_request(self.catalog().installation_id(), wake, overdue);
        let response = slot.request("schedule.wake", params, Some(context)).await;
        if !counts_as_use {
            // Marked after the handler; until then `_delivering` holds off
            // an idle stop or eviction.
            self.0.residency.self_woken();
        }
        match response.map_err(peer_error)? {
            Message::Response {
                outcome: chariox_app_runtime::wire::Outcome::Success(_),
                ..
            } => Ok(()),
            Message::Response {
                outcome: chariox_app_runtime::wire::Outcome::Failure(failure),
                ..
            } => Err(handler_error(&failure.error)),
            _ => Err(AppWorkerError::Unavailable.into()),
        }
    }

    /// Deliver one accepted inbox occurrence to the App's incoming event
    /// handler. Retries keep the occurrence ID, so the App can deduplicate.
    pub(crate) async fn deliver_event(
        &self,
        item: &chariox_app_runtime::app_inbox::InboxItem,
        timeout: Duration,
    ) -> Result<(), DeliveryError> {
        self.0.available()?;
        self.touch();
        let slot = self.0.peer.reserve(timeout).map_err(peer_error)?;
        let (params, context) = event_request(item);
        match slot
            .request("events.deliver", params, Some(context))
            .await
            .map_err(peer_error)?
        {
            Message::Response {
                outcome: chariox_app_runtime::wire::Outcome::Success(_),
                ..
            } => Ok(()),
            Message::Response {
                outcome: chariox_app_runtime::wire::Outcome::Failure(failure),
                ..
            } => Err(handler_error(&failure.error)),
            _ => Err(AppWorkerError::Unavailable.into()),
        }
    }
}

/// What the kernel sends for one due wake: its parameters and its background
/// context. The operation is one firing, so a rescheduled wake is a new one.
fn wake_request(
    installation: &str,
    wake: &chariox_app_runtime::managed_state::Wake,
    overdue: bool,
) -> (Value, Value) {
    (
        json!({
            "id": wake.id, "dueAtMs": wake.due_at_ms, "revision": wake.revision, "overdue": overdue,
        }),
        background_context(
            installation,
            "schedule".into(),
            bounded("wake-", &format!("{}-{}", wake.id, wake.due_at_ms)),
        ),
    )
}

/// What the kernel sends for one accepted inbox occurrence.
fn event_request(item: &chariox_app_runtime::app_inbox::InboxItem) -> (Value, Value) {
    (
        json!({
            "name": item.event_name, "occurrence_id": item.occurrence_id, "payload": item.payload,
        }),
        background_context(
            &item.installation_id,
            bounded("inbox:", &item.route_id),
            format!("inbox-{}", item.sequence),
        ),
    )
}

/// A kernel wake or an inbox delivery is background work: the handler gets the
/// same descriptive attribution a tool call gets (`actor.kind` "background",
/// naming the schedule or the inbox route), with no Room and no agent turn.
fn background_context(installation: &str, source: String, operation: String) -> Value {
    json!({
        "installation_id": installation,
        "operation_id": operation,
        "actor": chariox_app_runtime::app_catalog::Actor::Background(source),
    })
}

/// Context ids follow the tool-call bounds. A derived id that would break them
/// keeps its kind prefix and replaces only the variable part with a digest, so
/// `inbox:` and `wake-` ids stay recognizable.
fn bounded(prefix: &str, variable: &str) -> String {
    use sha2::{Digest, Sha256};
    let id = format!("{prefix}{variable}");
    if chariox_app_runtime::app_catalog::is_context_id(&id) {
        return id;
    }
    format!("{prefix}sha256-{:x}", Sha256::digest(variable.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::{bounded, event_request, wake_request};
    use chariox_app_runtime::{
        app_inbox::InboxItem,
        managed_state::Wake,
        wire::{Message, Sender, WIRE_VERSION},
    };

    #[test]
    fn background_deliveries_carry_background_attribution() {
        let item = InboxItem {
            sequence: 7,
            owner_id: "owner".into(),
            installation_id: "app_1".into(),
            route_id: "requests".into(),
            event_name: "todo_requested".into(),
            occurrence_id: "occurrence-1".into(),
            payload: serde_json::json!({}),
            attempts: 0,
            accepted_generation: 1,
        };
        let (params, context) = event_request(&item);
        assert_eq!(params["occurrence_id"], "occurrence-1");
        assert_eq!(
            context["actor"],
            serde_json::json!({"kind": "background", "id": "inbox:requests"})
        );
        assert_eq!(context["operation_id"], "inbox-7");
        assert_eq!(context["installation_id"], "app_1");
        // No Room or agent turn is invented for background work.
        assert!(context.get("room_id").is_none() && context.get("agent_id").is_none());

        // Each firing of a wake is its own operation, stable across retries.
        let wake = |due_at_ms| Wake {
            id: "todo-1".into(),
            due_at_ms,
            revision: "3".into(),
        };
        let (_, first) = wake_request("app_1", &wake(1_000), false);
        let (_, retried) = wake_request("app_1", &wake(1_000), true);
        let (_, rescheduled) = wake_request("app_1", &wake(2_000), false);
        assert_eq!(
            first["actor"],
            serde_json::json!({"kind": "background", "id": "schedule"})
        );
        assert_eq!(first["operation_id"], retried["operation_id"]);
        assert_ne!(first["operation_id"], rescheduled["operation_id"]);

        // The kernel may send it; the wire rejects it only from a worker.
        let request = Message::Request {
            version: WIRE_VERSION,
            generation: "generation-1".into(),
            id: "request-1".into(),
            method: "events.deliver".into(),
            params,
            deadline_ms: 1_000,
            context: Some(context),
        };
        assert!(request.validate("generation-1", Sender::Supervisor).is_ok());
        assert!(request.validate("generation-1", Sender::Worker).is_err());
    }

    #[test]
    fn derived_context_ids_stay_within_the_tool_call_bounds() {
        assert_eq!(bounded("inbox:", "requests"), "inbox:requests");
        // A route with a space or a long id keeps its kind prefix.
        let spaced = bounded("inbox:", "support requests");
        assert!(spaced.starts_with("inbox:sha256-") && spaced.len() <= 128);
        let long = bounded("wake-", &format!("{}-1", "x".repeat(200)));
        assert!(long.starts_with("wake-sha256-") && long.len() <= 128);
        assert!(chariox_app_runtime::app_catalog::is_context_id(&spaced));
    }
}
