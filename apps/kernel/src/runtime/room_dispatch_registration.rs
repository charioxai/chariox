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
        }))?;
    }
    Ok(())
}
