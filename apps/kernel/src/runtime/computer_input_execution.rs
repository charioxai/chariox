use std::collections::{btree_map::Entry, BTreeMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

type ExecutionKey = (String, String);

#[derive(Default)]
struct ComputerInputExecutionState {
    cancellation_requested: AtomicBool,
    process_group: Mutex<Option<u32>>,
}

#[derive(Clone)]
pub(crate) struct ComputerInputCancellation {
    state: Arc<ComputerInputExecutionState>,
}

impl ComputerInputCancellation {
    pub(crate) fn requested(&self) -> bool {
        self.state.cancellation_requested.load(Ordering::Acquire)
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
}

pub(crate) struct ComputerInputExecution {
    store: ComputerInputExecutionStore,
    key: ExecutionKey,
    state: Arc<ComputerInputExecutionState>,
}

impl ComputerInputExecutionStore {
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
    pub(crate) fn cancellation(&self) -> ComputerInputCancellation {
        ComputerInputCancellation {
            state: Arc::clone(&self.state),
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
    let Ok(process_group) = i32::try_from(process_group) else {
        return;
    };
    let _ = unsafe { libc::kill(-process_group, libc::SIGKILL) };
}

#[cfg(not(unix))]
fn kill_process_group(_process_group: u32) {}
