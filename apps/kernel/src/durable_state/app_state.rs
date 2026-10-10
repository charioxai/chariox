//! Structured App operations share the durable writer and its current signer
//! fence. The worker service retains admission until completion, including when
//! an awaiting SDK call is cancelled. No App payload provides its own catalog.

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;
use crate::runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped};
use chariox_app_runtime::{
    app_catalog::CatalogError,
    app_outbox::{
        AppOutbox, AutomationConfiguration, EventCatalog, Occurrence, OutboxError, Receipt,
        MAX_PENDING,
    },
    managed_state::{
        ManagedStateStore, StateChanges, StateError, StateRecord, StateScope, Wake, WakeChange,
    },
};
use rusqlite::{Connection, TransactionBehavior};
use std::sync::{mpsc, Arc};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppStateError {
    #[error(transparent)]
    Stopped(#[from] AppOperationStopped),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Outbox(#[from] OutboxError),
    #[error(transparent)]
    Storage(#[from] DaemonError),
}

pub(crate) enum AppStateOperation {
    Get {
        key: String,
    },
    Transaction {
        changes: StateChanges,
        occurrences: Vec<Occurrence>,
        wakes: Vec<WakeChange>,
        wakes_count_as_use: bool,
    },
    Emit(Occurrence),
    /// Kernel-owned wakes; see `managed_state::wakes`.
    Schedule {
        wakes: Vec<WakeChange>,
        wakes_count_as_use: bool,
    },
    ScheduleList,
    /// The App's own automations and their latest receipts (read-only).
    Automations,
    Status {
        receipt_id: String,
    },
    Retry {
        receipt_id: String,
    },
    /// A migrating worker finished the step to this data schema.
    MigrationStep {
        to: u32,
    },
    /// A migrating worker is about to (re)start its steps.
    MigrationRewind,
}
impl AppStateOperation {
    /// Records where the operation's wakes were armed: only a wake armed while
    /// the App served a tool call or an inbound event counts as use when it
    /// is delivered (owner decision 6). Other operations are unchanged.
    pub(crate) fn armed_during_use(mut self, during_use: bool) -> Self {
        if let Self::Transaction {
            wakes_count_as_use, ..
        }
        | Self::Schedule {
            wakes_count_as_use, ..
        } = &mut self
        {
            *wakes_count_as_use = during_use;
        }
        self
    }
    fn name(&self) -> &'static str {
        match self {
            Self::Get { .. } => "get",
            Self::Transaction { .. } => "transaction",
            Self::Emit(_) => "emit",
            Self::Schedule { .. } => "schedule",
            Self::ScheduleList => "schedule_list",
            Self::Automations => "automations",
            Self::Status { .. } => "status",
            Self::Retry { .. } => "retry",
            Self::MigrationStep { .. } => "migration_step",
            Self::MigrationRewind => "migration_rewind",
        }
    }
}
impl std::fmt::Debug for AppStateOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum AppStateOutcome {
    Value(Option<StateRecord>),
    Transaction {
        revision: u64,
        receipts: Vec<Receipt>,
    },
    Receipt(Receipt),
    Wakes(Vec<Wake>),
    Automations(Vec<(AutomationConfiguration, Option<Receipt>)>),
    /// The schema a rewound migration restarts from.
    Rewound(Option<u32>),
}

pub(super) struct AppStateRequest {
    owner: String,
    catalog: Arc<EventCatalog>,
    operation: AppStateOperation,
    budget: AppOperationBudget,
    wake_changed: Arc<tokio::sync::Notify>,
    response: mpsc::Sender<Result<AppStateOutcome, AppStateError>>,
}

impl std::fmt::Debug for AppStateRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppStateRequest")
            .field("installation_id", &self.catalog.installation_id())
            .field("generation", &self.catalog.generation())
            .field("operation", &self.operation.name())
            .finish_non_exhaustive()
    }
}

impl DurableKernelStateStore {
    /// Worker-internal operation. `catalog` belongs to that admitted worker,
    /// and owner comes from kernel authentication, never an App argument.
    /// This blocking method grants no capability or workflow target. Event
    /// receipts and structured state are acknowledged only after their commit.
    pub(crate) fn execute_app_state(
        &self,
        trusted_owner: &str,
        catalog: Arc<EventCatalog>,
        operation: AppStateOperation,
        budget: AppOperationBudget,
    ) -> Result<AppStateOutcome, AppStateError> {
        budget.check()?;
        // Validate before allocating/queuing owner state. Exact enrollment and
        // active generation are repeated on the writer, not trusted from here.
        StateScope::new(
            trusted_owner,
            catalog.installation_id(),
            catalog.generation(),
        )?;
        match &operation {
            AppStateOperation::Transaction { occurrences, .. } if !occurrences.is_empty() => {
                AppOutbox::validate_occurrences(occurrences)?;
            }
            AppStateOperation::Emit(occurrence) => {
                AppOutbox::validate_occurrences(std::slice::from_ref(occurrence))?;
            }
            _ => {}
        }
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppState(Box::new(AppStateRequest {
                owner: trusted_owner.into(),
                catalog,
                operation,
                budget,
                wake_changed: self.app_wake_changed.clone(),
                response,
            })))?;
        receiver
            .recv()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.await_app_state",
                message: error.to_string(),
            })?
    }
}

impl DurableKernelStateStore {
    /// The data schema a staged generation's worker migrates from, when its
    /// update opened a migration (quiesce does, in the writer).
    pub(crate) fn app_migration_from(
        &self,
        installation: &str,
        generation: u64,
    ) -> Result<Option<u32>, DaemonError> {
        let connection = self.lock_connection("durable_state.app_migration")?;
        ManagedStateStore::migration_from(&connection, installation, generation).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "durable_state.app_migration",
                message: error.to_string(),
            }
        })
    }
}

pub(super) fn initialize(connection: &mut Connection) -> Result<(), DaemonError> {
    ManagedStateStore::new(connection)
        .initialize()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.migrate_app_state",
            message: error.to_string(),
        })
}

pub(super) fn execute(connection: &mut Connection, request: AppStateRequest) {
    let changes_wakes = match &request.operation {
        AppStateOperation::Schedule { wakes, .. }
        | AppStateOperation::Transaction { wakes, .. } => !wakes.is_empty(),
        _ => false,
    };
    let result = apply(
        connection,
        &request.owner,
        &request.catalog,
        request.operation,
        &request.budget,
    );
    if let Err(
        AppStateError::State(StateError::Database(error))
        | AppStateError::Outbox(OutboxError::Database(error)),
    ) = &result
    {
        super::storage_full::observe(error);
    }
    if matches!(result, Err(AppStateError::Outbox(OutboxError::Full))) {
        // The refusal rolled its own transaction back; the owner's warning is
        // a separate write, and a failure to write it changes nothing else.
        let _ = warn_full_outbox(
            connection,
            &request.owner,
            request.catalog.installation_id(),
        );
    }
    if result.is_ok() && changes_wakes {
        // Notify on the writer after commit, even if the caller was cancelled.
        request.wake_changed.notify_one();
    }
    let _ = request.response.send(result);
}

/// Tells the owner once per backlog that the App's event outbox is full: a
/// new warning needs every event that was waiting at the last one to have
/// left the outbox, so a steady refusal loop writes one notice, not one per
/// refused event.
fn warn_full_outbox(
    connection: &mut Connection,
    owner: &str,
    installation: &str,
) -> Result<(), OutboxError> {
    const MARKER: &str = "outbox_full";
    // Mostly reads that decide to write nothing; the writer's own connection
    // takes the write lock only if it appends the notice.
    let transaction = connection.transaction()?;
    let oldest = AppOutbox::oldest_waiting_accepted_at_in(&transaction, owner, installation)?;
    let warned =
        super::app_logs::latest_kernel_notice_at_in(&transaction, owner, installation, MARKER)?;
    if warned.is_some_and(|warned| oldest.is_some_and(|oldest| warned >= oldest)) {
        return Ok(());
    }
    let mut fields = serde_json::Map::new();
    fields.insert(MARKER.into(), true.into());
    fields.insert("pending".into(), MAX_PENDING.into());
    super::app_logs::append_kernel_notice_in(
        &transaction,
        owner,
        installation,
        crate::session::unix_epoch_ms(),
        &format!(
            "The App's event outbox is full: {MAX_PENDING} events are waiting for delivery, so its new events are refused until some are delivered. Check that its automations' workflow targets are running."
        ),
        fields,
    )?;
    Ok(transaction.commit()?)
}

fn apply(
    connection: &mut Connection,
    owner: &str,
    catalog: &Arc<EventCatalog>,
    operation: AppStateOperation,
    budget: &AppOperationBudget,
) -> Result<AppStateOutcome, AppStateError> {
    // A request may have waited in the FIFO writer queue after IPC expiry.
    budget.check()?;
    let scope = StateScope::new(owner, catalog.installation_id(), catalog.generation())?;
    let mut transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StateError::from)?;
    // Publisher revoke/re-enroll can invalidate an otherwise unchanged active
    // generation. Hold its exact retained verification fence through commit.
    let current = catalog.app_catalog().require_current(&transaction, owner);
    // A migrating worker of the pending stage may only read and write state.
    let migration = match &operation {
        AppStateOperation::Get { .. }
        | AppStateOperation::MigrationStep { .. }
        | AppStateOperation::MigrationRewind => true,
        AppStateOperation::Transaction {
            occurrences, wakes, ..
        } => occurrences.is_empty() && wakes.is_empty(),
        _ => false,
    };
    match current {
        Err(_) if migration => catalog.app_catalog().require_staged(&transaction, owner)?,
        current => current?,
    }
    // BEGIN IMMEDIATE may itself wait for a writer lock. Check again after
    // acquiring it, immediately before state work. Once this check admits the
    // transaction, cancellation cannot promise that its commit was undone.
    budget.check()?;
    let outcome = match operation {
        AppStateOperation::Get { key } => {
            AppStateOutcome::Value(ManagedStateStore::read_in(&transaction, scope, &key)?)
        }
        AppStateOperation::Transaction {
            changes,
            occurrences,
            wakes,
            wakes_count_as_use,
        } => {
            let revision = ManagedStateStore::apply_in(&mut transaction, scope, &changes)?;
            let receipts = events::accept(&mut transaction, catalog, owner, &occurrences)?;
            ManagedStateStore::apply_wakes_in(&mut transaction, scope, &wakes, wakes_count_as_use)?;
            AppStateOutcome::Transaction { revision, receipts }
        }
        AppStateOperation::Schedule {
            wakes,
            wakes_count_as_use,
        } => {
            ManagedStateStore::apply_wakes_in(&mut transaction, scope, &wakes, wakes_count_as_use)?;
            AppStateOutcome::Wakes(ManagedStateStore::wakes_in(&transaction, scope)?)
        }
        AppStateOperation::ScheduleList => {
            AppStateOutcome::Wakes(ManagedStateStore::wakes_in(&transaction, scope)?)
        }
        AppStateOperation::Automations => {
            AppStateOutcome::Automations(AppOutbox::automations_in(&transaction, catalog, owner)?)
        }
        AppStateOperation::MigrationStep { to } => {
            ManagedStateStore::migration_step_in(&transaction, scope, to)?;
            AppStateOutcome::Value(None)
        }
        AppStateOperation::MigrationRewind => {
            AppStateOutcome::Rewound(ManagedStateStore::migration_rewind_in(&transaction, scope)?)
        }
        event => AppStateOutcome::Receipt(events::apply(&mut transaction, catalog, owner, event)?),
    };
    transaction.commit().map_err(StateError::from)?;
    Ok(outcome)
}

mod events;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn fixture_catalog(
    store: &DurableKernelStateStore,
) -> Arc<chariox_app_runtime::app_catalog::AppCatalog> {
    tests::catalog(store).app_catalog().clone()
}

#[cfg(test)]
pub(crate) fn fixture_event_catalog(store: &DurableKernelStateStore) -> Arc<EventCatalog> {
    tests::catalog(store)
}

/// Another active installation of the fixture package, for an owner who
/// already trusts its publisher (after `fixture_catalog`).
#[cfg(test)]
pub(crate) fn fixture_installation(
    store: &DurableKernelStateStore,
    owner: &str,
    installation_id: &str,
) {
    tests::install_package(store, owner, installation_id, tests::package());
}

/// Installs `installed` for `owner` from a package that also declares the
/// incoming event `received`; returns the package to stage its release.
#[cfg(test)]
pub(crate) fn fixture_inbox_installation(
    store: &DurableKernelStateStore,
    owner: &str,
) -> (Vec<u8>, chariox_app_package::TrustedPublisher) {
    let package = tests::inbox_package();
    tests::catalog_for_owner(store, owner, package.clone());
    package
}

/// P1.20: a deployment's own active installation of `package`, as a
/// committed deployment install leaves it (the owner already trusts the
/// fixture publisher).
#[cfg(test)]
pub(crate) fn fixture_copy_installation(
    store: &DurableKernelStateStore,
    owner: &str,
    installation_id: &str,
    deployment_id: &str,
    package: (Vec<u8>, chariox_app_package::TrustedPublisher),
) {
    tests::install_package(store, owner, installation_id, package);
    store.fixture_tag_app_installation(owner, installation_id, deployment_id);
}

/// The owner's installation updated to `package` (approved and committed).
#[cfg(test)]
pub(crate) fn fixture_update_installation(
    store: &DurableKernelStateStore,
    owner: &str,
    installation_id: &str,
    package: (Vec<u8>, chariox_app_package::TrustedPublisher),
) {
    tests::update_package(store, owner, installation_id, package);
}

/// The fixture inbox App at another version; `schema` > 0 declares data
/// migrations.
#[cfg(test)]
pub(crate) fn fixture_inbox_package_version(
    version: &str,
    schema: u32,
) -> (Vec<u8>, chariox_app_package::TrustedPublisher) {
    tests::inbox_package_version(version, schema)
}

/// Another active installation of the event fixture package for `owner`.
#[cfg(test)]
pub(crate) fn fixture_event_installation(
    store: &DurableKernelStateStore,
    owner: &str,
    installation_id: &str,
) {
    tests::install_package(store, owner, installation_id, tests::package());
}

#[cfg(test)]
pub(crate) fn fixture_inbox_package() -> (Vec<u8>, chariox_app_package::TrustedPublisher) {
    tests::inbox_package()
}

#[cfg(test)]
pub(crate) fn fixture_event_package() -> (Vec<u8>, chariox_app_package::TrustedPublisher) {
    tests::package()
}

#[cfg(test)]
pub(crate) fn fixture_release_package(
    version: &str,
    schema: u32,
    with_network: bool,
) -> (Vec<u8>, chariox_app_package::TrustedPublisher) {
    tests::release_package(version, schema, with_network)
}

#[cfg(test)]
pub(crate) fn fixture_http_catalog(store: &DurableKernelStateStore) -> Arc<EventCatalog> {
    tests::http_catalog(store)
}

#[cfg(test)]
pub(crate) fn fixture_http_package() -> (Vec<u8>, chariox_app_package::TrustedPublisher) {
    tests::http_package()
}

#[cfg(test)]
pub(crate) fn fixture_tool_catalog(store: &DurableKernelStateStore) -> Arc<EventCatalog> {
    tests::tool_catalog(store)
}

#[cfg(test)]
pub(crate) fn fixture_tool_package() -> (Vec<u8>, chariox_app_package::TrustedPublisher) {
    tests::tool_package()
}

#[cfg(test)]
pub(crate) fn fixture_browser_tool_package(
    store: &DurableKernelStateStore,
) -> (
    Arc<EventCatalog>,
    Vec<u8>,
    chariox_app_package::TrustedPublisher,
) {
    let (bytes, publisher) = tests::browser_tool_package();
    let catalog = tests::catalog_for_owner(store, "alice", (bytes.clone(), publisher.clone()));
    (catalog, bytes, publisher)
}

/// Stages the tool fixture's release again as an update of `installation`
/// (at generation 1) and approves it, without starting it.
#[cfg(test)]
pub(crate) fn fixture_stage_approved_tool_update(
    store: &DurableKernelStateStore,
    installation: &str,
) {
    tests::stage_approved_update(store, "alice", installation, tests::tool_package())
}

#[cfg(test)]
pub(crate) fn fixture_neighbour_catalog(store: &DurableKernelStateStore) -> Arc<EventCatalog> {
    tests::install_package(store, "alice", "neighbour", tests::package())
}

#[cfg(test)]
pub(crate) fn fixture_host_package() -> (Vec<u8>, chariox_app_package::TrustedPublisher) {
    tests::host_package()
}
#[cfg(test)]
pub(crate) fn fixture_event_catalog_from_package(
    store: &DurableKernelStateStore,
    package: (Vec<u8>, chariox_app_package::TrustedPublisher),
) -> Arc<EventCatalog> {
    tests::catalog_for_owner(store, "alice", package)
}
