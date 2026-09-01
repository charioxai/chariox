use super::*;
use crate::session::{
    ActionCancellationOutcome, EnvironmentActor, EnvironmentLifecycle, InputTarget, TakeoverOutcome,
};

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

    pub(crate) fn room_environment_events_after(
        &self,
        session_id: &str,
        cursor: u64,
    ) -> Result<EnvironmentReplay, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.events_after(session_id, cursor)
    }

    pub(crate) fn room_environment_action_history(
        &self,
        session_id: &str,
        before_sequence: Option<u64>,
        limit: usize,
    ) -> Result<crate::session::EnvironmentActionHistoryPage, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments
            .action_history(session_id, before_sequence, limit)
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

    pub(crate) fn update_room_environment_viewport_as_actor(
        &mut self,
        session_id: &str,
        actor: EnvironmentActor,
        expected_revision: u64,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.update_viewport_as_actor(
            session_id,
            actor,
            expected_revision,
            viewport,
        )
    }

    pub(crate) fn reconcile_room_environment_actors(
        &mut self,
        session_id: &str,
        actors: Vec<EnvironmentActor>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.reconcile_actors(session_id, actors)
    }

    pub(crate) fn request_room_environment_takeover_as_actor(
        &mut self,
        session_id: &str,
        actor: EnvironmentActor,
        target: InputTarget,
    ) -> Result<(TakeoverOutcome, RoomEnvironmentSnapshot), EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments
            .request_takeover_as_actor(session_id, actor, target)
    }

    pub(crate) fn release_room_environment_input(
        &mut self,
        session_id: &str,
        actor_id: &str,
        target: &InputTarget,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments
            .release_input(session_id, actor_id, target)
    }

    pub(crate) fn cancel_room_environment_action_as_actor(
        &mut self,
        session_id: &str,
        actor: EnvironmentActor,
        action_id: &str,
    ) -> Result<(ActionCancellationOutcome, RoomEnvironmentSnapshot), EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments
            .cancel_action_as_actor(session_id, actor, action_id)
    }
}
