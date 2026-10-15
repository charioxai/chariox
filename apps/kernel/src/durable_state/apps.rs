//! App persistence uses the existing kernel writer. Callers authenticate the
//! owner and verify packages/policy before using this internal, non-wire API.
//! These records do not themselves launch, sandbox or approve an App.

use std::sync::mpsc;

use chariox_app_runtime::installation::{
    ActiveGeneration, CapabilityDecision, Installation, InstallationError, InstallationPage,
    InstallationRegistry, ReleaseMetadata, StageToken, UpdateRecord,
};
use rusqlite::{Connection, OptionalExtension};

use crate::error::DaemonError;

use super::{DurableKernelStateStore, DurableWriterRequest};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppRegistryError {
    #[error(transparent)]
    Registry(#[from] InstallationError),
    #[error(transparent)]
    Storage(#[from] DaemonError),
}

/// Only trusted kernel code constructs mutations. In particular, release
/// metadata and capability decisions must never be deserialized from an App.
#[derive(Debug, Clone)]
pub(crate) enum AppRegistryMutation {
    #[allow(
        dead_code,
        reason = "Keep the existing typed writer operation or receipt payload for API and regression compatibility"
    )]
    CreateAndStage {
        installation_id: String,
        release: ReleaseMetadata,
        now_ms: u64,
    },
    #[allow(
        dead_code,
        reason = "Keep the existing typed writer operation or receipt payload for API and regression compatibility"
    )]
    Stage {
        installation_id: String,
        expected_generation: u64,
        release: ReleaseMetadata,
        now_ms: u64,
    },
    #[allow(
        dead_code,
        reason = "Keep the existing typed writer operation or receipt payload for API and regression compatibility"
    )]
    Decide {
        token: StageToken,
        decision: CapabilityDecision,
        now_ms: u64,
    },
    #[allow(
        dead_code,
        reason = "Keep the existing typed writer operation or receipt payload for API and regression compatibility"
    )]
    Quiesce { token: StageToken, now_ms: u64 },
    #[allow(
        dead_code,
        reason = "Keep the existing typed writer operation or receipt payload for API and regression compatibility"
    )]
    MarkPrepared { token: StageToken, now_ms: u64 },
    #[allow(
        dead_code,
        reason = "Keep the existing typed writer operation or receipt payload for API and regression compatibility"
    )]
    Commit { token: StageToken, now_ms: u64 },
    #[allow(
        dead_code,
        reason = "Keep the existing typed writer operation or receipt payload for API and regression compatibility"
    )]
    Abort {
        token: StageToken,
        reason: String,
        now_ms: u64,
    },
    Uninstall {
        installation_id: String,
        expected_generation: u64,
        now_ms: u64,
    },
    /// Deletes an uninstalled installation's data records and the release it
    /// kept, so it can no longer be reinstalled into.
    ForgetData {
        installation_id: String,
        expected_generation: u64,
    },
}

impl AppRegistryMutation {
    fn installation_id(&self) -> &str {
        match self {
            Self::CreateAndStage {
                installation_id, ..
            }
            | Self::Stage {
                installation_id, ..
            }
            | Self::Uninstall {
                installation_id, ..
            }
            | Self::ForgetData {
                installation_id, ..
            } => installation_id,
            Self::Decide { token, .. }
            | Self::Quiesce { token, .. }
            | Self::MarkPrepared { token, .. }
            | Self::Commit { token, .. }
            | Self::Abort { token, .. } => &token.installation_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve the existing AppRegistryOutcome typed actor payload layout"
)]
pub(crate) enum AppRegistryOutcome {
    Update(UpdateRecord),
    Active(ActiveGeneration),
    Installation(Installation),
}

#[derive(Debug)]
pub(super) struct AppRegistryRequest {
    pub(super) owner_id: String,
    pub(super) mutation: AppRegistryMutation,
    pub(super) response: mpsc::Sender<Result<AppRegistryOutcome, InstallationError>>,
}

impl DurableKernelStateStore {
    /// The owner is the already authenticated kernel principal, not a field
    /// supplied by a client. This blocking boundary belongs on a blocking task.
    pub(crate) fn mutate_app_installation(
        &self,
        owner_id: &str,
        mutation: AppRegistryMutation,
    ) -> Result<AppRegistryOutcome, AppRegistryError> {
        validate_owner(owner_id)?;
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::App(Box::new(AppRegistryRequest {
                owner_id: owner_id.to_owned(),
                mutation,
                response,
            })))?;
        let result = receiver
            .recv()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.await_write",
                message: error.to_string(),
            })?;
        Ok(result?)
    }

    pub(crate) fn get_app_installation(
        &self,
        owner_id: &str,
        installation_id: &str,
    ) -> Result<Installation, AppRegistryError> {
        validate_owner(owner_id)?;
        let mut connection = self.lock_connection("durable_state.read_app_installation")?;
        let registry = InstallationRegistry::new(&mut connection);
        Ok(owned_installation(&registry, owner_id, installation_id)?)
    }

    pub(crate) fn list_app_installations(
        &self,
        owner_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<InstallationPage, AppRegistryError> {
        validate_owner(owner_id)?;
        let mut connection = self.lock_connection("durable_state.list_app_installations")?;
        Ok(InstallationRegistry::new(&mut connection).list(owner_id, after, limit)?)
    }

    /// Protocol 367: the owner's installations without deployment copies,
    /// which belong to their deployment. The page query excludes them, so
    /// only the last page is short.
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    pub(crate) fn list_app_installations_without_copies(
        &self,
        owner_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<InstallationPage, AppRegistryError> {
        validate_owner(owner_id)?;
        let mut connection =
            self.lock_connection("durable_state.list_app_installations_without_copies")?;
        let mut ids = {
            let mut statement = connection
                .prepare(
                    "SELECT i.installation_id FROM app_installations i
                     WHERE i.owner_id=?1 AND i.installation_id>?2 AND NOT EXISTS(
                       SELECT 1 FROM app_installation_deployments d
                       WHERE d.installation_id=i.installation_id)
                     ORDER BY i.installation_id LIMIT ?3",
                )
                .map_err(InstallationError::from)?;
            let rows = statement
                .query_map(
                    rusqlite::params![owner_id, after.unwrap_or(""), limit as i64 + 1],
                    |row| row.get::<_, String>(0),
                )
                .map_err(InstallationError::from)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(InstallationError::from)?
        };
        let next_cursor = if ids.len() > limit {
            ids.truncate(limit);
            ids.last().cloned()
        } else {
            None
        };
        let registry = InstallationRegistry::new(&mut connection);
        let installations = ids
            .iter()
            .map(|id| registry.get(id))
            .collect::<Result<_, _>>()?;
        Ok(InstallationPage {
            installations,
            next_cursor,
        })
    }

    pub(crate) fn app_installation_journal(
        &self,
        owner_id: &str,
        installation_id: &str,
    ) -> Result<Vec<UpdateRecord>, AppRegistryError> {
        validate_owner(owner_id)?;
        let mut connection = self.lock_connection("durable_state.read_app_journal")?;
        let registry = InstallationRegistry::new(&mut connection);
        owned_installation(&registry, owner_id, installation_id)?;
        Ok(registry.journal(installation_id)?)
    }

    /// Whether an approved update of the installation is under way: from the
    /// owner's approval until it commits or aborts, the old generation drains
    /// and the new one starts, so calls meet no running worker.
    pub(crate) fn app_update_underway(
        &self,
        owner_id: &str,
        installation_id: &str,
    ) -> Result<bool, AppRegistryError> {
        validate_owner(owner_id)?;
        let mut connection = self.lock_connection("durable_state.read_app_update")?;
        let registry = InstallationRegistry::new(&mut connection);
        let Some(pending) =
            owned_installation(&registry, owner_id, installation_id)?.pending_generation
        else {
            return Ok(false);
        };
        Ok(registry.journal(installation_id)?.iter().any(|record| {
            record.token.generation == pending
                && matches!(record.decision, CapabilityDecision::Approved { .. })
        }))
    }
}

pub(super) fn initialize(connection: &mut Connection) -> Result<(), DaemonError> {
    InstallationRegistry::new(connection)
        .initialize()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.migrate_apps",
            message: error.to_string(),
        })
}

pub(super) fn execute(connection: &mut Connection, request: AppRegistryRequest) {
    // Called only between ordinary batches, with no outer transaction. A CAS
    // rejection rolls back this App operation without discarding other writes.
    let result = apply(connection, &request.owner_id, request.mutation);
    let _ = request.response.send(result);
}

fn apply(
    connection: &mut Connection,
    owner_id: &str,
    mutation: AppRegistryMutation,
) -> Result<AppRegistryOutcome, InstallationError> {
    validate_owner(owner_id)?;
    let installation_id = mutation.installation_id().to_owned();
    if matches!(
        &mutation,
        AppRegistryMutation::CreateAndStage { .. } | AppRegistryMutation::Stage { .. }
    ) {
        forget_uninstalled(connection, owner_id, &installation_id)?;
    }
    let uninstall = matches!(&mutation, AppRegistryMutation::Uninstall { .. });
    let mut registry = InstallationRegistry::new(connection);
    // The single writer also serializes this ownership check and the mutation.
    // Installation owner identity is immutable; no second writer is admitted.
    match registry.get(mutation.installation_id()) {
        Ok(installation) if installation.owner_id == owner_id => {}
        Ok(_) => return Err(InstallationError::NotFound),
        Err(InstallationError::NotFound)
            if matches!(&mutation, AppRegistryMutation::CreateAndStage { .. }) => {}
        Err(error) => return Err(error),
    }
    let outcome = match mutation {
        AppRegistryMutation::CreateAndStage {
            installation_id,
            release,
            now_ms,
        } => registry
            .create_and_stage(&installation_id, owner_id, release, now_ms)
            .map(AppRegistryOutcome::Update),
        AppRegistryMutation::Stage {
            installation_id,
            expected_generation,
            release,
            now_ms,
        } => registry
            .stage(&installation_id, expected_generation, release, now_ms)
            .map(AppRegistryOutcome::Update),
        AppRegistryMutation::Decide {
            token,
            decision,
            now_ms,
        } => registry
            .decide(&token, decision, now_ms)
            .map(AppRegistryOutcome::Update),
        AppRegistryMutation::Quiesce { token, now_ms } => registry
            .quiesce(&token, now_ms)
            .map(AppRegistryOutcome::Update),
        AppRegistryMutation::MarkPrepared { token, now_ms } => registry
            .mark_prepared(&token, now_ms)
            .map(AppRegistryOutcome::Update),
        AppRegistryMutation::Commit { token, now_ms } => registry
            .commit(&token, now_ms)
            .map(AppRegistryOutcome::Active),
        AppRegistryMutation::Abort {
            token,
            reason,
            now_ms,
        } => registry
            .abort(&token, &reason, now_ms)
            .map(AppRegistryOutcome::Update),
        AppRegistryMutation::Uninstall {
            installation_id,
            expected_generation,
            now_ms,
        } => registry
            .uninstall(&installation_id, expected_generation, now_ms)
            .map(AppRegistryOutcome::Installation),
        AppRegistryMutation::ForgetData {
            installation_id,
            expected_generation,
        } => forget_data(connection, owner_id, &installation_id, expected_generation)
            .map(AppRegistryOutcome::Installation),
    }?;
    // The uninstall is committed; a failed cleanup is repeated when a
    // reinstall stages, so it does not fail the uninstall.
    if uninstall {
        if let Err(error) = forget_uninstalled(connection, owner_id, &installation_id) {
            crate::logging::warn_with_fields(
                "daemon.apps",
                "configuration of an uninstalled App was not removed",
                serde_json::json!({"installation_id": installation_id, "error": error.to_string()}),
            );
        }
    }
    Ok(outcome)
}

fn owned_installation(
    registry: &InstallationRegistry<'_>,
    owner_id: &str,
    installation_id: &str,
) -> Result<Installation, InstallationError> {
    let installation = registry.get(installation_id)?;
    if installation.owner_id != owner_id {
        return Err(InstallationError::NotFound);
    }
    Ok(installation)
}

fn validate_owner(owner_id: &str) -> Result<(), InstallationError> {
    if owner_id.trim().is_empty() || owner_id.len() > 128 || owner_id.chars().any(char::is_control)
    {
        return Err(InstallationError::Invalid("owner identity"));
    }
    Ok(())
}

/// What an active release allowed ends with it: connection grants,
/// file picks and grants, automations and inbox routes. Runs after an uninstall
/// and again when a reinstall stages, so a reinstall inherits none of them. Only an uninstalled
/// installation (inactive, keeping the release its data belongs to) is touched.
pub(crate) fn forget_uninstalled(
    connection: &Connection,
    owner_id: &str,
    installation_id: &str,
) -> rusqlite::Result<()> {
    let uninstalled: Option<bool> = connection
        .query_row(
            "SELECT active_json IS NULL AND retained_json IS NOT NULL
             FROM app_installations WHERE installation_id=?1",
            [installation_id],
            |row| row.get(0),
        )
        .optional()?;
    if uninstalled != Some(true) {
        return Ok(());
    }
    super::app_file_grants::forget_inactive(connection, owner_id, installation_id)?;
    connection.execute(
        "DELETE FROM app_restore_receipts WHERE owner_id=?1 AND installation_id=?2",
        rusqlite::params![owner_id, installation_id],
    )?;
    super::app_connections::forget_inactive(connection, owner_id, installation_id)?;
    chariox_app_runtime::app_outbox::AppOutbox::disable_all_in(
        connection,
        owner_id,
        installation_id,
    )?;
    chariox_app_runtime::app_inbox::remove_all_routes_in(connection, owner_id, installation_id)
}

/// App data records of an uninstalled installation, deleted together with the
/// release it kept: structured state, wakes, logs, and the file contents it was
/// handed or offered. Its private storage (where a platform has one) is
/// deleted by the supervisor first.
const DATA_TABLES: [&str; 10] = [
    "app_state_values",
    "app_state_heads",
    "app_state_snapshot_values",
    "app_state_migrations",
    "app_wakes",
    "app_logs",
    "app_file_grants",
    "app_file_picks",
    "app_file_exports",
    "app_restore_receipts",
];

fn forget_data(
    connection: &mut Connection,
    owner_id: &str,
    installation_id: &str,
    expected_generation: u64,
) -> Result<Installation, InstallationError> {
    let generation = i64::try_from(expected_generation).map_err(|_| InstallationError::Conflict)?;
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let changed = transaction.execute(
        "UPDATE app_installations SET retained_json=NULL
         WHERE installation_id=?1 AND owner_id=?2 AND generation=?3
           AND active_json IS NULL AND pending_generation IS NULL",
        rusqlite::params![installation_id, owner_id, generation],
    )?;
    if changed != 1 {
        return Err(InstallationError::Conflict);
    }
    for table in DATA_TABLES {
        transaction.execute(
            &format!("DELETE FROM {table} WHERE installation_id=?1"),
            [installation_id],
        )?;
    }
    transaction.commit()?;
    InstallationRegistry::new(connection).get(installation_id)
}
