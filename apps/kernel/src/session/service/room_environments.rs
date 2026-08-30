use super::*;

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
}
