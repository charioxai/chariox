//! App persistence uses the existing kernel writer. Callers authenticate the
//! owner and verify packages/policy before using this internal, non-wire API.
//! These records do not themselves launch, sandbox or approve an App.

use std::sync::mpsc;

use chariox_app_runtime::installation::{
    ActiveGeneration, CapabilityDecision, Installation, InstallationError, InstallationPage,
    InstallationRegistry, ReleaseMetadata, StageToken, UpdateRecord,
};
use rusqlite::Connection;

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
    CreateAndStage {
        installation_id: String,
        release: ReleaseMetadata,
        now_ms: u64,
    },
    Stage {
        installation_id: String,
        expected_generation: u64,
        release: ReleaseMetadata,
        now_ms: u64,
    },
    Decide {
        token: StageToken,
        decision: CapabilityDecision,
        now_ms: u64,
    },
    Quiesce {
        token: StageToken,
        now_ms: u64,
    },
    MarkPrepared {
        token: StageToken,
        now_ms: u64,
    },
    Commit {
        token: StageToken,
        now_ms: u64,
    },
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
    match mutation {
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
    }
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
