//! Owner-scoped App control over the shared kernel command path. Blocking
//! SQLite work has bounded admission and never holds the runtime coordinator.

use std::sync::Arc;

use chariox_app_runtime::installation::{CapabilityDecision, InstallationError, UpdatePhase};
use tokio::sync::Semaphore;

use crate::durable_state::{apps::AppRegistryError, DurableKernelStateStore};
use crate::local::*;
use crate::runtime::command::{KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::session::DEFAULT_LOCAL_USER_ID;

mod projection;
#[cfg(test)]
mod tests;
mod uploads;

#[derive(Clone)]
pub(crate) struct AppControlService {
    store: DurableKernelStateStore,
    uploads: super::app_package_upload_control::AppPackageUploadControl,
    admission: Arc<Semaphore>,
}

impl AppControlService {
    pub(crate) fn new(store: DurableKernelStateStore) -> Self {
        Self {
            uploads: super::app_package_upload_control::AppPackageUploadControl::new(
                store.path().to_path_buf(),
            ),
            store,
            admission: Arc::new(Semaphore::new(8)),
        }
    }

    pub(crate) async fn execute(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Option<LocalDaemonResponse> {
        if !matches!(
            request,
            LocalDaemonRequest::ListAppInstallations(_)
                | LocalDaemonRequest::GetAppInstallation(_)
                | LocalDaemonRequest::GetAppInstallationJournal(_)
                | LocalDaemonRequest::BeginAppPackageUpload(_)
                | LocalDaemonRequest::PutAppPackageUploadChunk(_)
                | LocalDaemonRequest::GetAppPackageUpload(_)
                | LocalDaemonRequest::AbortAppPackageUpload(_)
        ) {
            return None;
        }
        let owner = match owner(command) {
            Ok(owner) => owner,
            Err(code) => return Some(failed(code)),
        };
        let permit = match Arc::clone(&self.admission).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return Some(failed(AppRequestErrorCode::Busy)),
        };
        match uploads::command(request) {
            Ok(Some(upload)) => {
                return Some(match self.uploads.execute(owner, upload, permit).await {
                    Ok(status) => uploads::response(status),
                    Err(error) => failed(uploads::error_code(error)),
                });
            }
            Err(code) => return Some(failed(code)),
            Ok(None) => {}
        }
        let store = self.store.clone();
        let request = request.clone();
        Some(
            match tokio::task::spawn_blocking(move || {
                // Kept by the task even if the client disconnects or cancels await.
                let _permit = permit;
                read(&store, &owner, request).unwrap_or_else(failed)
            })
            .await
            {
                Ok(response) => response,
                Err(_) => failed(AppRequestErrorCode::StorageUnavailable),
            },
        )
    }
}

fn owner(command: &KernelCommand) -> Result<String, AppRequestErrorCode> {
    let caller = &command.caller;
    if matches!(caller.caller_kind, KernelCallerKind::HostedService) {
        return Err(AppRequestErrorCode::Unauthorized);
    }
    if let Some(owner) = caller.user_id.as_deref() {
        if valid_identity(owner) {
            return Ok(owner.to_owned());
        }
        return Err(AppRequestErrorCode::Unauthorized);
    }
    if matches!(
        command.source,
        KernelCommandSource::LocalCli | KernelCommandSource::LocalIpc
    ) && matches!(caller.caller_kind, KernelCallerKind::LocalClient)
    {
        return Ok(DEFAULT_LOCAL_USER_ID.to_owned());
    }
    // Unverified remote callers must never inherit the default local identity.
    Err(AppRequestErrorCode::Unauthorized)
}

fn valid_identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn read(
    store: &DurableKernelStateStore,
    owner: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
    match request {
        LocalDaemonRequest::ListAppInstallations(request) => {
            let limit = request.limit.unwrap_or(50) as usize;
            if !(1..=100).contains(&limit)
                || request
                    .after
                    .as_deref()
                    .is_some_and(|id| !valid_identity(id))
            {
                return Err(AppRequestErrorCode::InvalidRequest);
            }
            let page = store
                .list_app_installations(owner, request.after.as_deref(), limit)
                .map_err(registry_error)?;
            Ok(LocalDaemonResponse::AppInstallationsListed {
                installations: page
                    .installations
                    .into_iter()
                    .map(projection::installation)
                    .collect(),
                next_cursor: page.next_cursor,
            })
        }
        LocalDaemonRequest::GetAppInstallation(request) => {
            if !valid_identity(&request.installation_id) {
                return Err(AppRequestErrorCode::InvalidRequest);
            }
            let installation = store
                .get_app_installation(owner, &request.installation_id)
                .map_err(registry_error)?;
            Ok(LocalDaemonResponse::AppInstallation {
                installation: projection::installation(installation),
            })
        }
        LocalDaemonRequest::GetAppInstallationJournal(request) => {
            if !valid_identity(&request.installation_id) {
                return Err(AppRequestErrorCode::InvalidRequest);
            }
            let updates = store
                .app_installation_journal(owner, &request.installation_id)
                .map_err(registry_error)?;
            Ok(LocalDaemonResponse::AppInstallationJournal {
                installation_id: request.installation_id,
                updates: updates.into_iter().map(projection::update).collect(),
            })
        }
        _ => Err(AppRequestErrorCode::InvalidRequest),
    }
}

fn registry_error(error: AppRegistryError) -> AppRequestErrorCode {
    match error {
        AppRegistryError::Registry(InstallationError::NotFound) => AppRequestErrorCode::NotFound,
        // Caller fields were validated before the read. Residual invariant
        // failures describe persisted state, not a request the user can fix.
        _ => AppRequestErrorCode::StorageUnavailable,
    }
}

fn failed(code: AppRequestErrorCode) -> LocalDaemonResponse {
    LocalDaemonResponse::AppRequestFailed { code }
}
