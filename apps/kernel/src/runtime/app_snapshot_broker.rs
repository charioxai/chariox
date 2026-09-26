//! `files.snapshot`: an App-requested copy of its private files and structured
//! state, kept by the kernel beside its database, outside App data. A
//! `quiescent` snapshot holds every SDK write of the worker (state, events,
//! wakes, file replacement and import) until the copy is done; the App itself
//! must pause its own `node:fs` writes. A `crash_consistent` one fences
//! nothing and is labelled so. Delivery receipts are never included.
use super::app_operation_budget::AppOperationBudget;
use crate::durable_state::DurableKernelStateStore;
use chariox_app_runtime::{
    app_outbox::EventCatalog,
    wire::RemoteError,
    worker_peer::BrokerRequest,
    worker_process::{PrivateData, TreeLimits},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{RwLock, Semaphore};

/// Snapshots kept per installation; a new one replaces the oldest.
pub(crate) const RETAINED: usize = 2;
/// The same bounds as the private data volume.
pub(crate) const LIMITS: TreeLimits = TreeLimits {
    files: 10_000,
    bytes: 512 * 1024 * 1024,
    depth: 32,
};
/// Free space the host keeps beyond a full-size snapshot.
const HOST_RESERVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// One snapshot at a time per installation, across workers: a copy never
/// races another's staging or retention.
static IN_PROGRESS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(Default::default);
/// Orders snapshots taken in the same millisecond.
static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    name: String,
    consistency: String,
}

#[derive(Clone)]
pub(crate) struct AppSnapshotBroker {
    store: DurableKernelStateStore,
    owner: String,
    catalog: Arc<EventCatalog>,
    admission: Arc<Semaphore>,
    data: PrivateData,
    fence: Arc<RwLock<()>>,
}

impl AppSnapshotBroker {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        owner: String,
        catalog: Arc<EventCatalog>,
        admission: Arc<Semaphore>,
        data: PrivateData,
        fence: Arc<RwLock<()>>,
    ) -> Self {
        Self {
            store,
            owner,
            catalog,
            admission,
            data,
            fence,
        }
    }

    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        let budget = AppOperationBudget::from_broker(&request);
        let params: Request =
            serde_json::from_value(request.params).map_err(|_| error("INVALID_ARGUMENT", false))?;
        let quiescent = match params.consistency.as_str() {
            "quiescent" => true,
            "crash_consistent" => false,
            _ => return Err(error("INVALID_ARGUMENT", false)),
        };
        if params.name.is_empty()
            || params.name.len() > 64
            || !params
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(error("INVALID_ARGUMENT", false));
        }
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("APP_BUSY", true))?;
        let serial = IN_PROGRESS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(self.catalog.installation_id().to_owned())
            .or_default()
            .clone();
        let serial = serial.lock_owned().await;
        // Held across the copy: in-flight SDK writes finish first, new ones wait.
        let fence = if quiescent {
            Some(self.fence.clone().write_owned().await)
        } else {
            None
        };
        budget
            .check()
            .map_err(|_| error("APP_OPERATION_STOPPED", false))?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _serial = serial;
            let _fence = fence;
            service.take(&params.name, &params.consistency, &budget)
        })
        .await
        .map_err(|_| error("STORAGE_UNAVAILABLE", true))?
    }

    fn take(
        &self,
        name: &str,
        consistency: &str,
        budget: &AppOperationBudget,
    ) -> Result<Value, RemoteError> {
        let installation = self.catalog.installation_id();
        let generation = self.catalog.generation();
        // Only the active generation snapshots: a staged update's data is not
        // committed yet.
        self.require_active()?;
        let root =
            root(&self.store, installation).map_err(|_| error("STORAGE_UNAVAILABLE", true))?;
        if free_bytes(&root).is_none_or(|free| free < LIMITS.bytes + HOST_RESERVE_BYTES) {
            return Err(error("LIMIT_EXCEEDED", false));
        }
        // Sortable by name: time, then order within this kernel.
        let id = format!(
            "snapshot-{:013}-{:08x}-{:016x}",
            crate::session::unix_epoch_ms(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            rand::random::<u64>()
        );
        let staging = root.join(format!(".tmp-{id}"));
        let result = (|| {
            private_dir(&staging)?;
            private_dir(&staging.join("files"))?;
            // Checked per file, so a stopped call releases the fence early.
            let mut stopped = false;
            let files = self
                .data
                .copy_tree(&staging.join("files"), LIMITS, &mut || {
                    stopped = budget.check().is_err();
                    !stopped
                })
                .map_err(|failure| {
                    use chariox_app_runtime::worker_process::PrivateDataError;
                    match failure {
                        _ if stopped => error("APP_OPERATION_STOPPED", false),
                        PrivateDataError::Invalid => error("LIMIT_EXCEEDED", false),
                        PrivateDataError::Identity => error("STORAGE_UNAVAILABLE", false),
                        _ => error("STORAGE_UNAVAILABLE", true),
                    }
                })?;
            budget
                .check()
                .map_err(|_| error("APP_OPERATION_STOPPED", false))?;
            let state = self
                .store
                .export_app_state(&self.owner, installation)
                .map_err(|code| error(code, true))?;
            self.require_active()?;
            let manifest = json!({
                "schema": "chariox.app-snapshot.v1",
                "snapshot_id": id,
                "name": name,
                "consistency": consistency,
                "installation_id": installation,
                "generation": generation,
                "package_digest": self.catalog.app_catalog().package_digest(),
                "created_at_ms": crate::session::unix_epoch_ms(),
                "files": files.files.iter().map(|file| json!({
                    "path": file.path, "bytes": file.bytes, "sha256": file.sha256,
                })).collect::<Vec<_>>(),
                "bytes": files.bytes,
                "skipped": files.skipped,
            });
            write_private(&staging.join("state.json"), &state)?;
            write_private(&staging.join("manifest.json"), &manifest)?;
            std::fs::rename(&staging, root.join(&id)).map_err(storage)?;
            sync_dir(&root)?;
            Ok(json!({
                "snapshotId": id,
                "consistency": consistency,
                "files": files.files.len(),
                "bytes": files.bytes,
            }))
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        } else {
            prune(&root);
        }
        result
    }

    fn require_active(&self) -> Result<(), RemoteError> {
        let installation = self
            .store
            .get_app_installation(&self.owner, self.catalog.installation_id())
            .map_err(|_| error("STORAGE_UNAVAILABLE", true))?;
        if installation.generation != self.catalog.generation() || installation.active.is_none() {
            return Err(error("CONFLICT", false));
        }
        Ok(())
    }
}

/// `<kernel state>/app-snapshots/<installation>`, created private.
pub(crate) fn root(
    store: &DurableKernelStateStore,
    installation: &str,
) -> std::io::Result<PathBuf> {
    let base = store
        .path()
        .parent()
        .ok_or(std::io::ErrorKind::NotFound)?
        .join("app-snapshots");
    private_dir(&base).map_err(|_| std::io::ErrorKind::PermissionDenied)?;
    let root = base.join(installation);
    private_dir(&root).map_err(|_| std::io::ErrorKind::PermissionDenied)?;
    // The copy opens it with no symlink in any ancestor (e.g. macOS /var).
    std::fs::canonicalize(root)
}

fn private_dir(path: &Path) -> Result<(), RemoteError> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(storage(error)),
    }
    let metadata = std::fs::symlink_metadata(path).map_err(storage)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(error("STORAGE_UNAVAILABLE", false));
    }
    Ok(())
}

fn write_private(path: &Path, value: &Value) -> Result<(), RemoteError> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(storage)?;
    file.write_all(&serde_json::to_vec(value).map_err(|_| error("STORAGE_UNAVAILABLE", false))?)
        .map_err(storage)?;
    file.sync_all().map_err(storage)
}

fn sync_dir(path: &Path) -> Result<(), RemoteError> {
    std::fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(storage)
}

/// Keeps the newest `RETAINED` snapshots (ids sort by time) and drops copies
/// a crash left unfinished; the caller holds the installation's snapshot lock.
fn prune(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut kept = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".tmp-") {
            let _ = std::fs::remove_dir_all(entry.path());
        } else {
            kept.push((name, entry.path()));
        }
    }
    kept.sort();
    let excess = kept.len().saturating_sub(RETAINED);
    for (_, path) in kept.into_iter().take(excess) {
        let _ = std::fs::remove_dir_all(path);
    }
}

fn free_bytes(path: &Path) -> Option<u64> {
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    let stat = unsafe { stat.assume_init() };
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

fn storage(_: std::io::Error) -> RemoteError {
    error("STORAGE_UNAVAILABLE", true)
}

fn error(code: &str, retryable: bool) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: code.to_ascii_lowercase().replace('_', " "),
        retryable: Some(retryable),
    }
}
