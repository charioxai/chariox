use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::RwLock;

/// Actions hold shared access through execution, not only during admission.
/// Import must hold exclusive access before recording recovery state or writing cookies.
#[derive(Clone, Default)]
pub(super) struct EnvironmentExecutionGates {
    rooms: Arc<Mutex<BTreeMap<String, Weak<RwLock<()>>>>>,
}

impl EnvironmentExecutionGates {
    pub(super) fn for_room(&self, room: &str) -> Arc<RwLock<()>> {
        let mut rooms = self.rooms.lock().unwrap_or_else(|error| error.into_inner());
        rooms.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = rooms.get(room).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(RwLock::new(()));
        rooms.insert(room.to_owned(), Arc::downgrade(&gate));
        gate
    }
}

#[cfg(test)]
impl super::KernelRuntimeState {
    /// The destination executor must retain this guard through recovery and journal cleanup.
    /// This drains kernel actions; it does not suspend autonomous page/network writers.
    pub(crate) async fn begin_exclusive_browser_import(
        &self,
        session_id: &str,
        request_id: &str,
        user_id: &str,
    ) -> Result<tokio::sync::OwnedRwLockWriteGuard<()>, crate::error::DaemonError> {
        let guard = self
            .owned
            .environment_execution_gates
            .for_room(session_id)
            .write_owned()
            .await;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|_| denied())?;
        self.owned
            .durable_state_store
            .begin_browser_import_recovery(
                &environment.environment_id,
                request_id,
                user_id,
                session_id,
            )
            .map_err(|_| denied())?;
        Ok(guard)
    }
}

#[cfg(test)]
fn denied() -> crate::error::DaemonError {
    crate::error::DaemonError::LocalTransport {
        operation: "browser_import",
        message: "browser_import_denied".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn import_waits_for_actions_and_excludes_new_actions_in_only_its_room() {
        let gates = EnvironmentExecutionGates::default();
        let gate = gates.for_room("room");
        let action = gate.clone().read_owned().await;
        assert!(gate.try_write().is_err());
        assert!(gates.for_room("other").try_write().is_ok());
        let mut import = Box::pin(gate.clone().write_owned());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut import)
                .await
                .is_err()
        );
        assert!(gates.for_room("room").try_read().is_err());
        drop(action);
        let import = import.await;
        assert!(gates.for_room("room").try_read().is_err());
        drop(import);
        assert!(gate.try_read().is_ok());
    }
}
