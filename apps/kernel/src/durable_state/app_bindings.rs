//! Binding eligibility shares the kernel writer. A persisted binding is tool
//! selection, never a cached permission to invoke a revoked or replaced App.

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;
use chariox_app_runtime::{
    installation::{ActiveGeneration, InstallationError, InstallationRegistry, VerifiedStageError},
    publisher_trust::PublisherTrustRegistry,
};
use rusqlite::{Connection, TransactionBehavior};
use std::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppBindingError {
    #[error(transparent)]
    Verified(#[from] VerifiedStageError),
    #[error(transparent)]
    Storage(#[from] DaemonError),
}

#[derive(Debug)]
pub(super) struct AppBindingRequest {
    owner: String,
    installation_id: String,
    room: RoomScope,
    response: mpsc::Sender<Result<ActiveGeneration, VerifiedStageError>>,
}

/// Which Room's installations a binding check sees.
#[derive(Debug)]
enum RoomScope {
    /// The installations visible in this Room (`None`: outside any Room, release installs only).
    Visible(Option<String>),
    /// The installation wherever it lives: the owner's own, including a dev install of any Room.
    Owned,
}

impl DurableKernelStateStore {
    /// Blocking check; retain the command's admission until this returns. An
    /// offline worker is fine: no worker readiness or tool availability is implied.
    pub(crate) fn check_app_binding_in_room(
        &self,
        trusted_owner: &str,
        installation_id: &str,
        room: Option<&str>,
    ) -> Result<ActiveGeneration, AppBindingError> {
        self.check_app_binding_scoped(
            trusted_owner,
            installation_id,
            RoomScope::Visible(room.map(str::to_owned)),
        )
    }

    /// The same check for an App the owner opens as a client view: a dev install belongs to one
    /// Room, and the window is that Room's, so the Room is the installation's own.
    pub(crate) fn check_owned_app_binding(
        &self,
        trusted_owner: &str,
        installation_id: &str,
    ) -> Result<ActiveGeneration, AppBindingError> {
        self.check_app_binding_scoped(trusted_owner, installation_id, RoomScope::Owned)
    }

    fn check_app_binding_scoped(
        &self,
        trusted_owner: &str,
        installation_id: &str,
        room: RoomScope,
    ) -> Result<ActiveGeneration, AppBindingError> {
        for value in [trusted_owner, installation_id] {
            if value.is_empty()
                || value.len() > 128
                || value.chars().any(|c| c.is_whitespace() || c.is_control())
            {
                return Err(VerifiedStageError::from(InstallationError::Invalid(
                    "app binding identity",
                ))
                .into());
            }
        }
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppBinding(Box::new(
                AppBindingRequest {
                    owner: trusted_owner.into(),
                    installation_id: installation_id.into(),
                    room,
                    response,
                },
            )))?;
        crate::durable_state::await_writer_response(receiver)
            .map_err(|_| DaemonError::LocalTransport {
                operation: "durable_state.await_app_binding",
                message: "App binding check did not complete".into(),
            })?
            .map_err(AppBindingError::from)
    }
}

pub(super) fn execute(connection: &mut Connection, request: AppBindingRequest) {
    let result = apply(
        connection,
        &request.owner,
        &request.installation_id,
        &request.room,
    );
    let _ = request.response.send(result);
}

fn apply(
    connection: &mut Connection,
    owner: &str,
    installation_id: &str,
    room: &RoomScope,
) -> Result<ActiveGeneration, VerifiedStageError> {
    let binding = InstallationRegistry::new(connection).active_trust(owner, installation_id)?;
    let trust = PublisherTrustRegistry::new(connection).trusted_publisher(
        owner,
        binding.publisher_id(),
        binding.key_id(),
    )?;
    let installation = InstallationRegistry::new(connection).get(installation_id)?;
    let visible = match room {
        RoomScope::Visible(room) => installation.visible_in_room(room.as_deref()),
        RoomScope::Owned => true,
    };
    if !visible {
        return Err(InstallationError::NotFound.into());
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // The lookups above use the sole writer; require_active repeats the exact
    // installation, retained binding and enrollment checks under this transaction.
    let active = binding.require_active(&transaction, owner, &trust)?;
    transaction.commit()?;
    Ok(active)
}

#[cfg(test)]
mod tests;
