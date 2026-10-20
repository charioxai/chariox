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
    let prompt = prompt
        .ok_or_else(|| crate::durable_state::agent_lifecycle::error("no live creating turn"))?;
    store.agent_lifecycle(
        crate::durable_state::agent_lifecycle::Operation::RegisterObligation {
            owner: actor.owner_user_id().into(),
            room: actor.session_id().into(),
            agent: actor.id().into(),
            prompt: prompt.into(),
            run: run.map(str::to_owned),
            id: id.clone(),
            kind: kind.into(),
            resource: resource.map(str::to_owned),
            now: crate::session::unix_epoch_ms(),
        },
    )?;
    Ok(id)
}

pub(crate) fn receipt(
    store: &DurableKernelStateStore,
    id: Option<&str>,
    accepted: bool,
    resource: Option<&str>,
) -> Result<(), DaemonError> {
    if let Some(id) = id {
        store
            .agent_lifecycle(
                crate::durable_state::agent_lifecycle::Operation::DispatchReceipt {
                    id: id.into(),
                    accepted,
                    resource: resource.map(str::to_owned),
                },
            )
            .map_err(|error| dispatch_error(Some(id), Some(accepted), resource, error))?;
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
