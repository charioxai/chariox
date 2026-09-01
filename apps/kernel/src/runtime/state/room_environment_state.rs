use std::collections::BTreeSet;

use crate::session::{
    agent_environment_actor_id, human_environment_actor_id, human_environment_actor_label,
    ActionAdmission, ActionCancellationOutcome, CanonicalViewport, EnvironmentActionHistoryPage,
    EnvironmentActionRequest, EnvironmentActionTerminal, EnvironmentActor, EnvironmentActorKind,
    EnvironmentComponent, EnvironmentComponentHealthState, EnvironmentError, EnvironmentLifecycle,
    EnvironmentReplay, RoomEnvironmentSnapshot,
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

    pub(crate) fn room_environment_events_after(
        &self,
        session_id: &str,
        cursor: u64,
    ) -> Result<EnvironmentReplay, EnvironmentError> {
        self.owned
            .session_store
            .room_environment_events_after(session_id, cursor)
    }

    pub(crate) fn room_environment_action_history(
        &self,
        session_id: &str,
        before_sequence: Option<u64>,
        limit: usize,
    ) -> Result<EnvironmentActionHistoryPage, EnvironmentError> {
        self.owned
            .session_store
            .room_environment_action_history(session_id, before_sequence, limit)
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

    pub(crate) fn begin_stop_room_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .begin_stop_room_environment(session_id)
    }

    pub(crate) fn complete_stop_room_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .complete_stop_room_environment(session_id)
    }

    pub(crate) fn retry_room_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned.session_store.retry_room_environment(session_id)
    }

    pub(crate) fn transition_room_environment(
        &self,
        session_id: &str,
        lifecycle: EnvironmentLifecycle,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .transition_room_environment(session_id, lifecycle)
    }

    pub(crate) fn update_room_environment_component_health(
        &self,
        session_id: &str,
        component: EnvironmentComponent,
        state: EnvironmentComponentHealthState,
        diagnostic_code: Option<&str>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .update_room_environment_component_health(session_id, component, state, diagnostic_code)
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

    pub(crate) fn update_room_environment_pointer_as_actor(
        &self,
        session_id: &str,
        actor: EnvironmentActor,
        runtime_generation: u64,
        viewport_revision: u64,
        position: Option<crate::session::EnvironmentPointerPosition>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .update_room_environment_pointer_as_actor(
                session_id,
                actor,
                runtime_generation,
                viewport_revision,
                position,
            )
    }

    pub(crate) fn reconcile_room_environment_actors(
        &self,
        session_id: &str,
        additional_user_id: Option<&str>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let mut user_ids = self
            .owned
            .attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter_map(|attachment_id| {
                self.owned
                    .attachment_store
                    .get_attachment(&attachment_id)
                    .ok()
                    .map(|attachment| attachment.owner_user_id().to_string())
            })
            .collect::<BTreeSet<_>>();
        user_ids.extend(additional_user_id.map(str::to_string));

        let mut actors = user_ids
            .into_iter()
            .map(|user_id| {
                EnvironmentActor::new(
                    human_environment_actor_id(&user_id),
                    EnvironmentActorKind::Human,
                    human_environment_actor_label(&user_id),
                )
            })
            .collect::<Vec<_>>();
        actors.extend(
            self.owned
                .agent_store
                .get_session_agents(session_id)
                .into_iter()
                .map(|agent| {
                    let display_label = agent.alias().unwrap_or(agent.agent_ref());
                    EnvironmentActor::new(
                        agent_environment_actor_id(agent.id()),
                        EnvironmentActorKind::Agent,
                        display_label,
                    )
                }),
        );

        self.owned
            .session_store
            .reconcile_room_environment_actors(session_id, actors)
    }

    pub(crate) fn reconcile_room_environment_controller_tabs(
        &self,
        session_id: &str,
        tabs: Vec<crate::session::EnvironmentTabObservation>,
        focused_runtime_target_id: Option<&str>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .reconcile_room_environment_controller_tabs(session_id, tabs, focused_runtime_target_id)
    }

    pub(crate) fn room_environment_controller_tab_binding(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<crate::session::EnvironmentTabRuntimeBinding, EnvironmentError> {
        self.owned
            .session_store
            .room_environment_controller_tab_binding(session_id, tab_id)
    }

    pub(crate) fn room_environment_tab_id_for_controller_target(
        &self,
        session_id: &str,
        controller_target_id: &str,
    ) -> Result<Option<String>, EnvironmentError> {
        self.owned
            .session_store
            .room_environment_tab_id_for_controller_target(session_id, controller_target_id)
    }

    pub(crate) fn register_room_environment_element_references(
        &self,
        session_id: &str,
        tab_id: &str,
        runtime_generation: u64,
        document_revision: u64,
        controller_node_refs: impl IntoIterator<Item = String>,
    ) -> Result<std::collections::BTreeMap<String, String>, EnvironmentError> {
        self.owned
            .session_store
            .register_room_environment_element_references(
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
        self.owned
            .session_store
            .resolve_room_environment_element_reference(session_id, reference_id)
    }

    pub(crate) fn submit_room_environment_action(
        &self,
        session_id: &str,
        request: EnvironmentActionRequest,
    ) -> Result<(ActionAdmission, RoomEnvironmentSnapshot), EnvironmentError> {
        self.owned
            .session_store
            .submit_room_environment_action(session_id, request)
    }

    pub(crate) fn existing_room_environment_action(
        &self,
        session_id: &str,
        request: &EnvironmentActionRequest,
    ) -> Result<Option<ActionAdmission>, EnvironmentError> {
        self.owned
            .session_store
            .existing_room_environment_action(session_id, request)
    }

    pub(crate) fn finish_room_environment_action(
        &self,
        session_id: &str,
        action_id: &str,
        terminal: EnvironmentActionTerminal,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .finish_room_environment_action(session_id, action_id, terminal)
    }

    pub(crate) fn begin_room_environment_browser_controller_recovery(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .begin_room_environment_browser_controller_recovery(session_id)
    }

    pub(crate) fn complete_room_environment_browser_controller_recovery(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .complete_room_environment_browser_controller_recovery(session_id)
    }

    pub(crate) fn request_room_environment_takeover_as_actor(
        &self,
        session_id: &str,
        actor: EnvironmentActor,
        target: crate::session::InputTarget,
    ) -> Result<(crate::session::TakeoverOutcome, RoomEnvironmentSnapshot), EnvironmentError> {
        self.owned
            .session_store
            .request_room_environment_takeover_as_actor(session_id, actor, target)
    }

    pub(crate) fn release_room_environment_input(
        &self,
        session_id: &str,
        actor_id: &str,
        target: &crate::session::InputTarget,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.owned
            .session_store
            .release_room_environment_input(session_id, actor_id, target)
    }

    pub(crate) fn cancel_room_environment_action_as_actor(
        &self,
        session_id: &str,
        actor: EnvironmentActor,
        action_id: &str,
    ) -> Result<(ActionCancellationOutcome, RoomEnvironmentSnapshot), EnvironmentError> {
        self.owned
            .session_store
            .cancel_room_environment_action_as_actor(session_id, actor, action_id)
    }
}
