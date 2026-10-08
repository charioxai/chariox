use std::collections::{btree_map::Entry, BTreeMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

type ExecutionKey = (String, String);

#[derive(Default)]
struct ComputerInputExecutionState {
    cancellation_requested: AtomicBool,
    process_group_terminated: AtomicBool,
    process_group: Mutex<Option<u32>>,
    // Retained by the blocking helper's cancellation handle even if its
    // async caller is dropped. Capture resumes only after physical input settles.
    capture_exclusion: Mutex<Option<tokio::sync::OwnedRwLockWriteGuard<()>>>,
}

#[derive(Clone)]
pub(crate) struct ComputerInputCancellation {
    state: Arc<ComputerInputExecutionState>,
    authorizer: Option<Arc<dyn Fn() -> Result<(), crate::error::DaemonError> + Send + Sync>>,
    agent_input: bool,
}

impl ComputerInputCancellation {
    pub(crate) fn with_agent_input(mut self, agent_input: bool) -> Self {
        self.agent_input = agent_input;
        self
    }

    pub(crate) fn is_agent_input(&self) -> bool {
        self.agent_input
    }

    pub(crate) fn with_authorizer(
        mut self,
        authorizer: Arc<dyn Fn() -> Result<(), crate::error::DaemonError> + Send + Sync>,
    ) -> Self {
        self.authorizer = Some(authorizer);
        self
    }

    pub(crate) fn authorize(&self) -> Result<(), crate::error::DaemonError> {
        if let Some(authorizer) = &self.authorizer {
            authorizer()?;
        }
        Ok(())
    }

    pub(crate) fn requested(&self) -> bool {
        self.state.cancellation_requested.load(Ordering::Acquire)
    }

    pub(crate) fn process_group_was_terminated(&self) -> bool {
        self.state.process_group_terminated.load(Ordering::Acquire)
    }

    pub(crate) fn register_process_group(&self, process_group: u32) {
        let mut registered = self
            .state
            .process_group
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *registered = Some(process_group);
        if self.requested() {
            kill_process_group(process_group);
        }
    }

    pub(crate) fn terminate_process_group(&self) {
        if let Some(process_group) = *self
            .state
            .process_group
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            self.state
                .process_group_terminated
                .store(true, Ordering::Release);
            kill_process_group(process_group);
        }
    }

    pub(crate) fn clear_process_group(&self, process_group: u32) {
        let mut registered = self
            .state
            .process_group
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if registered.as_ref() == Some(&process_group) {
            *registered = None;
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ComputerInputExecutionStore {
    active: Arc<Mutex<BTreeMap<ExecutionKey, Arc<ComputerInputExecutionState>>>>,
    display_capture: Arc<tokio::sync::RwLock<()>>,
}

pub(crate) struct ComputerInputExecution {
    store: ComputerInputExecutionStore,
    key: ExecutionKey,
    state: Arc<ComputerInputExecutionState>,
}

impl ComputerInputExecutionStore {
    pub(crate) fn capture_guard(
        &self,
    ) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, &'static str> {
        self.display_capture
            .clone()
            .try_read_owned()
            .map_err(|_| "agent screen capture withheld while computer credential input is running")
    }

    pub(crate) fn begin(
        &self,
        session_id: &str,
        action_id: &str,
    ) -> Result<ComputerInputExecution, &'static str> {
        let key = (session_id.to_string(), action_id.to_string());
        let state = Arc::new(ComputerInputExecutionState::default());
        let mut active = self
            .active
            .lock()
            .map_err(|_| "computer input execution registry poisoned")?;
        match active.entry(key.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(Arc::clone(&state));
            }
            Entry::Occupied(_) => {
                return Err("computer input action is already executing");
            }
        }
        Ok(ComputerInputExecution {
            store: self.clone(),
            key,
            state,
        })
    }

    pub(crate) fn cancel(&self, session_id: &str, action_id: &str) -> bool {
        let state = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&(session_id.to_string(), action_id.to_string()))
            .cloned();
        let Some(state) = state else {
            return false;
        };
        state.cancellation_requested.store(true, Ordering::Release);
        if let Some(process_group) = *state
            .process_group
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            kill_process_group(process_group);
        }
        true
    }
}

impl ComputerInputExecution {
    pub(crate) async fn withhold_capture(&self) {
        let guard = self.store.display_capture.clone().write_owned().await;
        *self
            .state
            .capture_exclusion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(guard);
    }

    pub(crate) fn cancellation(&self) -> ComputerInputCancellation {
        ComputerInputCancellation {
            state: Arc::clone(&self.state),
            authorizer: None,
            agent_input: false,
        }
    }
}

impl Drop for ComputerInputExecution {
    fn drop(&mut self) {
        let mut active = self
            .store
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active
            .get(&self.key)
            .is_some_and(|state| Arc::ptr_eq(state, &self.state))
        {
            active.remove(&self.key);
        }
    }
}

#[cfg(unix)]
fn kill_process_group(process_group: u32) {
    let Some(target) = owned_process_group_signal_target(process_group) else {
        return;
    };
    // Only child groups created by the registered helper's Command::process_group(0)
    // reach this seam. Never interpret an invalid child ID as a system-wide target.
    let _ = unsafe { libc::kill(target, libc::SIGKILL) };
}

#[cfg(unix)]
fn owned_process_group_signal_target(process_group: u32) -> Option<i32> {
    let group = i32::try_from(process_group).ok()?;
    (group > 1).then_some(-group)
}

#[cfg(all(test, unix))]
#[test]
fn mp08_mp10_mp11_computer_signal_rejects_system_and_overflow_ids() {
    for group in [0, 1, u32::MAX, i32::MAX as u32 + 1] {
        assert_eq!(owned_process_group_signal_target(group), None);
    }
    assert_eq!(owned_process_group_signal_target(12345), Some(-12345));
}

#[cfg(not(unix))]
fn kill_process_group(_process_group: u32) {}

#[cfg(test)]
mod secret_capture_tests {
    use super::*;

    #[tokio::test]
    async fn capture_stays_withheld_until_a_dropped_callers_helper_settles() {
        let store = ComputerInputExecutionStore::default();
        let execution = store.begin("room", "secret").unwrap();
        execution.withhold_capture().await;
        let helper = execution.cancellation();
        drop(execution);
        assert!(
            store.capture_guard().is_err(),
            "dropping caller must not reopen capture while its helper lives"
        );
        drop(helper);
        assert!(
            store.capture_guard().is_ok(),
            "capture resumes after physical input settles"
        );
    }

    #[tokio::test]
    async fn insertion_waits_for_prior_frame_capture_and_withholds_new_frames() {
        let store = ComputerInputExecutionStore::default();
        let frame = store.capture_guard().unwrap();
        let execution = store.begin("room", "secret").unwrap();
        let mut wait = Box::pin(execution.withhold_capture());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut wait)
                .await
                .is_err()
        );
        assert!(
            store.capture_guard().is_err(),
            "queued insertion must withhold new frame reads"
        );
        drop(frame);
        wait.await;
        assert!(store.capture_guard().is_err());
        drop(execution);
        assert!(store.capture_guard().is_ok());
    }
}
