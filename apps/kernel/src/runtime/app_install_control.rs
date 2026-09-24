//! Retained installation work, advanced by the existing kernel pump. The task
//! set owns every slow operation; no task retains KernelRuntimeState/AppControl.
mod jobs;
mod pump;
mod requests;
mod selection;
#[cfg(test)]
mod tests;

use crate::{
    durable_state::{
        app_installation_operations::{
            InstallApprovalChallenge, InstallOperation, InstallOperationError, InstallPhase,
            InstallReviewDisposition,
        },
        DurableKernelStateStore,
    },
    runtime::{
        app_lifecycle::AppLifecycleService,
        app_operation_budget::AppOperationBudget,
        app_package_preparation::AppPackagePreparation,
        state::{KernelRuntimeState, PendingInteractionResolution},
    },
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{oneshot, Semaphore},
    task::{Id, JoinSet},
};

const OWNERS: usize = 32;
const JOBS: usize = 4;
const RETRY: Duration = Duration::from_millis(500);
type Key = (String, String);
#[derive(Clone)]
pub(crate) struct AppInstallControl(Arc<Inner>);
struct Inner {
    shared: Arc<Shared>,
    state: Mutex<State>,
    pumping: AtomicBool,
    shutdown: Mutex<()>,
    stopped: Arc<AtomicBool>,
    #[cfg(test)]
    request_checkpoint: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}
struct Shared {
    store: DurableKernelStateStore,
    preparation: AppPackagePreparation,
    admission: Arc<Semaphore>,
    lifecycle: AppLifecycleService,
}
struct State {
    entries: BTreeMap<Key, Entry>,
    tasks: JoinSet<(Key, jobs::Result)>,
    requests: JoinSet<()>,
    task_keys: BTreeMap<Id, Key>,
    job_cursor: Option<Key>,
    scan_next: Instant,
    scan_cursor: Option<Key>,
    scanning: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            tasks: JoinSet::new(),
            requests: JoinSet::new(),
            task_keys: BTreeMap::new(),
            job_cursor: None,
            scan_next: Instant::now(),
            scan_cursor: None,
            scanning: false,
        }
    }
}
struct Entry {
    cancelled: Arc<AtomicBool>,
    step: Step,
    busy: bool,
    next: Instant,
}
enum Step {
    Work,
    Waiting {
        session: String,
        challenge: Arc<InstallApprovalChallenge>,
        receiver: oneshot::Receiver<PendingInteractionResolution>,
        deadline: Instant,
    },
    Decide {
        challenge: Arc<InstallApprovalChallenge>,
        accepted: bool,
        deadline: Instant,
    },
    Start,
    Stop,
    Fail(&'static str),
    Done,
}
impl Entry {
    fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            step: Step::Work,
            busy: false,
            next: Instant::now(),
        }
    }
}
impl AppInstallControl {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        preparation: AppPackagePreparation,
        admission: Arc<Semaphore>,
        lifecycle: AppLifecycleService,
    ) -> Self {
        Self(Arc::new(Inner {
            shared: Arc::new(Shared {
                store,
                preparation,
                admission,
                lifecycle,
            }),
            state: Mutex::new(State::default()),
            pumping: AtomicBool::new(false),
            shutdown: Mutex::new(()),
            stopped: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            request_checkpoint: Mutex::new(None),
        }))
    }
    fn notify(&self, key: Key) {
        if self.0.stopped.load(Ordering::Acquire) {
            return;
        }
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.0.stopped.load(Ordering::Acquire) {
            return;
        }
        if state.entries.len() < OWNERS {
            state.entries.entry(key).or_insert_with(Entry::new);
        }
    }
    fn cancel_admission(&self, key: &Key) {
        let state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = state.entries.get(key) {
            entry.cancelled.store(true, Ordering::Release);
        }
    }
    fn cancelled(&self, key: Key, has_stage: bool) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.0.stopped.load(Ordering::Acquire) {
            return;
        }
        if state.entries.len() >= OWNERS && !state.entries.contains_key(&key) {
            return;
        }
        let entry = state.entries.entry(key).or_insert_with(Entry::new);
        entry.cancelled.store(true, Ordering::Release);
        entry.step = if has_stage { Step::Stop } else { Step::Done };
        entry.next = Instant::now();
    }
    /// Called on retained blocking shutdown ownership before runtime teardown.
    /// Cancellation of its caller cannot detach the verification/writer tasks.
    pub(crate) fn shutdown_blocking(&self, runtime: tokio::runtime::Handle) {
        self.0.stopped.store(true, Ordering::Release);
        let _shutdown = self
            .0
            .shutdown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut state = std::mem::take(
            &mut *self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for entry in state.entries.values() {
            entry.cancelled.store(true, Ordering::Release);
        }
        // Dropping receivers invokes the generic interaction abandonment path.
        state.entries.clear();
        runtime.block_on(async {
            while state.requests.join_next().await.is_some() {}
            while state.tasks.join_next().await.is_some() {}
        });
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for entry in state.entries.values() {
            entry.cancelled.store(true, Ordering::Release);
        }
        // Production drains explicitly. Any last-resort abort cannot cancel an
        // already started blocking verifier; it retains its own permit/leases.
        state.tasks.abort_all();
        state.requests.abort_all();
    }
}
