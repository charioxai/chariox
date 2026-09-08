//! Retained owners for approved first installs and active-generation restarts.
//! Data migrations and terminal approval projection remain separate duties.
mod first_install;
mod manual_stop;
mod operations;
mod owner;
mod ownership;
mod recovery;
mod start;
#[cfg(test)]
mod tests;
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
    #[error("app_lifecycle_stopped")]
    Stopped,
    #[error("app_lifecycle_authority")]
    Authority,
    #[error("app_lifecycle_storage")]
    Storage,
    #[error("app_lifecycle_preparation")]
    Preparation,
    #[error("app_lifecycle_registration")]
    Registration,
    #[error("app_lifecycle_health")]
    Health,
    #[error("app_install_commit_unknown")]
    CommitUnknown,
    #[error("app_lifecycle_startup")]
    Startup,
    #[error("app_lifecycle_worker_exit")]
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
    Active { recovery: bool },
    First { request_id: String },
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
}
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
    stop: AtomicBool,
    manual: AtomicBool,
    manual_committed: AtomicBool,
    done: Mutex<bool>,
    wake: Condvar,
    drain: Mutex<Option<crate::runtime::app_worker::AppWorkerDrain>>,
}
struct Operation<'a> {
    inner: &'a Inner,
    key: Key,
}
impl AppLifecycleService {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        admission: Arc<Semaphore>,
        publisher: AppWorkerPublisher,
    ) -> Self {
        Self(Arc::new(Inner {
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
        }))
    }
}
