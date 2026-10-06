//! MP-08 / MP-11, A01: one durable intent/receipt format at creation boundaries.
use crate::{agent::AgentInstance, durable_state::DurableKernelStateStore, error::DaemonError};

pub(crate) fn register(
    store: &DurableKernelStateStore,
    actor: &AgentInstance,
    run: Option<&str>,
    prompt: Option<&str>,
    kind: &str,
    resource: Option<&str>,
) -> Result<String, DaemonError> {
    let id = format!("obligation-{:032x}", rand::random::<u128>());
    store.append_event("room.obligation.registered", Some(id.clone()), serde_json::json!({
        "schema_version": 1, "id": id, "room_id": actor.session_id(), "creating_agent_id": actor.id(),
        "creating_provider_run_id": run, "creating_prompt_id": prompt, "kind": kind,
        "resource_ref": resource, "status": "open", "dispatch_state": "intent", "created_at_ms": crate::session::unix_epoch_ms(),
    }))?;
    Ok(id)
}

pub(crate) fn receipt(
    store: &DurableKernelStateStore,
    id: Option<&str>,
    accepted: bool,
    resource: Option<&str>,
) -> Result<(), DaemonError> {
    if let Some(id) = id {
        store.append_event("room.obligation.dispatch_receipt", Some(id.to_string()), serde_json::json!({
            "schema_version": 1, "id": id, "dispatch_state": if accepted {"accepted"} else {"rejected"},
            "status": if accepted {"open"} else {"failed"}, "resource_id": resource, "recorded_at_ms": crate::session::unix_epoch_ms(),
        })).map_err(|error| dispatch_error(Some(id), Some(accepted), resource, error))?;
    }
    Ok(())
}

/// After effects, preserve their identity even if persistence/projection fails.
/// Unknown acceptance must never be classified as a proven rejection.
pub(crate) fn dispatch_error(
    id: Option<&str>,
    accepted: Option<bool>,
    resource: Option<&str>,
    error: DaemonError,
) -> DaemonError {
    let Some(id) = id else {
        return error;
    }; // Legacy callers retain their error contract.
    DaemonError::LocalTransport {
        operation: "room_dispatch",
        message: format!("{} dispatch for obligation `{id}`, resource `{}`; persistence/dispatch failed: {error}; inspect the resource before retrying",
            match accepted {Some(true) => "accepted", Some(false) => "rejected", None => "uncertain"}, resource.unwrap_or("none")),
    }
}

pub(crate) fn reject_if_failed<T>(
    store: &DurableKernelStateStore,
    id: Option<&str>,
    result: Result<T, DaemonError>,
) -> Result<T, DaemonError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            receipt(store, id, false, None)?;
            Err(error)
        }
    }
}
