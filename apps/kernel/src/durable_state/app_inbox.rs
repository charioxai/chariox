//! Installation inbox routes and occurrences on the sole durable writer.
use super::{DurableKernelStateStore, DurableWriterRequest};
use chariox_app_runtime::{
    app_inbox::{self, Accepted, InboxCounts, InboxError, InboxItem, InboxRoute, InboxState},
    installation::InstallationRegistry,
};
use rusqlite::Connection;
use serde_json::Value;
use std::sync::mpsc;

pub(crate) enum AppInboxOperation {
    CreateRoute {
        route: InboxRoute,
        now_ms: u64,
    },
    RemoveRoute {
        owner: String,
        installation: String,
        route_id: String,
    },
    /// P1.20: a deployment copy's route takes over the owner's (see
    /// `balance_owner_inbox_routes`).
    SetRouteActive {
        owner: String,
        installation: String,
        route_id: String,
        active: bool,
    },
    Routes {
        owner: String,
        installation: String,
    },
    /// One route, without counting occurrences.
    Route {
        owner: String,
        installation: String,
        route_id: String,
    },
    /// The payload was validated against the incoming schema of `generation`,
    /// which must still be the active one.
    Accept {
        owner: String,
        installation: String,
        route_id: String,
        occurrence_id: String,
        payload: Value,
        generation: u64,
        now_ms: u64,
    },
    Due {
        now_ms: u64,
        limit: usize,
    },
    Delivered {
        sequence: i64,
        generation: u64,
    },
    Failed {
        sequence: i64,
        now_ms: u64,
    },
    Postponed {
        sequence: i64,
        until_ms: u64,
    },
    /// The live generation no longer accepts the occurrence as it was admitted.
    Undeliverable {
        sequence: i64,
    },
}

#[derive(Debug, PartialEq)]
pub(crate) enum AppInboxOutcome {
    Routes(Vec<(InboxRoute, InboxCounts)>),
    Route(Option<InboxRoute>),
    Accepted(Accepted),
    Due(Vec<InboxItem>),
    Recorded(Option<InboxState>),
}

pub(super) struct AppInboxRequest {
    operation: AppInboxOperation,
    response: mpsc::Sender<Result<AppInboxOutcome, InboxError>>,
}
impl std::fmt::Debug for AppInboxRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppInboxRequest(..)")
    }
}

impl DurableKernelStateStore {
    pub(crate) fn app_inbox(
        &self,
        operation: AppInboxOperation,
    ) -> Result<AppInboxOutcome, InboxError> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppInbox(Box::new(AppInboxRequest {
                operation,
                response,
            })))
            .map_err(|_| InboxError::Corrupt)?;
        receiver.recv().map_err(|_| InboxError::Corrupt)?
    }
}

pub(super) fn initialize(connection: &Connection) -> Result<(), crate::DaemonError> {
    app_inbox::initialize(connection).map_err(|error| crate::DaemonError::LocalTransport {
        operation: "durable_state.migrate_app_inbox",
        message: error.to_string(),
    })
}

pub(super) fn execute(connection: &mut Connection, request: AppInboxRequest) {
    let _ = request.response.send(apply(connection, request.operation));
}

fn apply(
    connection: &mut Connection,
    operation: AppInboxOperation,
) -> Result<AppInboxOutcome, InboxError> {
    match operation {
        AppInboxOperation::CreateRoute { route, now_ms } => {
            let tx = connection.transaction()?;
            app_inbox::create_route_in(&tx, &route, now_ms)?;
            tx.commit()?;
            Ok(AppInboxOutcome::Recorded(None))
        }
        AppInboxOperation::RemoveRoute {
            owner,
            installation,
            route_id,
        } => {
            let tx = connection.transaction()?;
            app_inbox::remove_route_in(&tx, &owner, &installation, &route_id)?;
            tx.commit()?;
            Ok(AppInboxOutcome::Recorded(None))
        }
        AppInboxOperation::SetRouteActive {
            owner,
            installation,
            route_id,
            active,
        } => {
            app_inbox::set_route_active_in(connection, &owner, &installation, &route_id, active)?;
            Ok(AppInboxOutcome::Recorded(None))
        }
        AppInboxOperation::Routes {
            owner,
            installation,
        } => {
            let mut routes = Vec::new();
            for route in app_inbox::routes(connection, &owner, &installation)? {
                let counts = app_inbox::counts(connection, &route)?;
                routes.push((route, counts));
            }
            Ok(AppInboxOutcome::Routes(routes))
        }
        AppInboxOperation::Route {
            owner,
            installation,
            route_id,
        } => Ok(AppInboxOutcome::Route(app_inbox::route(
            connection,
            &owner,
            &installation,
            &route_id,
        )?)),
        AppInboxOperation::Accept {
            owner,
            installation,
            route_id,
            occurrence_id,
            payload,
            generation,
            now_ms,
        } => {
            let route = app_inbox::route(connection, &owner, &installation, &route_id)?
                .ok_or(InboxError::NotFound)?;
            let active = InstallationRegistry::new(connection)
                .active_trust(&owner, &installation)
                .map_err(|_| InboxError::NotFound)?
                .token()
                .generation;
            if active != generation {
                return Err(InboxError::Conflict);
            }
            let tx = connection.transaction()?;
            let accepted =
                app_inbox::accept_in(&tx, &route, &occurrence_id, &payload, generation, now_ms)?;
            tx.commit()?;
            Ok(AppInboxOutcome::Accepted(accepted))
        }
        AppInboxOperation::Due { now_ms, limit } => {
            let tx = connection.transaction()?;
            let due = app_inbox::due(&tx, now_ms, limit)?;
            tx.commit()?;
            Ok(AppInboxOutcome::Due(due))
        }
        AppInboxOperation::Delivered {
            sequence,
            generation,
        } => app_inbox::delivered_in(connection, sequence, generation)
            .map(|()| AppInboxOutcome::Recorded(Some(InboxState::Delivered))),
        AppInboxOperation::Failed { sequence, now_ms } => {
            app_inbox::failed_attempt_in(connection, sequence, now_ms)
                .map(|state| AppInboxOutcome::Recorded(Some(state)))
        }
        AppInboxOperation::Undeliverable { sequence } => {
            app_inbox::undeliverable_in(connection, sequence)
                .map(|()| AppInboxOutcome::Recorded(Some(InboxState::Failed)))
        }
        AppInboxOperation::Postponed { sequence, until_ms } => {
            app_inbox::postpone_in(connection, sequence, until_ms)
                .map(|()| AppInboxOutcome::Recorded(None))
        }
    }
}

impl DurableKernelStateStore {
    /// Every inbox route an event generator feeds for an active installation,
    /// for the kernel's claims. Read on the query connection; a route changes
    /// only on the writer.
    pub(crate) fn app_generator_routes(&self) -> Result<Vec<InboxRoute>, InboxError> {
        let connection = self
            .lock_connection("durable_state.app_generator_routes")
            .map_err(|_| InboxError::Corrupt)?;
        let mut active = connection.prepare(
            "SELECT owner_id,installation_id FROM app_installations WHERE active_json IS NOT NULL",
        )?;
        let active = active
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
        let mut routes = app_inbox::generator_routes(&connection)?;
        routes.retain(|route| {
            active.contains(&(route.owner_id.clone(), route.installation_id.clone()))
        });
        Ok(routes)
    }

    /// The route an event service delivers under `binding_id`.
    pub(crate) fn app_route_by_binding(
        &self,
        binding_id: &str,
    ) -> Result<Option<InboxRoute>, InboxError> {
        let connection = self
            .lock_connection("durable_state.app_route_by_binding")
            .map_err(|_| InboxError::Corrupt)?;
        app_inbox::route_by_binding(&connection, binding_id)
    }
}

#[cfg(test)]
mod tests;
