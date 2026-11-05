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
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) struct EnvironmentOperations {
    root: PathBuf,
    live: Mutex<HashMap<String, Arc<Live>>>,
    managed: crate::room_terminal::managed_work::ManagedWork,
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
            live: Mutex::new(HashMap::new()),
            managed: Default::default(),
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
        !lock(&self.live).is_empty()
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
        fs::create_dir_all(&self.root)
            .map_err(|_| environment_error("the operation directory is unavailable"))?;
        let bytes = serde_json::to_vec(view)
            .map_err(|_| environment_error("the operation could not be encoded"))?;
        let temporary = self.root.join(format!(".tmp-{}", rand::random::<u64>()));
        let result = fs::write(&temporary, bytes)
            .and_then(|_| fs::rename(&temporary, self.path(&view.operation_id)));
        let _ = fs::remove_file(&temporary);
        result.map_err(|_| environment_error("saving the operation failed"))
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
        self.get_locked(&lock(&self.live), operation_id)
    }

    /// The `live` lock is the admission lock: a record that is saved but not yet registered is
    /// only visible while it is held, so reading it as orphaned from a run needs it too.
    fn get_locked(
        &self,
        live: &HashMap<String, Arc<Live>>,
        operation_id: &str,
    ) -> Result<Option<EnvironmentOperationView>, DaemonError> {
        if let Some(live) = live.get(operation_id) {
            return Ok(Some(lock(&live.view).clone()));
        }
        let Some(mut view) = self.load(operation_id)? else {
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
        let _admission = self.managed.admission()?;
        let mut registry = lock(&self.live);
        if let Some(existing) = self.get_locked(&registry, &view.operation_id)? {
            if existing.project_id != view.project_id {
                return Err(environment_error(
                    "this operation id belongs to another Project",
                ));
            }
            return Ok(Begun::Existing(existing));
        }
        self.persist(&view)?;
        let (stop, _) = tokio::sync::watch::channel(false);
        let live = Arc::new(Live {
            view: Mutex::new(view.clone()),
            stop,
        });
        registry.insert(view.operation_id, live.clone());
        drop(registry);
        self.report_work();
        Ok(Begun::Started(OperationHandle {
            operations: self.clone(),
            live,
        }))
    }

    /// Stop: the operation reads cancelled at once; what it already landed stays.
    pub(crate) fn cancel(
        &self,
        operation_id: &str,
    ) -> Result<Option<EnvironmentOperationView>, DaemonError> {
        let Some(live) = lock(&self.live).get(operation_id).cloned() else {
            return self.get(operation_id);
        };
        let view = {
            let mut view = lock(&live.view);
            if view.state == EnvironmentOperationState::Running {
                view.state = EnvironmentOperationState::Cancelled;
                view.ended_at_ms = Some(crate::session::unix_epoch_ms());
                self.persist(&view)?;
            }
            view.clone()
        };
        live.stop.send_replace(true);
        Ok(Some(view))
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
        let mut view = lock(&self.live.view);
        if view.state != EnvironmentOperationState::Running {
            return;
        }
        change(&mut view);
        let _ = self.operations.persist(&view);
    }

    pub(crate) fn finish(&self, state: EnvironmentOperationState, reason: Option<String>) {
        let mut view = lock(&self.live.view);
        if view.state != EnvironmentOperationState::Running {
            return;
        }
        view.state = state;
        view.reason = reason;
        view.ended_at_ms = Some(crate::session::unix_epoch_ms());
        let _ = self.operations.persist(&view);
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
        lock(&self.operations.live).remove(&id);
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
}
