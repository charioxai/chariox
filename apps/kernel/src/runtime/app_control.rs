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
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod workers;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
pub(crate) use workers::AppWorkerPublisher;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod catalog;

#[derive(Clone)]
pub(crate) struct AppControlService {
    store: DurableKernelStateStore,
    uploads: super::app_package_upload_control::AppPackageUploadControl,
    preparation: super::app_package_preparation::AppPackagePreparation,
    admission: Arc<Semaphore>,
    event_pump: super::app_event_pump::AppEventPump,
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    workers: workers::ActiveWorkers,
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    lifecycle: super::app_lifecycle::AppLifecycleService,
}

impl AppControlService {
    pub(crate) fn new(store: DurableKernelStateStore) -> Self {
        let uploads = super::app_package_upload_control::AppPackageUploadControl::new(
            store.path().to_path_buf(),
        );
        let admission = Arc::new(Semaphore::new(8));
        let event_pump = super::app_event_pump::AppEventPump::new();
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        let workers = workers::ActiveWorkers::default();
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        let lifecycle = super::app_lifecycle::AppLifecycleService::new(
            store.clone(),
            admission.clone(),
            AppWorkerPublisher::new(workers.clone(), event_pump.clone()),
        );
        Self {
            preparation: super::app_package_preparation::AppPackagePreparation::new(
                store.clone(),
                uploads.clone(),
            ),
            uploads,
            store,
            admission,
            event_pump,
            #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
            workers,
            #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
            lifecycle,
        }
    }

    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    pub(crate) fn lifecycle(&self) -> &super::app_lifecycle::AppLifecycleService {
        &self.lifecycle
    }

    pub(crate) fn schedule_maintenance(&self) {
        self.uploads.schedule_maintenance(&self.admission);
    }

    pub(crate) fn event_pump(&self) -> &super::app_event_pump::AppEventPump {
        &self.event_pump
    }

    pub(crate) fn try_admit(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, AppRequestErrorCode> {
        Arc::clone(&self.admission)
            .try_acquire_owned()
            .map_err(|_| AppRequestErrorCode::Busy)
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
