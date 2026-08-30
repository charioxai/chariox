use std::collections::BTreeSet;

use crate::session::{
    agent_environment_actor_id, human_environment_actor_id, human_environment_actor_label,
    ActionCancellationOutcome, CanonicalViewport, EnvironmentActor, EnvironmentActorKind,
    EnvironmentError, EnvironmentReplay, RoomEnvironmentSnapshot,
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
