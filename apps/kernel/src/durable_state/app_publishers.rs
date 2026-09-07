//! Local developer publisher trust shares the kernel's existing durable writer.
//! The caller authenticates ownership and obtains an exact policy/user decision;
//! this internal API does not turn package claims into an enrollment decision.

use std::sync::mpsc;

use chariox_app_package::TrustedPublisher;
use chariox_app_runtime::publisher_trust::{
    PublisherTrustEntry, PublisherTrustError, PublisherTrustRegistry, TrustDecision,
    TrustDecisionReceipt, TrustedPublisherSnapshot,
};
use rusqlite::Connection;

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppPublisherError {
    #[error(transparent)]
    Trust(#[from] PublisherTrustError),
    #[error(transparent)]
    Storage(#[from] DaemonError),
}

/// Kernel-only, never deserialized from App code or treated as human approval.
#[derive(Debug, Clone)]
pub(crate) enum AppPublisherMutation {
    Enroll {
        publisher: TrustedPublisher,
        expected_revision: u64,
        decision: TrustDecision,
        now_ms: u64,
    },
    Revoke {
        publisher_id: String,
        key_id: String,
        expected_revision: u64,
        decision: TrustDecision,
        now_ms: u64,
    },
}

#[derive(Debug)]
pub(super) struct AppPublisherRequest {
    pub(super) owner_id: String,
    pub(super) mutation: AppPublisherMutation,
    pub(super) response: mpsc::Sender<Result<TrustDecisionReceipt, PublisherTrustError>>,
}

impl DurableKernelStateStore {
    /// Blocking completion means the exact decision transaction committed, not
    /// merely that it entered the queue. Retain caller admission until return.
    pub(crate) fn mutate_app_publisher(
        &self,
        trusted_owner: &str,
        mutation: AppPublisherMutation,
    ) -> Result<TrustDecisionReceipt, AppPublisherError> {
        validate_owner(trusted_owner)?;
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppPublisher(Box::new(
                AppPublisherRequest {
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

    pub(crate) fn list_app_publishers(
        &self,
        trusted_owner: &str,
    ) -> Result<Vec<PublisherTrustEntry>, AppPublisherError> {
        validate_owner(trusted_owner)?;
        let mut connection = self.lock_connection("durable_state.list_app_publishers")?;
        Ok(PublisherTrustRegistry::new(&mut connection).list(trusted_owner)?)
    }

    /// Verification input only. The installer must recheck this snapshot in the
    /// existing writer transaction before publishing an installation stage.
    pub(crate) fn trusted_app_publisher(
        &self,
        trusted_owner: &str,
        publisher_id: &str,
        key_id: &str,
    ) -> Result<TrustedPublisherSnapshot, AppPublisherError> {
        validate_owner(trusted_owner)?;
        let mut connection = self.lock_connection("durable_state.read_app_publisher")?;
        Ok(
            PublisherTrustRegistry::new(&mut connection).trusted_publisher(
                trusted_owner,
                publisher_id,
                key_id,
            )?,
        )
    }
}

pub(super) fn initialize(connection: &mut Connection) -> Result<(), DaemonError> {
    PublisherTrustRegistry::new(connection)
        .initialize()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.migrate_app_publishers",
            message: error.to_string(),
        })
}

pub(super) fn execute(connection: &mut Connection, request: AppPublisherRequest) {
    // Run between ordinary batches. Conflicts roll back only this decision;
    // earlier unrelated writes have already completed their normal transaction.
    let mut registry = PublisherTrustRegistry::new(connection);
    let result = match request.mutation {
        AppPublisherMutation::Enroll {
            publisher,
            expected_revision,
            decision,
            now_ms,
        } => registry.enroll(
            &request.owner_id,
            &publisher,
            expected_revision,
            &decision,
            now_ms,
        ),
        AppPublisherMutation::Revoke {
            publisher_id,
            key_id,
            expected_revision,
            decision,
            now_ms,
        } => registry.revoke(
            &request.owner_id,
            &publisher_id,
            &key_id,
            expected_revision,
            &decision,
            now_ms,
        ),
    };
    let _ = request.response.send(result);
}

fn validate_owner(owner: &str) -> Result<(), PublisherTrustError> {
    if owner.trim().is_empty() || owner.len() > 128 || owner.chars().any(char::is_control) {
        return Err(PublisherTrustError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
