use super::*;
use crate::durable_state::DurableKernelStateStore;

impl SessionService {
    pub(crate) fn with_room_environment_durability(
        mut self,
        state: DurableKernelStateStore,
    ) -> Self {
        self.room_environment_durable_state = Some(state);
        self
    }

    pub(crate) fn restore_room_environments(&mut self) -> Result<(), DaemonError> {
        let Some(state) = self.room_environment_durable_state.as_ref() else {
            return Ok(());
        };
        for mut environment in state.load_room_environments(&self.host_daemon_id)? {
            let session_id = environment.snapshot().session_id;
            // Session deletion/replay and kernel ownership are authoritative over retained records.
            if !self.has_session(&session_id) {
                continue;
            }
            environment.reconcile_after_kernel_restart();
            state.save_room_environment(&self.host_daemon_id, &environment)?;
            self.room_environments.restore(environment);
        }
        Ok(())
    }

    pub(super) fn mutate_room_environment<T>(
        &mut self,
        session_id: &str,
        mutate: impl FnOnce(&mut RoomEnvironmentRegistry) -> Result<T, EnvironmentError>,
    ) -> Result<T, EnvironmentError> {
        let previous = self.room_environments.environment(session_id).cloned();
        let result = mutate(&mut self.room_environments);
        let current = self.room_environments.environment(session_id);
        if current != previous.as_ref() {
            if let (Some(state), Some(environment)) =
                (&self.room_environment_durable_state, current)
            {
                if state
                    .save_room_environment(&self.host_daemon_id, environment)
                    .is_err()
                {
                    self.room_environments.remove(session_id);
                    if let Some(previous) = previous {
                        self.room_environments.restore(previous);
                    }
                    return Err(EnvironmentError::DurableStateUnavailable);
                }
            }
        }
        result
    }
}
