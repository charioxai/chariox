use super::*;
use crate::session::EnvironmentLifecycle;

impl SessionService {
    pub(crate) fn create_room_environment(
        &mut self,
        session_id: &str,
        environment_id: impl Into<String>,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments
            .create(session_id, environment_id, viewport)
    }

    pub(crate) fn room_environment_snapshot(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.room_environments.snapshot(session_id)
    }

    pub(crate) fn start_room_environment(
        &mut self,
        session_id: &str,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.start(session_id, viewport)
    }

    pub(crate) fn stop_room_environment(
        &mut self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.stop(session_id)
    }

    pub(crate) fn retry_room_environment(
        &mut self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.retry(session_id)
    }

    // The managed controller adapter reports lifecycle completion in Milestone 2.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn transition_room_environment(
        &mut self,
        session_id: &str,
        lifecycle: EnvironmentLifecycle,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.transition(session_id, lifecycle)
    }
}
