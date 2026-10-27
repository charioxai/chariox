//! Owner-scoped App control over the shared kernel command path. Blocking
//! SQLite work has bounded admission and never holds the runtime coordinator.

use std::sync::Arc;

use chariox_app_runtime::installation::{CapabilityDecision, InstallationError, UpdatePhase};
use tokio::sync::Semaphore;

use crate::durable_state::{apps::AppRegistryError, DurableKernelStateStore};
use crate::local::*;
use crate::runtime::command::{KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::session::DEFAULT_LOCAL_USER_ID;

#[cfg(test)]
mod fixture_storage;
mod projection;
mod request_receipts;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use fixture_storage::FixtureAppStorage;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod readiness;
mod uploads;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod workers;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
pub(crate) use workers::AppWorkerPublisher;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod catalog;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod first_install;
#[cfg(all(
    test,
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
pub(crate) use first_install::FirstInstallControlError;

/// See `AppControlService::admit_reply`.
const REPLY_ADMISSION_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Wait briefly, but never past the call's own deadline.
fn reply_wait(remaining: std::time::Duration) -> std::time::Duration {
    remaining.min(REPLY_ADMISSION_WAIT)
}

async fn admit_within(
    admission: &Arc<Semaphore>,
    wait: std::time::Duration,
) -> Result<tokio::sync::OwnedSemaphorePermit, AppRequestErrorCode> {
    tokio::time::timeout(wait, Arc::clone(admission).acquire_owned())
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or(AppRequestErrorCode::Busy)
}

#[derive(Clone)]
#[allow(
    clippy::type_complexity,
    reason = "Keep the explicit AppControlService state or return type at the existing boundary"
)]
pub(crate) struct AppControlService {
    store: DurableKernelStateStore,
    request_receipts: request_receipts::AppRequestReceipts,
    uploads: super::app_package_upload_control::AppPackageUploadControl,
    preparation: super::app_package_preparation::AppPackagePreparation,
    admission: Arc<Semaphore>,
    event_pump: super::app_event_pump::AppEventPump,
    wake_pump: super::app_wake_pump::AppWakePump,
    wake_scheduler: Arc<Semaphore>,
    views: super::app_views::AppViews,
    user_views: super::user_app_views::UserAppViews,
    validation_pump: super::app_wake_pump::AppWakePump,
    /// Shown prompts: operation → (owner, the session showing it once known).
    validation_prompts:
        Arc<std::sync::Mutex<std::collections::BTreeMap<String, (String, Option<String>)>>>,
    publishers: super::app_publisher_control::AppPublisherControl,
    /// Agents with a delayed App catalog refresh pending (at most one each).
    catalog_refreshes: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    workers: workers::ActiveWorkers,
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    lifecycle: super::app_lifecycle::AppLifecycleService,
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    installs: super::app_install_control::AppInstallControl,
    /// Tests only: the App storage uninstall deletes from, once a test gave
    /// this kernel one; until then, the platform's.
    #[cfg(test)]
    fixture_storage: Arc<std::sync::OnceLock<FixtureAppStorage>>,
}

impl AppControlService {
    pub(crate) fn new(store: DurableKernelStateStore) -> Self {
        let uploads = super::app_package_upload_control::AppPackageUploadControl::new(
            store.path().to_path_buf(),
        );
        let admission = Arc::new(Semaphore::new(8));
        let event_pump = super::app_event_pump::AppEventPump::new();
        let publishers = super::app_publisher_control::AppPublisherControl::new(
            store.clone(),
            admission.clone(),
        );
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        let workers = workers::ActiveWorkers::default();
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        let lifecycle = super::app_lifecycle::AppLifecycleService::new(
            store.clone(),
            admission.clone(),
            AppWorkerPublisher::new(workers.clone(), event_pump.clone()),
        );
        let preparation = super::app_package_preparation::AppPackagePreparation::new(
            store.clone(),
            uploads.clone(),
        );
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        let installs = super::app_install_control::AppInstallControl::new(
            store.clone(),
            preparation.clone(),
            admission.clone(),
            lifecycle.clone(),
        );
        let request_receipts = request_receipts::AppRequestReceipts::new(
            store.path().with_extension("app-command-results.jsonl"),
        );
        Self {
            request_receipts,
            preparation,
            #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
            installs,
            uploads,
            store,
            admission,
            event_pump,
            wake_pump: Default::default(),
            wake_scheduler: Arc::new(Semaphore::new(1)),
            views: Default::default(),
            user_views: Default::default(),
            validation_pump: Default::default(),
            validation_prompts: Default::default(),
            publishers,
            catalog_refreshes: Default::default(),
            #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
            workers,
            #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
            lifecycle,
            #[cfg(test)]
            fixture_storage: Default::default(),
        }
    }

    /// Tests only: from now on this kernel deletes App storage from the
    /// returned fixture instead of the platform's. Every call returns the same
    /// fixture.
    #[cfg(test)]
    pub(crate) fn fixture_app_storage(&self) -> FixtureAppStorage {
        self.fixture_storage.get_or_init(Default::default).clone()
    }

    /// Tests only: the fixture storage, once `fixture_app_storage` gave one.
    #[cfg(test)]
    pub(crate) fn fixture_storage(&self) -> Option<&FixtureAppStorage> {
        self.fixture_storage.get()
    }

    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    pub(crate) fn lifecycle(&self) -> &super::app_lifecycle::AppLifecycleService {
        &self.lifecycle
    }

    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    pub(crate) fn installs(&self) -> &super::app_install_control::AppInstallControl {
        &self.installs
    }

    /// Protocol 367: verifies deployment releases from the local release store.
    pub(crate) fn preparation(&self) -> &super::app_package_preparation::AppPackagePreparation {
        &self.preparation
    }

    pub(crate) fn schedule_maintenance(&self) {
        self.uploads.schedule_maintenance(&self.admission);
    }

    pub(crate) fn publishers(&self) -> &super::app_publisher_control::AppPublisherControl {
        &self.publishers
    }

    pub(crate) fn event_pump(&self) -> &super::app_event_pump::AppEventPump {
        &self.event_pump
    }

    /// Shared across runtime clones: only one deadline delivery lane may run.
    pub(crate) fn reserve_wake_scheduler(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.wake_scheduler.clone().try_acquire_owned().ok()
    }

    pub(crate) fn wake_pump(&self) -> &super::app_wake_pump::AppWakePump {
        &self.wake_pump
    }

    pub(crate) fn user_views(&self) -> &super::user_app_views::UserAppViews {
        &self.user_views
    }

    pub(crate) fn views(&self) -> &super::app_views::AppViews {
        &self.views
    }

    pub(crate) fn validation_pump(&self) -> &super::app_wake_pump::AppWakePump {
        &self.validation_pump
    }

    /// At most one live approval per validation operation, and a few per
    /// owner, so App validations never take the owner's whole share of kernel
    /// decisions (install and publisher approvals keep room).
    pub(crate) fn begin_validation_prompt(&self, operation_id: &str, owner: &str) -> bool {
        const PER_OWNER: usize = 4;
        // Half of the kernel-wide decision limit (32) stays for other kinds.
        const TOTAL: usize = 16;
        let mut prompts = self
            .validation_prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if prompts.contains_key(operation_id)
            || prompts.len() >= TOTAL
            || prompts.values().filter(|(live, _)| live == owner).count() >= PER_OWNER
        {
            return false;
        }
        prompts.insert(operation_id.to_owned(), (owner.to_owned(), None));
        true
    }

    /// Records which session shows the prompt, so an answer from any other
    /// session closes it there.
    pub(crate) fn show_validation_prompt(&self, operation_id: &str, session: &str) {
        if let Some(prompt) = self
            .validation_prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(operation_id)
        {
            prompt.1 = Some(session.to_owned());
        }
    }

    pub(crate) fn validation_prompt_session(&self, operation_id: &str) -> Option<String> {
        self.validation_prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(operation_id)?
            .1
            .clone()
    }

    pub(crate) fn end_validation_prompt(&self, operation_id: &str) {
        self.validation_prompts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(operation_id);
    }

    pub(crate) fn admission(&self) -> Arc<Semaphore> {
        Arc::clone(&self.admission)
    }

    pub(crate) fn try_admit(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, AppRequestErrorCode> {
        Arc::clone(&self.admission)
            .try_acquire_owned()
            .map_err(|_| AppRequestErrorCode::Busy)
    }

    /// Admission for recording a call the App already answered: it waits
    /// briefly instead of failing at once, so momentary contention does not
    /// throw away a completed call's result. It never waits past the call's
    /// own deadline (`remaining`), after which the answer would be refused
    /// as late although the App did answer.
    pub(crate) async fn admit_reply(
        &self,
        remaining: std::time::Duration,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, AppRequestErrorCode> {
        admit_within(&self.admission, reply_wait(remaining)).await
    }

    /// Waits for an App admission slot.
    pub(crate) async fn admit(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.admission).acquire_owned().await.ok()
    }

    /// True when the caller should schedule the agent's delayed App catalog
    /// refresh: none is pending yet.
    pub(crate) fn begin_catalog_refresh(&self, agent: &str) -> bool {
        self.catalog_refreshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(agent.to_owned())
    }

    pub(crate) fn end_catalog_refresh(&self, agent: &str) {
        self.catalog_refreshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(agent);
    }

    #[cfg(test)]
    pub(crate) fn catalog_refresh_pending(&self, agent: &str) -> bool {
        self.catalog_refreshes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(agent)
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

pub(crate) fn owner(command: &KernelCommand) -> Result<String, AppRequestErrorCode> {
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
            // Protocol 367: deployment copies belong to their deployment.
            let page = store
                .list_app_installations_without_copies(owner, request.after.as_deref(), limit)
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

pub(crate) fn installation_summary(
    value: chariox_app_runtime::installation::Installation,
) -> AppInstallationSummary {
    projection::installation(value)
}

pub(crate) fn registry_error(error: AppRegistryError) -> AppRequestErrorCode {
    match error {
        AppRegistryError::Registry(InstallationError::NotFound) => AppRequestErrorCode::NotFound,
        // Caller fields were validated before the read. Residual invariant
        // failures describe persisted state, not a request the user can fix.
        _ => AppRequestErrorCode::StorageUnavailable,
    }
}

pub(crate) fn failed(code: AppRequestErrorCode) -> LocalDaemonResponse {
    LocalDaemonResponse::AppRequestFailed { code }
}
