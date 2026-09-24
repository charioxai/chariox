//! Retained human enrollment decisions. Only the kernel pump holds runtime
//! session access; owned jobs retain stores, cancellation and bounded permits.
mod jobs;
mod pump;
mod requests;
mod terminal;
#[cfg(test)]
mod tests;

use crate::{
    durable_state::{
        app_publisher_operations::{
            PublisherApprovalChallenge, PublisherEnrollmentInput, PublisherOperation,
            PublisherOperationError, PublisherOperationPhase, PublisherReview,
        },
        DurableKernelStateStore,
    },
    runtime::{
        app_operation_budget::AppOperationBudget,
        state::{KernelRuntimeState, PendingInteractionResolution},
    },
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    sync::{oneshot, Semaphore},
    task::{Id, JoinSet},
    time::Instant,
};

const OWNERS: usize = 32;
const JOBS: usize = 4;
const RETRY: Duration = Duration::from_millis(500);
type Key = (String, String);
#[derive(Clone)]
pub(crate) struct AppPublisherControl(Arc<Inner>);
struct Shared {
    store: DurableKernelStateStore,
    admission: Arc<Semaphore>,
}
struct Inner {
    shared: Arc<Shared>,
    state: Mutex<State>,
    pumping: AtomicBool,
    stopped: Arc<AtomicBool>,
    shutdown: Mutex<()>,
    #[cfg(test)]
    request_checkpoint: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}
struct State {
    entries: BTreeMap<Key, Entry>,
    jobs: JoinSet<(Key, jobs::Result)>,
    job_keys: BTreeMap<Id, Key>,
    requests: JoinSet<()>,
    cursor: Option<Key>,
    scan_cursor: Option<Key>,
    scan_next: Instant,
    scanning: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            jobs: JoinSet::new(),
            job_keys: BTreeMap::new(),
            requests: JoinSet::new(),
            cursor: None,
            scan_cursor: None,
            scan_next: Instant::now(),
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
    Arm,
    Waiting {
        challenge: Arc<PublisherApprovalChallenge>,
        receiver: oneshot::Receiver<PendingInteractionResolution>,
    },
    Decide {
        challenge: Arc<PublisherApprovalChallenge>,
        accepted: bool,
    },
    Cancel,
    Done,
}
impl Entry {
    fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            step: Step::Arm,
            busy: false,
            next: Instant::now(),
        }
    }
}
impl AppPublisherControl {
    pub(crate) fn new(store: DurableKernelStateStore, admission: Arc<Semaphore>) -> Self {
        Self(Arc::new(Inner {
            shared: Arc::new(Shared { store, admission }),
            state: Mutex::new(State::default()),
            pumping: AtomicBool::new(false),
            stopped: Arc::new(AtomicBool::new(false)),
            shutdown: Mutex::new(()),
            #[cfg(test)]
            request_checkpoint: Mutex::new(None),
        }))
    }
    fn notify(&self, key: Key, cancelled: bool) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.0.stopped.load(Ordering::Acquire)
            || (state.entries.len() >= OWNERS && !state.entries.contains_key(&key))
        {
            return;
        }
        let entry = state.entries.entry(key).or_insert_with(Entry::new);
        if cancelled {
            entry.cancelled.store(true, Ordering::Release);
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
        // Closed human-wait receivers enter generic interaction abandonment.
        state.entries.clear();
        runtime.block_on(async {
            while state.jobs.join_next().await.is_some() {}
            while state.requests.join_next().await.is_some() {}
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
        // The normal owner joins both sets. Last-resort abort cannot interrupt
        // a blocking writer closure, which still owns its store and permit.
        state.jobs.abort_all();
        state.requests.abort_all();
    }
}
