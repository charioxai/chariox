use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use arroba_relay::protocol::RelayKernelPresence;

use crate::local::RemoteMachineRecord;
use crate::session::unix_epoch_ms;

#[derive(Clone, Default)]
pub(crate) struct RemoteRelayInventoryProjectionStore {
    state: Arc<StdMutex<RemoteRelayInventoryProjectionState>>,
}

#[derive(Debug, Clone, Default)]
struct RemoteRelayInventoryProjectionState {
    remote_machines: Vec<RemoteMachineRecord>,
    remote_kernels: Vec<RelayKernelPresence>,
    refreshed_at_ms: u64,
    refresh_requested_at_ms: u64,
}

impl RemoteRelayInventoryProjectionStore {
    pub(crate) fn snapshot(&self) -> (Vec<RemoteMachineRecord>, Vec<RelayKernelPresence>) {
        let state = self
            .state
            .lock()
            .expect("remote relay inventory projection lock should not be poisoned");
        (state.remote_machines.clone(), state.remote_kernels.clone())
    }

    pub(crate) fn should_request_refresh(
        &self,
        now_ms: u64,
        stale_after_ms: u64,
        cooldown_ms: u64,
    ) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("remote relay inventory projection lock should not be poisoned");
        let empty = state.remote_machines.is_empty() && state.remote_kernels.is_empty();
        let stale = state.refreshed_at_ms == 0
            || now_ms.saturating_sub(state.refreshed_at_ms) >= stale_after_ms;
        let cooled_down = state.refresh_requested_at_ms == 0
            || now_ms.saturating_sub(state.refresh_requested_at_ms) >= cooldown_ms;
        if (empty || stale) && cooled_down {
            state.refresh_requested_at_ms = now_ms;
            return true;
        }
        false
    }

    pub(crate) fn update(
        &self,
        remote_machines: Vec<RemoteMachineRecord>,
        remote_kernels: Vec<RelayKernelPresence>,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("remote relay inventory projection lock should not be poisoned");
        state.remote_machines = remote_machines;
        state.remote_kernels = remote_kernels;
        state.refreshed_at_ms = unix_epoch_ms();
        state.refresh_requested_at_ms = state.refreshed_at_ms;
    }

    pub(crate) fn clear(&self) {
        let mut state = self
            .state
            .lock()
            .expect("remote relay inventory projection lock should not be poisoned");
        state.remote_machines.clear();
        state.remote_kernels.clear();
        state.refreshed_at_ms = 0;
        state.refresh_requested_at_ms = 0;
    }
}
