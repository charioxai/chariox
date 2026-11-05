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

/// Wakes the readers that wait for an operation to change; every operation of this kernel signals it.
#[derive(Clone)]
pub(crate) struct OperationChanges(Arc<tokio::sync::watch::Sender<u64>>);

impl OperationChanges {
    pub(crate) fn new() -> Self {
        Self(Arc::new(tokio::sync::watch::channel(0).0))
    }

    pub(crate) fn notify(&self) {
        self.0.send_modify(|count| *count += 1);
    }

    /// Subscribe before reading the view, so a change in between is not missed.
    pub(crate) fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.0.subscribe()
    }
}

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
    changes: OperationChanges,
    live: Mutex<HashMap<String, Arc<Live>>>,
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
    pub(crate) fn new(private_state_root: &Path, changes: OperationChanges) -> Arc<Self> {
        Arc::new(Self {
            root: private_state_root.join(DIRECTORY),
            changes,
            live: Mutex::new(HashMap::new()),
        })
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
        if let Some(live) = lock(&self.live).get(operation_id) {
            return Ok(Some(lock(&live.view).clone()));
        }
        let Some(mut view) = self.load(operation_id)? else {
            return Ok(None);
        };
        if view.state == EnvironmentOperationState::Running {
            view.state = EnvironmentOperationState::Failed;
            view.ended_at_ms = Some(crate::session::unix_epoch_ms());
            view.reason = Some("Kernel restarted".into());
            view.updates += 1;
            self.persist(&view)?;
        }
        Ok(Some(view))
    }

    pub(crate) fn begin(
        self: &Arc<Self>,
        view: EnvironmentOperationView,
    ) -> Result<Begun, DaemonError> {
        if let Some(existing) = self.get(&view.operation_id)? {
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
        lock(&self.live).insert(view.operation_id, live.clone());
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
                view.updates += 1;
                self.persist(&view)?;
            }
            view.clone()
        };
        live.stop.send_replace(true);
        self.changes.notify();
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

    /// Runs `work` to its end and answers its result, or None after Stop. Stop asks the work to end
    /// through `stop` and still waits for it: the operation stays live until its work has ended,
    /// and dropping this handle afterwards is the point where that work is over.
    pub(crate) async fn run_until_stopped<T>(
        &self,
        work: impl std::future::Future<Output = T>,
        stop: &crate::provider::UtilityStop,
    ) -> Option<T> {
        tokio::pin!(work);
        let done = tokio::select! {
            done = &mut work => done,
            _ = self.stopped() => {
                stop.stop();
                work.await
            }
        };
        (!self.is_stopped()).then_some(done)
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
        view.updates += 1;
        let _ = self.operations.persist(&view);
        drop(view);
        self.operations.changes.notify();
    }

    pub(crate) fn finish(&self, state: EnvironmentOperationState, reason: Option<String>) {
        let mut view = lock(&self.live.view);
        if view.state != EnvironmentOperationState::Running {
            return;
        }
        view.state = state;
        view.reason = reason;
        view.ended_at_ms = Some(crate::session::unix_epoch_ms());
        view.updates += 1;
        let _ = self.operations.persist(&view);
        drop(view);
        self.operations.changes.notify();
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
            updates: 0,
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
        let operations = EnvironmentOperations::new(&root, OperationChanges::new());
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
    async fn stop_keeps_the_operation_live_until_its_work_has_ended() {
        let root = scratch();
        let operations = EnvironmentOperations::new(&root, OperationChanges::new());
        let Begun::Started(handle) = operations.begin(view("a")).unwrap() else {
            panic!("new operation");
        };
        let stop = crate::provider::UtilityStop::default();
        let (ended_tx, ended_rx) = std::sync::mpsc::channel();
        // Blocking work like a provider utility: it sees Stop only through `stop`, and ends later.
        let work = {
            let stop = stop.clone();
            tokio::task::spawn_blocking(move || {
                // Bounded, so a handle that never signals Stop fails the test instead of hanging it.
                let give_up = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while !stop.is_stopped() && std::time::Instant::now() < give_up {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
                ended_tx.send(()).unwrap();
            })
        };
        let task = tokio::spawn(async move {
            let answered = handle.run_until_stopped(work, &stop).await;
            assert!(answered.is_none(), "Stop answers no result");
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        operations.cancel("a").unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(ended_rx.try_recv().is_err(), "the work is still running");
        assert!(
            lock(&operations.live).contains_key("a"),
            "the operation was released while its work still ran"
        );
        task.await.unwrap();
        assert!(ended_rx.try_recv().is_ok());
        assert!(!lock(&operations.live).contains_key("a"));
        assert_eq!(
            operations.get("a").unwrap().unwrap().state,
            EnvironmentOperationState::Cancelled
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_repeated_id_answers_the_existing_operation_and_a_restart_fails_a_running_one() {
        let root = scratch();
        let operations = EnvironmentOperations::new(&root, OperationChanges::new());
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
        let restarted = EnvironmentOperations::new(&root, OperationChanges::new());
        let settled = restarted.get("a").unwrap().unwrap();
        assert_eq!(settled.state, EnvironmentOperationState::Failed);
        assert_eq!(settled.reason.as_deref(), Some("Kernel restarted"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn every_change_counts_as_an_update_and_wakes_a_waiting_reader() {
        let root = scratch();
        let changes = OperationChanges::new();
        let operations = EnvironmentOperations::new(&root, changes.clone());
        let Begun::Started(handle) = operations.begin(view("a")).unwrap() else {
            panic!("new operation");
        };
        let mut waiting = changes.subscribe();
        let woken = tokio::spawn(async move { waiting.changed().await.is_ok() });
        handle.update(|view| view.source_label = Some("A".into()));
        assert!(woken.await.unwrap(), "the update woke the reader");
        assert_eq!(operations.get("a").unwrap().unwrap().updates, 1);
        let mut waiting = changes.subscribe();
        let woken = tokio::spawn(async move { waiting.changed().await.is_ok() });
        handle.finish(EnvironmentOperationState::Succeeded, None);
        assert!(woken.await.unwrap(), "the end woke the reader");
        assert_eq!(operations.get("a").unwrap().unwrap().updates, 2);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_dropped_handle_does_not_leave_the_operation_running() {
        let root = scratch();
        let operations = EnvironmentOperations::new(&root, OperationChanges::new());
        let Begun::Started(handle) = operations.begin(view("a")).unwrap() else {
            panic!("new operation");
        };
        drop(handle);
        let view = operations.get("a").unwrap().unwrap();
        assert_eq!(view.state, EnvironmentOperationState::Failed);
        let _ = fs::remove_dir_all(root);
    }
}
