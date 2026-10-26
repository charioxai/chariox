//! Retained owners for approved first installs and active-generation restarts.
//! Data migrations and terminal approval projection remain separate duties.
mod authority_check;
mod disk_space;
mod first_install;
mod manual_stop;
mod notifications;
mod operations;
mod owner;
mod ownership;
mod recovery;
mod restore;
mod start;
#[cfg(target_os = "macos")]
pub(crate) use start::macos_storage_root;
#[cfg(test)]
pub(crate) mod tests;
mod uninstall;
use crate::{
    durable_state::{
        app_installation_operations::{ApprovedFirstInstall, InstallOperationError, InstallPhase},
        app_worker_lifecycle::{ActiveStartAdmission, LifecycleStoreError, WorkerPhase},
        DurableKernelStateStore,
    },
    runtime::{app_control::AppWorkerPublisher, app_operation_budget::AppOperationBudget},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore},
};

const LIVE_LIMIT: usize = 4;
const RECOVERY_INTERVAL: Duration = Duration::from_secs(5);
type Key = (String, String);
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LifecycleError {
    #[error("app_lifecycle_busy")]
    Busy,
    /// Every live worker slot is taken; only this is relieved by stopping an
    /// idle worker. Other `Busy` causes clear by themselves.
    #[error("app_lifecycle_live_limit")]
    LiveLimit,
    #[error("app_lifecycle_stopped")]
    Stopped,
    /// Work must wait through restart backoff.
    #[error("app_lifecycle_restart_deferred")]
    RestartDeferred,
    #[error("app_lifecycle_quarantined")]
    Quarantined,
    #[error("app_lifecycle_authority")]
    Authority,
    #[error("app_lifecycle_storage")]
    Storage,
    #[error("app_lifecycle_preparation")]
    Preparation,
    /// App storage would leave less than the host's reserve free. The App's
    /// log says how much is free and how much to free (`disk_space`).
    #[error("app_lifecycle_disk_space")]
    DiskSpace(chariox_app_runtime::worker_process::HostDiskSpace),
    #[error("app_lifecycle_registration")]
    Registration,
    #[error("app_lifecycle_registration_cancelled")]
    RegistrationCancelled,
    /// Readiness exceeded its bounded wait. The owner is reaped; a fresh
    /// explicit start can retry the committed generation on the same kernel.
    #[error("app_lifecycle_registration_deadline")]
    RegistrationDeadline,
    #[error("app_lifecycle_health")]
    Health,
    #[error("app_install_commit_unknown")]
    CommitUnknown,
    #[error("app_lifecycle_startup")]
    Startup,
    #[error("app_lifecycle_notification")]
    Notification,
    #[error("app_lifecycle_notification_not_dispatched")]
    NotificationNotDispatched,
    #[error("app_lifecycle_worker_exit")]
    #[allow(dead_code)] // Existing worker lifecycle error vocabulary.
    WorkerExit,
    #[error("app_lifecycle_supervisor")]
    Supervisor,
}
impl From<LifecycleStoreError> for LifecycleError {
    fn from(value: LifecycleStoreError) -> Self {
        match value {
            LifecycleStoreError::Storage => Self::Storage,
            LifecycleStoreError::Stopped => Self::Stopped,
            LifecycleStoreError::Stale => Self::Authority,
        }
    }
}
impl From<InstallOperationError> for LifecycleError {
    fn from(value: InstallOperationError) -> Self {
        match value {
            InstallOperationError::CommitUnknown => Self::CommitUnknown,
            InstallOperationError::Storage => Self::Storage,
            InstallOperationError::Stopped => Self::Stopped,
            InstallOperationError::Limit => Self::Busy,
            _ => Self::Authority,
        }
    }
}
#[derive(Clone)]
enum StartKind {
    Active {
        recovery: bool,
    },
    /// A supervised install operation. `replace` is a local update: the
    /// installation's current worker is drained (not user-stopped) first.
    First {
        request_id: String,
        replace: bool,
    },
}
type Result<T> = std::result::Result<T, LifecycleError>;
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StartDisposition {
    Starting { attempt: String },
    Existing { attempt: String },
}

#[derive(Clone)]
pub(crate) struct AppLifecycleService(Arc<Inner>);
struct Inner {
    http_limits: Arc<crate::runtime::app_http::HttpLimits>,
    /// The kernel's config, for App actions through event generator
    /// connections (protocol 359); attached once when the kernel starts.
    event_config: EventConfig,
    store: DurableKernelStateStore,
    publisher: AppWorkerPublisher,
    admission: Arc<Semaphore>,
    preparation: Arc<Semaphore>,
    live: Arc<Semaphore>,
    stopped: AtomicBool,
    entries: Mutex<BTreeMap<Key, Arc<Entry>>>,
    operations: Mutex<BTreeSet<Key>>,
    shutdown: Mutex<()>,
    maintenance: Mutex<Maintenance>,
    #[cfg(test)]
    fixture: Mutex<Option<start::FixturePlatform>>,
    #[cfg(test)]
    claim_checkpoint: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    start_checkpoint: Mutex<Option<StartObserver>>,
}
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum StartCheckpoint {
    BeforeClaim,
    BeforePublication,
}
#[cfg(test)]
type StartObserver = Arc<dyn Fn(StartCheckpoint) + Send + Sync>;
struct Maintenance {
    running: bool,
    next: Instant,
    cursor: Option<Key>,
    first_cursor: Option<Key>,
    first_next: bool,
}
struct Entry {
    attempt: String,
    control: Arc<Control>,
    thread: Mutex<Option<JoinHandle<()>>>,
}
struct Control {
    first_request: Option<String>,
    restore_data: Mutex<Option<chariox_app_runtime::worker_process::PrivateData>>,
    stop: AtomicBool,
    /// The stop is a local update replacing this owner's generation.
    update: AtomicBool,
    manual: AtomicBool,
    manual_committed: AtomicBool,
    /// Native ownership ended; terminal persistence/thread teardown may remain.
    retiring: AtomicBool,
    done: Mutex<bool>,
    wake: Condvar,
    drain: Mutex<Option<crate::runtime::app_worker::AppWorkerDrain>>,
    notification: Mutex<Option<notifications::Request>>,
    idle: AtomicBool,
    idle_requested: AtomicBool,
    #[cfg(test)]
    completion_checkpoint: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    idle_refusal_checkpoint: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}
pub(crate) struct Operation {
    inner: Arc<Inner>,
    key: Key,
}
pub(crate) type EventConfig =
    Arc<std::sync::OnceLock<crate::runtime::projection::DaemonConfigProjectionStore>>;

impl AppLifecycleService {
    pub(crate) fn attach_event_config(
        &self,
        config: crate::runtime::projection::DaemonConfigProjectionStore,
    ) {
        let _ = self.0.event_config.set(config);
    }

    pub(crate) fn new(
        store: DurableKernelStateStore,
        admission: Arc<Semaphore>,
        publisher: AppWorkerPublisher,
    ) -> Self {
        Self(Arc::new(Inner {
            http_limits: Arc::new(crate::runtime::app_http::HttpLimits::default()),
            event_config: EventConfig::default(),
            store,
            publisher,
            admission,
            preparation: Arc::new(Semaphore::new(1)),
            live: Arc::new(Semaphore::new(LIVE_LIMIT)),
            stopped: AtomicBool::new(false),
            entries: Mutex::new(BTreeMap::new()),
            operations: Mutex::new(BTreeSet::new()),
            shutdown: Mutex::new(()),
            maintenance: Mutex::new(Maintenance {
                running: false,
                next: Instant::now(),
                cursor: None,
                first_cursor: None,
                first_next: false,
            }),
            #[cfg(test)]
            fixture: Mutex::new(None),
            #[cfg(test)]
            claim_checkpoint: Mutex::new(None),
            #[cfg(test)]
            start_checkpoint: Mutex::new(None),
        }))
    }
}
