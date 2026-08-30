use crate::session::{CanonicalViewport, EnvironmentError, RoomEnvironmentSnapshot};

use super::KernelRuntimeState;

impl KernelRuntimeState {
    pub(crate) fn room_environment_snapshot(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .room_environment_snapshot(session_id)
    }

    pub(crate) fn start_room_environment(
        &self,
        session_id: &str,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .start_room_environment(session_id, viewport)
    }

    pub(crate) fn stop_room_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned.session_store.stop_room_environment(session_id)
    }

    pub(crate) fn retry_room_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned.session_store.retry_room_environment(session_id)
    }
}
