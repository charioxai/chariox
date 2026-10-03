//! Verified package staging and activation use the existing durable writer.
//! No terminal/App payload can construct the immutable verification candidate.
//! Caller admission and verified runtime/file lifetime remain installer duties.

use std::sync::mpsc;

use chariox_app_runtime::{
    installation::{
        InstallationError, InstallationRegistry, StageToken, VerifiedInstallCandidate,
        VerifiedStageError,
    },
    publisher_trust::PublisherTrustRegistry,
};
use rusqlite::Connection;

use super::{apps::AppRegistryOutcome, DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppVerifiedInstallationError {
    #[error(transparent)]
    Verified(#[from] VerifiedStageError),
    #[error(transparent)]
    Storage(#[from] DaemonError),
}

/// Owned, internal requests. A candidate comes only from verified package bytes
/// plus enrolled signer provenance. Capability decisions remain a separate
/// existing kernel operation; these requests do not assert human approval.
#[derive(Debug, Clone)]
pub(crate) enum AppVerifiedInstallationMutation {
    CreateAndStage {
        installation_id: String,
        candidate: VerifiedInstallCandidate,
        now_ms: u64,
    },
    Stage {
        installation_id: String,
        expected_generation: u64,
        candidate: VerifiedInstallCandidate,
        now_ms: u64,
    },
    Commit {
        token: StageToken,
        now_ms: u64,
    },
}

#[derive(Debug)]
pub(super) struct AppVerifiedInstallationRequest {
    pub(super) owner_id: String,
    pub(super) mutation: AppVerifiedInstallationMutation,
    pub(super) response: mpsc::Sender<Result<AppRegistryOutcome, VerifiedStageError>>,
}

impl DurableKernelStateStore {
    /// Blocking return means the writer committed/rejected the transaction.
    /// The installer must retain its bounded admission until this returns.
    pub(crate) fn mutate_verified_app_installation(
        &self,
        trusted_owner: &str,
        mutation: AppVerifiedInstallationMutation,
    ) -> Result<AppRegistryOutcome, AppVerifiedInstallationError> {
        validate_owner(trusted_owner)?;
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::VerifiedApp(Box::new(
                AppVerifiedInstallationRequest {
                    owner_id: trusted_owner.to_owned(),
                    mutation,
                    response,
                },
            )))?;
        let result = receiver
            .recv()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.await_write",
                message: error.to_string(),
            })?;
        Ok(result?)
    }
}

/// Called after the existing writer flushes its ordinary batch. A failed trust
/// or activation check rolls back this operation without poisoning later work.
pub(super) fn execute(connection: &mut Connection, request: AppVerifiedInstallationRequest) {
    let result = apply(connection, &request.owner_id, request.mutation);
    let _ = request.response.send(result);
}

fn apply(
    connection: &mut Connection,
    owner: &str,
    mutation: AppVerifiedInstallationMutation,
) -> Result<AppRegistryOutcome, VerifiedStageError> {
    validate_owner(owner)?;
    match mutation {
        AppVerifiedInstallationMutation::CreateAndStage {
            installation_id,
            candidate,
            now_ms,
        } => InstallationRegistry::new(connection)
            .create_and_stage_verified(&installation_id, owner, &candidate, now_ms)
            .map(AppRegistryOutcome::Update),
        AppVerifiedInstallationMutation::Stage {
            installation_id,
            expected_generation,
            candidate,
            now_ms,
        } => InstallationRegistry::new(connection)
            .stage_verified(
                &installation_id,
                owner,
                expected_generation,
                &candidate,
                now_ms,
            )
            .map(AppRegistryOutcome::Update),
        AppVerifiedInstallationMutation::Commit { token, now_ms } => {
            let binding = InstallationRegistry::new(connection).staged_trust(owner, &token)?;
            // Obtain current trust on the writer. The activation transaction
            // repeats the check and compares against the stage's retained
            // revision, so fetching a fresh snapshot never revives an old stage.
            let current_trust = PublisherTrustRegistry::new(connection).trusted_publisher(
                owner,
                binding.publisher_id(),
                binding.key_id(),
            )?;
            InstallationRegistry::new(connection)
                .commit_verified(&token, owner, &current_trust, now_ms)
                .map(AppRegistryOutcome::Active)
        }
    }
}

fn validate_owner(owner: &str) -> Result<(), VerifiedStageError> {
    if owner.trim().is_empty() || owner.len() > 128 || owner.chars().any(char::is_control) {
        return Err(InstallationError::Invalid("owner identity").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
