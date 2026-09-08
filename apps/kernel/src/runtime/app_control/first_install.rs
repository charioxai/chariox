//! Internal upload -> verified preparation -> first-install receipt. Public
//! protocol and RuntimeInteraction decision routing are a coordinated next step.
use super::*;
use crate::{
    durable_state::app_installation_operations::{InstallOperation, InstallOperationError},
    runtime::{
        app_operation_budget::AppOperationBudget, app_package_preparation::PreparationError,
    },
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum FirstInstallControlError {
    #[error("app_install_busy")]
    Busy,
    #[error("app_install_invalid")]
    Invalid,
    #[error("app_install_preparation:{0:?}")]
    Preparation(PreparationError),
    #[error(transparent)]
    Operation(#[from] InstallOperationError),
}
impl AppControlService {
    /// owner is the authenticated kernel caller, separate from future wire
    /// fields. Neither upload metadata nor an App can assert that identity.
    pub(crate) async fn prepare_first_install(
        &self,
        owner: String,
        request_id: String,
        upload_handle: String,
        expected_package_digest: String,
    ) -> Result<InstallOperation, FirstInstallControlError> {
        if !valid_identity(&owner)
            || !valid_identity(&request_id)
            || expected_package_digest.len() != 71
            || !expected_package_digest
                .strip_prefix("sha256:")
                .is_some_and(|digest| {
                    digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
        {
            return Err(FirstInstallControlError::Invalid);
        }
        let permit = self
            .try_admit()
            .map_err(|_| FirstInstallControlError::Busy)?;
        let lookup = self.store.clone();
        let lookup_owner = owner.clone();
        let lookup_request = request_id.clone();
        let lookup_digest = expected_package_digest.clone();
        let prior = tokio::task::spawn_blocking(move || {
            let prior = lookup.replay_first_app_install(
                &lookup_owner,
                &lookup_request,
                &lookup_digest,
                AppOperationBudget::from_supervisor(|| false),
            );
            if matches!(prior, Err(InstallOperationError::CommitUnknown)) {
                let _ = lookup.fence_writer();
            }
            (prior, permit)
        })
        .await
        .map_err(|_| FirstInstallControlError::Operation(InstallOperationError::Storage))?;
        let (prior, permit) = prior;
        match prior {
            Ok(operation) if operation.package_digest == expected_package_digest => {
                return Ok(operation)
            }
            Ok(_) => return Err(InstallOperationError::Conflict.into()),
            Err(InstallOperationError::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
        let prepared = self
            .preparation
            .prepare(owner.clone(), upload_handle, permit)
            .await
            .map_err(FirstInstallControlError::Preparation)?;
        let permit = self
            .try_admit()
            .map_err(|_| FirstInstallControlError::Busy)?;
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            // Retain the prepared release through the same writer's stage and
            // operation receipt. No worker slot is held while approval waits.
            let candidate = prepared
                .candidate(&owner)
                .map_err(FirstInstallControlError::Preparation)?
                .clone();
            if candidate.release_metadata().package_digest != expected_package_digest {
                return Err(InstallOperationError::Conflict.into());
            }
            let result = store.begin_first_app_install(
                &owner,
                &request_id,
                candidate,
                AppOperationBudget::from_supervisor(|| false),
            );
            if matches!(result, Err(InstallOperationError::CommitUnknown)) {
                let _ = store.fence_writer();
            }
            result.map_err(Into::into)
        })
        .await
        .map_err(|_| FirstInstallControlError::Operation(InstallOperationError::CommitUnknown))?
    }
}
