//! Environment operations that are not a script: Import from another kernel and Draft with an
//! agent. A client chooses the operation id, polls the view and may Stop; a kernel restart turns a
//! run that was still going into a failed one with that reason.
use super::environment_error;
use crate::error::DaemonError;
use crate::local::{EnvironmentOperationState, EnvironmentOperationView};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

const DIRECTORY: &str = "project-environment-operations";

struct Live {
    view: Mutex<EnvironmentOperationView>,
    stop: tokio::sync::watch::Sender<bool>,
    publication: Mutex<Option<Result<(), String>>>,
    published: std::sync::Condvar,
    persistence: crate::blocking_io::Serial,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Default)]
struct Registry {
    entries: HashMap<String, Arc<Live>>,
    generation: u64,
}

pub(crate) struct EnvironmentOperations {
    root: PathBuf,
    live: Mutex<Registry>,
    managed: crate::room_terminal::managed_work::ManagedWork,
    #[cfg(test)]
    pub(crate) before_persist: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

/// The running side of one operation: progress goes through it, and it tells whether Stop was pressed.
pub(crate) struct OperationHandle {
    operations: Arc<EnvironmentOperations>,
    live: Arc<Live>,
}

pub(crate) enum Begun {
    Started(OperationHandle),
    /// The client repeated an operation id: this is the operation that already carries it.
    Existing(EnvironmentOperationView),
}

impl EnvironmentOperations {
    pub(crate) fn new(private_state_root: &Path) -> Arc<Self> {
        Arc::new(Self {
            root: private_state_root.join(DIRECTORY),
            live: Mutex::new(Registry::default()),
            managed: Default::default(),
            #[cfg(test)]
            before_persist: Mutex::new(None),
        })
    }

    pub(crate) fn link_managed_kernel(
        &self,
        gate: Arc<crate::runtime::state::ManagedKernelQuiescenceGate>,
        callback: std::sync::Weak<dyn Fn() + Send + Sync>,
    ) {
        self.managed.link(gate, callback);
    }

    pub(crate) fn has_running_work(&self) -> bool {
        // Stop settles the view immediately; keep the work until the task finishes its cleanup.
        !lock(&self.live).entries.is_empty()
    }

    fn report_work(&self) {
        self.managed.report(|| self.has_running_work());
    }

    fn path(&self, operation_id: &str) -> PathBuf {
        self.root.join(format!(
            "op-{:x}.json",
            Sha256::digest(operation_id.as_bytes())
        ))
    }

    fn persist(&self, view: &EnvironmentOperationView) -> Result<(), DaemonError> {
        #[cfg(test)]
        {
            let hook = lock(&self.before_persist).take();
            if let Some(hook) = hook {
                hook();
            }
        }
        fs::create_dir_all(&self.root)
            .map_err(|_| environment_error("the operation directory is unavailable"))?;
        let bytes = serde_json::to_vec(view)
            .map_err(|_| environment_error("the operation could not be encoded"))?;
        crate::config::write_private_file(&self.path(&view.operation_id), &bytes)
            .map_err(|_| environment_error("saving the operation failed"))
    }

    fn load(&self, operation_id: &str) -> Result<Option<EnvironmentOperationView>, DaemonError> {
        match fs::read(self.path(operation_id)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| environment_error("the saved operation is not valid")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(environment_error("the saved operation is unreadable")),
        }
    }

    pub(crate) fn get(
        &self,
        operation_id: &str,
    ) -> Result<Option<EnvironmentOperationView>, DaemonError> {
        loop {
            let registry = lock(&self.live);
            let live = registry.entries.get(operation_id).cloned();
            let generation = registry.generation;
            drop(registry);
            if let Some(live) = live {
                live.wait_for_publication()?;
                return Ok(Some(lock(&live.view).clone()));
            }
            let saved = self.load(operation_id);
            let registry = lock(&self.live);
            if registry.generation != generation {
                continue;
            }
            drop(registry);
            return self.settle_restarted(saved?);
        }
    }

    fn remove(&self, operation_id: &str) {
        let mut registry = lock(&self.live);
        registry.entries.remove(operation_id);
        registry.generation += 1;
    }

    fn settle_restarted(
        &self,
        saved: Option<EnvironmentOperationView>,
    ) -> Result<Option<EnvironmentOperationView>, DaemonError> {
        let Some(mut view) = saved else {
            return Ok(None);
        };
        if view.state == EnvironmentOperationState::Running {
            view.state = EnvironmentOperationState::Failed;
            view.ended_at_ms = Some(crate::session::unix_epoch_ms());
            view.reason = Some("Kernel restarted".into());
            self.persist(&view)?;
        }
        Ok(Some(view))
    }

    pub(crate) fn begin(
        self: &Arc<Self>,
        view: EnvironmentOperationView,
    ) -> Result<Begun, DaemonError> {
        let admission = self.managed.admission()?;
        let mut registry = lock(&self.live);
        if let Some(existing) = registry.entries.get(&view.operation_id).cloned() {
            drop(registry);
            drop(admission);
            existing.wait_for_publication()?;
            let existing = lock(&existing.view).clone();
            return existing_operation(existing, &view.project_id);
        }
        let (stop, _) = tokio::sync::watch::channel(false);
        let live = Arc::new(Live {
            view: Mutex::new(view.clone()),
            stop,
            publication: Mutex::new(None),
            published: std::sync::Condvar::new(),
            persistence: Default::default(),
        });
        // Count the sole provisional owner before releasing admission. Stop rechecks
        // this registry even though the intent and its activity are not durable yet.
        registry
            .entries
            .insert(view.operation_id.clone(), live.clone());
        registry.generation += 1;
        drop(registry);
        drop(admission);
        // Another mutation can sample this provisional owner. Report both edges
        // even if intent persistence fails before an operation handle is returned.
        self.report_work();
        let publication = self
            .load(&view.operation_id)
            .and_then(|saved| self.settle_restarted(saved))
            .and_then(|saved| match saved {
                Some(saved) => Ok(Some(saved)),
                None => self.persist(&view).map(|()| None),
            });
        match publication {
            Ok(None) => {
                live.publish(Ok(()));
                self.report_work();
                Ok(Begun::Started(OperationHandle {
                    operations: self.clone(),
                    live,
                }))
            }
            Ok(Some(saved)) => {
                *lock(&live.view) = saved.clone();
                live.publish(Ok(()));
                self.remove(&view.operation_id);
                self.report_work();
                existing_operation(saved, &view.project_id)
            }
            Err(error) => {
                live.publish(Err(error.to_string()));
                self.remove(&view.operation_id);
                self.report_work();
                Err(error)
            }
        }
    }

    /// Stop: the operation reads cancelled at once; what it already landed stays.
    pub(crate) fn cancel(
        &self,
        operation_id: &str,
    ) -> Result<Option<EnvironmentOperationView>, DaemonError> {
        let Some(live) = lock(&self.live).entries.get(operation_id).cloned() else {
            return self.get(operation_id);
        };
        live.wait_for_publication()?;
        let _persistence = live.persistence.acquire();
        let mut view = lock(&live.view).clone();
        if view.state == EnvironmentOperationState::Running {
            view.state = EnvironmentOperationState::Cancelled;
            view.ended_at_ms = Some(crate::session::unix_epoch_ms());
            self.persist(&view)?;
            *lock(&live.view) = view.clone();
        }
        live.stop.send_replace(true);
        Ok(Some(view))
    }
}

fn existing_operation(
    view: EnvironmentOperationView,
    project_id: &str,
) -> Result<Begun, DaemonError> {
    if view.project_id != project_id {
        return Err(environment_error(
            "this operation id belongs to another Project",
        ));
    }
    Ok(Begun::Existing(view))
}

impl Live {
    fn publish(&self, result: Result<(), String>) {
        *lock(&self.publication) = Some(result);
        self.published.notify_all();
    }

    fn wait_for_publication(&self) -> Result<(), DaemonError> {
        let mut publication = lock(&self.publication);
        while publication.is_none() {
            publication = self
                .published
                .wait(publication)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        publication
            .as_ref()
            .expect("publication completed")
            .clone()
            .map_err(environment_error)
    }
}

impl OperationHandle {
    pub(crate) fn snapshot(&self) -> Option<EnvironmentOperationView> {
        Some(lock(&self.live.view).clone())
    }

    pub(crate) fn is_stopped(&self) -> bool {
        *self.live.stop.borrow()
    }

    /// Completes when Stop is pressed, so a long wait can be abandoned.
    pub(crate) async fn stopped(&self) {
        let mut stop = self.live.stop.subscribe();
        let _ = stop.wait_for(|stopped| *stopped).await;
    }

    /// Changes the view and saves it. After Stop the view is final and later changes are dropped.
    pub(crate) fn update(&self, change: impl FnOnce(&mut EnvironmentOperationView)) {
        let _persistence = self.live.persistence.acquire();
        let mut view = lock(&self.live.view).clone();
        if view.state != EnvironmentOperationState::Running {
            return;
        }
        change(&mut view);
        let _ = self.operations.persist(&view);
        *lock(&self.live.view) = view;
    }

    pub(crate) fn finish(&self, state: EnvironmentOperationState, reason: Option<String>) {
        let _persistence = self.live.persistence.acquire();
        let mut view = lock(&self.live.view).clone();
        if view.state != EnvironmentOperationState::Running {
            return;
        }
        view.state = state;
        view.reason = reason;
        view.ended_at_ms = Some(crate::session::unix_epoch_ms());
        let _ = self.operations.persist(&view);
        *lock(&self.live.view) = view;
    }
}

impl Drop for OperationHandle {
    fn drop(&mut self) {
        // A task that ends without a result must not leave its operation running for ever.
        self.finish(
            EnvironmentOperationState::Failed,
            Some("The operation ended without a result".into()),
        );
        let id = lock(&self.live.view).operation_id.clone();
        self.operations.remove(&id);
        self.operations.report_work();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::EnvironmentOperationKind;

    fn view(id: &str) -> EnvironmentOperationView {
        EnvironmentOperationView {
            operation_id: id.into(),
            project_id: "p".into(),
            checkout_path: "/work/p".into(),
            kind: EnvironmentOperationKind::Import,
            state: EnvironmentOperationState::Running,
            started_at_ms: 1,
            ended_at_ms: None,
            exit_code: None,
            reason: None,
            output_tail: String::new(),
            entries: Vec::new(),
            source_label: None,
        }
    }

    fn scratch() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chariox-env-ops-{:032x}", rand::random::<u128>()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn stop_ends_the_view_at_once_and_later_changes_are_dropped() {
        let root = scratch();
        let operations = EnvironmentOperations::new(&root);
        let Begun::Started(handle) = operations.begin(view("a")).unwrap() else {
            panic!("new operation");
        };
        handle.update(|view| view.source_label = Some("A".into()));
        assert!(!handle.is_stopped());
        let stopped = operations.cancel("a").unwrap().unwrap();
        assert_eq!(stopped.state, EnvironmentOperationState::Cancelled);
        assert!(handle.is_stopped());
        handle.stopped().await;
        handle.update(|view| view.source_label = Some("B".into()));
        handle.finish(EnvironmentOperationState::Succeeded, None);
        let after = operations.get("a").unwrap().unwrap();
        assert_eq!(after.state, EnvironmentOperationState::Cancelled);
        assert_eq!(after.source_label.as_deref(), Some("A"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_repeated_id_answers_the_existing_operation_and_a_restart_fails_a_running_one() {
        let root = scratch();
        let operations = EnvironmentOperations::new(&root);
        let Begun::Started(handle) = operations.begin(view("a")).unwrap() else {
            panic!("new operation");
        };
        assert!(matches!(
            operations.begin(view("a")).unwrap(),
            Begun::Existing(_)
        ));
        let mut other_project = view("a");
        other_project.project_id = "q".into();
        assert!(operations.begin(other_project).is_err());
        // A second kernel process over the same state sees the run as cut off.
        std::mem::forget(handle);
        let restarted = EnvironmentOperations::new(&root);
        let settled = restarted.get("a").unwrap().unwrap();
        assert_eq!(settled.state, EnvironmentOperationState::Failed);
        assert_eq!(settled.reason.as_deref(), Some("Kernel restarted"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_dropped_handle_does_not_leave_the_operation_running() {
        let root = scratch();
        let operations = EnvironmentOperations::new(&root);
        let Begun::Started(handle) = operations.begin(view("a")).unwrap() else {
            panic!("new operation");
        };
        drop(handle);
        let view = operations.get("a").unwrap().unwrap();
        assert_eq!(view.state, EnvironmentOperationState::Failed);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_requests_with_one_id_start_one_task_that_stop_still_reaches() {
        for round in 0..40 {
            let root = scratch();
            let operations = EnvironmentOperations::new(&root);
            let gate = Arc::new(std::sync::Barrier::new(8));
            let id = format!("same-{round}");
            let outcomes: Vec<_> = (0..8)
                .map(|_| {
                    let (operations, gate, id) = (operations.clone(), gate.clone(), id.clone());
                    std::thread::spawn(move || {
                        gate.wait();
                        operations.begin(view(&id)).unwrap()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect();
            let mut started = outcomes
                .into_iter()
                .filter_map(|begun| match begun {
                    Begun::Started(handle) => Some(handle),
                    Begun::Existing(_) => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(started.len(), 1, "round {round}");
            let handle = started.pop().unwrap();
            operations.cancel(&id).unwrap();
            assert!(handle.is_stopped(), "Stop reaches the one running task");
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn polling_while_an_operation_is_published_never_settles_it_as_restarted() {
        for round in 0..40 {
            let root = scratch();
            let operations = EnvironmentOperations::new(&root);
            let id = format!("poll-{round}");
            let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let poller = {
                let (operations, id, done) = (operations.clone(), id.clone(), done.clone());
                std::thread::spawn(move || {
                    let mut seen = Vec::new();
                    while !done.load(std::sync::atomic::Ordering::SeqCst) {
                        if let Some(view) = operations.get(&id).unwrap() {
                            seen.push(view.state);
                        }
                    }
                    seen
                })
            };
            let Begun::Started(handle) = operations.begin(view(&id)).unwrap() else {
                panic!("new operation");
            };
            done.store(true, std::sync::atomic::Ordering::SeqCst);
            let seen = poller.join().unwrap();
            assert!(
                seen.iter()
                    .all(|state| *state == EnvironmentOperationState::Running),
                "round {round}: a poll saw {seen:?}"
            );
            drop(handle);
            let _ = fs::remove_dir_all(root);
        }
    }
    #[test]
    fn intent_io_releases_admission_while_provisional_work_blocks_stop() {
        use crate::runtime::state::{ManagedActivityObservation, ManagedKernelQuiescenceGate};
        use std::sync::mpsc;
        use std::time::Duration;
        let root = scratch();
        let store =
            crate::durable_state::DurableKernelStateStore::open(root.join("activity.sqlite"))
                .unwrap();
        let gate =
            ManagedKernelQuiescenceGate::restore(store, "kernel".into(), Arc::new(Mutex::new(())))
                .unwrap();
        let operations = EnvironmentOperations::new(&root);
        let callback: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        operations.link_managed_kernel(gate.clone(), Arc::downgrade(&callback));
        gate.confirm_activity_report(
            1,
            1,
            ManagedActivityObservation {
                working_agent_count: 0,
                changed_at_ms: 1,
            },
        );
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        *lock(&operations.before_persist) = Some(Arc::new(move || {
            entered_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }));
        let starting = {
            let operations = operations.clone();
            std::thread::spawn(move || operations.begin(view("provisional")))
        };
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (admitted_tx, admitted_rx) = mpsc::channel();
        let probing = {
            let gate = gate.clone();
            std::thread::spawn(move || {
                let _admission = gate.admission_guard().unwrap();
                admitted_tx.send(()).unwrap();
            })
        };
        let acquired = admitted_rx.recv_timeout(Duration::from_millis(250)).is_ok();
        let (counted_tx, counted_rx) = mpsc::channel();
        let counting = {
            let operations = operations.clone();
            std::thread::spawn(move || counted_tx.send(operations.has_running_work()).unwrap())
        };
        let counted = counted_rx.recv_timeout(Duration::from_millis(250)).ok();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let stopping = {
            let gate = gate.clone();
            let operations = operations.clone();
            std::thread::spawn(move || {
                stopped_tx
                    .send(
                        gate.reserve_if_current(
                            crate::runtime::state::ManagedKernelQuiescenceChallenge {
                                challenge_id: "intent-stop".into(),
                                account_id: "account".into(),
                                environment_id: "environment".into(),
                                machine_id: "machine".into(),
                                kernel_id: "kernel".into(),
                                desired_revision: 1,
                                idle_sequence: 1,
                                idle_deadline_at: "2026-11-05T00:00:00.000Z".into(),
                                stop_operation_id: "stop".into(),
                                nonce: "nonce".into(),
                            },
                            || {
                                Ok((
                                    1,
                                    ManagedActivityObservation {
                                        working_agent_count: u8::from(
                                            operations.has_running_work(),
                                        ),
                                        changed_at_ms: 1,
                                    },
                                ))
                            },
                        )
                        .unwrap(),
                    )
                    .unwrap()
            })
        };
        let rejected = stopped_rx.recv_timeout(Duration::from_millis(250)).ok() == Some(false);
        assert!(
            EnvironmentOperations::new(&root)
                .get("provisional")
                .unwrap()
                .is_none(),
            "a crash before the intent commit has no runnable operation to replay"
        );
        release_tx.send(()).unwrap();
        let begun = starting.join().unwrap().unwrap();
        probing.join().unwrap();
        counting.join().unwrap();
        stopping.join().unwrap();
        assert!(
            rejected,
            "Stop must reject the provisional owner without waiting for disk"
        );
        assert!(
            acquired,
            "MP-08/09/10/11: intent disk IO must not hold admission"
        );
        assert_eq!(
            counted,
            Some(true),
            "provisional intent must remain authoritative busy work"
        );
        let Begun::Started(handle) = begun else {
            panic!("new operation")
        };
        assert_eq!(
            ManagedActivityObservation {
                working_agent_count: u8::from(operations.has_running_work()),
                changed_at_ms: 1
            }
            .working_agent_count,
            1
        );
        drop(handle);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_intent_publication_retracts_the_owner_and_allows_retry() {
        let root = scratch();
        let operations = EnvironmentOperations::new(&root);
        let blocked_directory = operations.root.clone();
        *lock(&operations.before_persist) = Some(Arc::new(move || {
            fs::write(&blocked_directory, b"injected disk failure").unwrap();
        }));
        assert!(operations.begin(view("failed")).is_err());
        assert!(
            !operations.has_running_work(),
            "failed persistence left an orphan owner"
        );
        fs::remove_file(&operations.root).unwrap();
        assert!(operations.get("failed").unwrap().is_none());
        let Begun::Started(handle) = operations.begin(view("failed")).unwrap() else {
            panic!("rollback must permit a new owner")
        };
        drop(handle);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_ids_wait_for_one_durable_intent_owner() {
        use std::sync::mpsc;
        use std::time::Duration;
        let root = scratch();
        let operations = EnvironmentOperations::new(&root);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        *lock(&operations.before_persist) = Some(Arc::new(move || {
            entered_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }));
        let first = {
            let operations = operations.clone();
            std::thread::spawn(move || operations.begin(view("duplicate")).unwrap())
        };
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (duplicate_tx, duplicate_rx) = mpsc::channel();
        let second = {
            let operations = operations.clone();
            std::thread::spawn(move || {
                duplicate_tx
                    .send(operations.begin(view("duplicate")))
                    .unwrap()
            })
        };
        let exposed = duplicate_rx
            .recv_timeout(Duration::from_millis(100))
            .is_ok();
        release_tx.send(()).unwrap();
        let first = first.join().unwrap();
        let second_result = duplicate_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        second.join().unwrap();
        assert!(!exposed, "a duplicate exposed an uncommitted operation");
        assert!(matches!(second_result, Begun::Existing(_)));
        let Begun::Started(handle) = first else {
            panic!("one owner")
        };
        assert_eq!(fs::read_dir(&operations.root).unwrap().count(), 1);
        operations.cancel("duplicate").unwrap();
        assert!(handle.is_stopped());
        drop(handle);
        fs::remove_dir_all(root).unwrap();
    }
}
