use super::*;
use crate::session::{
    ActionCancellationOutcome, EnvironmentActor, EnvironmentComponent,
    EnvironmentComponentHealthState, EnvironmentLifecycle, EnvironmentTabObservation, InputTarget,
    TakeoverOutcome,
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

    pub(crate) fn begin_stop_room_environment(
        &mut self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.begin_stop(session_id)
    }

    pub(crate) fn complete_stop_room_environment(
        &mut self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.complete_stop(session_id)
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

    pub(crate) fn update_room_environment_component_health(
        &mut self,
        session_id: &str,
        component: EnvironmentComponent,
        state: EnvironmentComponentHealthState,
        diagnostic_code: Option<&str>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.update_component_health(
            session_id,
            component,
            state,
            diagnostic_code,
        )
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

    pub(crate) fn reconcile_room_environment_controller_tabs(
        &mut self,
        session_id: &str,
        tabs: Vec<EnvironmentTabObservation>,
        focused_runtime_target_id: Option<&str>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.reconcile_controller_tabs(
            session_id,
            tabs,
            focused_runtime_target_id,
        )
    }

    pub(crate) fn room_environment_controller_tab_binding(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<crate::session::EnvironmentTabRuntimeBinding, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments
            .controller_tab_binding(session_id, tab_id)
    }

    pub(crate) fn register_room_environment_element_references(
        &mut self,
        session_id: &str,
        tab_id: &str,
        runtime_generation: u64,
        document_revision: u64,
        controller_node_refs: impl IntoIterator<Item = String>,
    ) -> Result<std::collections::BTreeMap<String, String>, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments.register_element_references(
            session_id,
            tab_id,
            runtime_generation,
            document_revision,
            controller_node_refs,
        )
    }

    pub(crate) fn resolve_room_environment_element_reference(
        &self,
        session_id: &str,
        reference_id: &str,
    ) -> Result<crate::session::EnvironmentElementTarget, EnvironmentError> {
        if !self.has_session(session_id) {
            return Err(EnvironmentError::RoomNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.room_environments
            .resolve_element_reference(session_id, reference_id)
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
