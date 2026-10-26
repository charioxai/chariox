//! One kernel development path: build, pack, upload, then the normal installer.
//! No watcher or runtime-state reference is retained by a build job.
mod build;

use super::{
    app_install_control::AppInstallControl,
    app_package_upload_control::{AppPackageUploadControl, UploadCommand},
};
use crate::{
    durable_state::DurableKernelStateStore,
    local::*,
    runtime::{command::KernelCommand, state::KernelRuntimeState},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{sync::Semaphore, task::JoinSet};

type Key = (String, String);
type Entries = Arc<Mutex<BTreeMap<Key, Entry>>>;
struct Entry {
    session: String,
    directory: String,
    updated: Instant,
    phase: Phase,
}
enum Phase {
    Building,
    Installing,
    Failed(Failure),
}

struct Failure {
    code: AppDevPushFailureCode,
    message: String,
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            code: AppDevPushFailureCode::BuildFailed,
            message,
        }
    }
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

#[derive(Clone)]
pub(crate) struct AppDevPushControl {
    entries: Entries,
    jobs: Arc<Mutex<JoinSet<()>>>,
    stopped: Arc<AtomicBool>,
    build_slot: Arc<Semaphore>,
    admission: Arc<Semaphore>,
    uploads: AppPackageUploadControl,
    installs: AppInstallControl,
    store: DurableKernelStateStore,
}

impl AppDevPushControl {
    pub(crate) fn room_available(&self, owner: &str, session: &str) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|((entry_owner, _), entry)| entry_owner == owner && entry.session == session)
    }
    pub(crate) fn new(
        store: DurableKernelStateStore,
        uploads: AppPackageUploadControl,
        installs: AppInstallControl,
        admission: Arc<Semaphore>,
    ) -> Self {
        Self {
            entries: Default::default(),
            jobs: Arc::new(Mutex::new(JoinSet::new())),
            stopped: Arc::new(AtomicBool::new(false)),
            build_slot: Arc::new(Semaphore::new(1)),
            admission,
            uploads,
            installs,
            store,
        }
    }

    pub(crate) async fn execute(
        &self,
        runtime: &KernelRuntimeState,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Option<LocalDaemonResponse> {
        let (session, id, directory) = match request {
            LocalDaemonRequest::BeginAppDevPush(value) => {
                (&value.session_id, &value.request_id, Some(&value.directory))
            }
            LocalDaemonRequest::GetAppDevPush(value) => {
                (&value.session_id, &value.request_id, None)
            }
            _ => return None,
        };
        let owner = match super::app_control::owner(command) {
            Ok(owner) => owner,
            Err(code) => return Some(failed(code)),
        };
        if !identity(session)
            || !identity(id)
            || directory.is_some_and(|path| {
                path.is_empty()
                    || path.len() > 4096
                    || path.contains('\0')
                    || !std::path::Path::new(path).is_absolute()
            })
        {
            return Some(failed(AppRequestErrorCode::InvalidRequest));
        }
        if !runtime.app_install_session_member(session, &owner) {
            return Some(failed(AppRequestErrorCode::Unauthorized));
        }
        Some(
            self.request(owner, session.clone(), id.clone(), directory.cloned())
                .await,
        )
    }

    /// The caller is already authenticated, including its current Room. Agents
    /// use this same method after checking that this Room is a developer Room.
    pub(crate) async fn request(
        &self,
        owner: String,
        session: String,
        id: String,
        directory: Option<String>,
    ) -> LocalDaemonResponse {
        if !identity(&session)
            || !identity(&id)
            || directory.as_ref().is_some_and(|path| {
                path.is_empty()
                    || path.len() > 4096
                    || path.contains('\0')
                    || !std::path::Path::new(path).is_absolute()
            })
        {
            return failed(AppRequestErrorCode::InvalidRequest);
        }
        if self.stopped.load(Ordering::Acquire) {
            return failed(AppRequestErrorCode::StorageUnavailable);
        }
        let Some(parent) = self.store.path().parent() else {
            return failed(AppRequestErrorCode::StorageUnavailable);
        };
        let scratch = parent.join("app-dev-builds");
        let key = (owner.clone(), id.clone());
        {
            let entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(entry) = entries.get(&key) {
                if entry.session != session
                    || directory
                        .as_ref()
                        .is_some_and(|path| path != &entry.directory)
                {
                    return failed(AppRequestErrorCode::Conflict);
                }
                match &entry.phase {
                    Phase::Building => {
                        return LocalDaemonResponse::AppDevPushBuilding { request_id: id }
                    }
                    Phase::Failed(error) => {
                        return LocalDaemonResponse::AppDevPushFailed {
                            request_id: id,
                            code: error.code,
                            message: error.message.clone(),
                        }
                    }
                    Phase::Installing => {}
                }
            }
        }
        match self.installs.dev_push_status(&owner, &session, &id).await {
            Ok(Some(value)) => return value,
            Err(code) => return failed(code),
            Ok(None) => {}
        }
        let Some(directory) = directory else {
            return failed(AppRequestErrorCode::NotFound);
        };
        let Ok(slot) = self.build_slot.clone().try_acquire_owned() else {
            return failed(AppRequestErrorCode::Busy);
        };
        let mut jobs = self
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.stopped.load(Ordering::Acquire) {
            return failed(AppRequestErrorCode::StorageUnavailable);
        }
        while jobs.try_join_next().is_some() {}
        {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Recheck after the asynchronous durable replay: another begin may
            // have won. A request id always names its first source revision.
            if let Some(entry) = entries.get(&key) {
                return if entry.session == session && entry.directory == directory {
                    LocalDaemonResponse::AppDevPushBuilding { request_id: id }
                } else {
                    failed(AppRequestErrorCode::Conflict)
                };
            }
            entries.retain(|_, value| {
                matches!(value.phase, Phase::Building)
                    || value.updated.elapsed() < Duration::from_secs(1800)
            });
            if entries.len() >= 64 {
                // Finished builds are replay history, not concurrent work. Keep
                // it bounded without imposing a push rate limit across Rooms.
                let oldest = entries
                    .iter()
                    .filter(|(_, entry)| !matches!(entry.phase, Phase::Building))
                    .min_by_key(|(_, entry)| entry.updated)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    entries.remove(&oldest);
                } else {
                    return failed(AppRequestErrorCode::Busy);
                }
            }
            entries.insert(
                key.clone(),
                Entry {
                    session: session.clone(),
                    directory: directory.clone(),
                    updated: Instant::now(),
                    phase: Phase::Building,
                },
            );
        }
        let entries = self.entries.clone();
        let stopped = self.stopped.clone();
        let (uploads, installs, admission) = (
            self.uploads.clone(),
            self.installs.clone(),
            self.admission.clone(),
        );
        let response_id = id.clone();
        jobs.spawn(async move {
            let _slot = slot;
            let build = tokio::task::spawn_blocking(move || build::build(&PathBuf::from(directory), &scratch, &stopped)).await;
            let result = match build {
                Ok(Ok(build)) => stage(&owner, &session, &id, build, uploads, installs, admission).await.map_err(|message| Failure { code: AppDevPushFailureCode::InstallFailed, message }),
                Ok(Err(error)) => Err(error),
                Err(_) => Err("App compiler task failed. The previous generation is still available; fix the build and save again.".into()),
            };
            if let Some(entry) = entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_mut(&key) {
                entry.updated = Instant::now();
                entry.phase = match result { Ok(()) => Phase::Installing, Err(error) => Phase::Failed(error) };
            }
        });
        LocalDaemonResponse::AppDevPushBuilding {
            request_id: response_id,
        }
    }

    pub(crate) fn shutdown_blocking(&self, runtime: tokio::runtime::Handle) {
        self.stopped.store(true, Ordering::Release);
        let mut jobs = self
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        runtime.block_on(async { while jobs.join_next().await.is_some() {} });
    }
}

async fn stage(
    owner: &str,
    session: &str,
    id: &str,
    build: build::Build,
    uploads: AppPackageUploadControl,
    installs: AppInstallControl,
    admission: Arc<Semaphore>,
) -> Result<(), String> {
    let upload = |command| {
        let admission = admission.clone();
        let uploads = uploads.clone();
        async move {
            let permit = tokio::time::timeout(Duration::from_secs(5), admission.acquire_owned())
                .await
                .map_err(|_| "Kernel App admission is busy; save again to retry.".to_owned())?
                .map_err(|_| "Kernel App admission stopped.".to_owned())?;
            uploads
                .execute(owner.to_owned(), command, permit)
                .await
                .map_err(|error| {
                    format!("App package staging failed ({error:?}); save again to retry.")
                })
        }
    };
    let status = upload(UploadCommand::Begin {
        request_id: format!("dev-{id}"),
        expected_size: build.bytes.len() as u64,
        sha256: build.digest.clone(),
    })
    .await?;
    let handle = status.handle;
    let result = async {
        for (index, bytes) in build.bytes.chunks(chariox_app_runtime::package_upload::MAX_UPLOAD_CHUNK_BYTES).enumerate() {
            upload(UploadCommand::Chunk { handle: handle.clone(), offset: (index * chariox_app_runtime::package_upload::MAX_UPLOAD_CHUNK_BYTES) as u64, data_base64: STANDARD.encode(bytes), chunk_sha256: format!("sha256:{:x}", Sha256::digest(bytes)) }).await?;
        }
        installs.begin_built_dev_push(owner.to_owned(), session.to_owned(), id.to_owned(), handle.clone(), build.digest, build.app_id).await
            .map_err(|code| format!("App installation could not start ({code:?}); the previous generation is unchanged. Save again to retry."))
    }.await;
    if result.is_err() {
        let _ = upload(UploadCommand::Abort { handle }).await;
    }
    result
}

fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 120 && !value.chars().any(char::is_control)
}
fn failed(code: AppRequestErrorCode) -> LocalDaemonResponse {
    LocalDaemonResponse::AppRequestFailed { code }
}
