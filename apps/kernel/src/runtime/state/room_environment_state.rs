use crate::session::{
    CanonicalViewport, EnvironmentActor, EnvironmentError, RoomEnvironmentSnapshot,
};

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

    pub(crate) fn update_room_environment_viewport_as_actor(
        &self,
        session_id: &str,
        actor: EnvironmentActor,
        expected_revision: u64,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .update_room_environment_viewport_as_actor(
                session_id,
                actor,
                expected_revision,
                viewport,
            )
    }
}
