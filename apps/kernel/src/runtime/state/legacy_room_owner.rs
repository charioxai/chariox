//! MP-08/MP-11 F4: the enrolled owner and the `local` owner are one person in a room that was
//! made before enrollment. Nothing is rewritten: the caller keeps its authenticated identity and
//! the stored owners and members keep theirs. This is the single place that relates the two.
use super::*;
use crate::session::{RuntimeSession, DEFAULT_LOCAL_USER_ID};

impl KernelRuntimeOwnedState {
    /// The identity a room owned by `room_owner_user_id` stores for `user_id`: `local` when
    /// `user_id` is the enrolled owner and the room predates enrollment, otherwise `user_id`.
    /// This is the one rule; the other methods apply it.
    pub(in crate::runtime) fn room_member_user_id_in(
        &self,
        room_owner_user_id: &str,
        user_id: &str,
    ) -> String {
        // Cheap alias test first: this runs for every command of a terminal.
        if room_owner_user_id == DEFAULT_LOCAL_USER_ID
            && self.owner_context_review_owner(DEFAULT_LOCAL_USER_ID) == user_id
        {
            DEFAULT_LOCAL_USER_ID.to_string()
        } else {
            user_id.to_string()
        }
    }

    /// Takes the session store's read lock: never call it while holding that lock, pass the
    /// room owner to the `_in` variants instead.
    pub(in crate::runtime) fn room_member_user_id(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> String {
        match self.session_store.session_owner_user_id(session_id) {
            Some(room_owner) => self.room_member_user_id_in(&room_owner, user_id),
            None => user_id.to_string(),
        }
    }

    /// Whether `user_id` is the owner `resource_owner_user_id` of a resource in a room owned by
    /// `room_owner_user_id`: the same identity, or the enrolled owner and a `local` resource of
    /// a pre-enrollment room.
    pub(in crate::runtime) fn acts_for_owner_in(
        &self,
        room_owner_user_id: &str,
        user_id: &str,
        resource_owner_user_id: &str,
    ) -> bool {
        user_id == resource_owner_user_id
            || self.room_member_user_id_in(room_owner_user_id, user_id) == resource_owner_user_id
    }

    /// `acts_for_owner_in` for a room looked up by id; same lock rule as `room_member_user_id`.
    pub(in crate::runtime) fn acts_for_owner(
        &self,
        session_id: &str,
        user_id: &str,
        resource_owner_user_id: &str,
    ) -> bool {
        user_id == resource_owner_user_id
            || self.room_member_user_id(session_id, user_id) == resource_owner_user_id
    }

    pub(in crate::runtime) fn is_room_member(
        &self,
        session: &RuntimeSession,
        user_id: &str,
    ) -> bool {
        session.has_member(&self.room_member_user_id_in(session.owner_user_id(), user_id))
    }

    /// The room's prompt rule for `user_id` and an agent owned by `agent_owner_user_id`.
    pub(in crate::runtime) fn can_prompt_agent_owned_by(
        &self,
        session: &RuntimeSession,
        user_id: &str,
        agent_owner_user_id: &str,
    ) -> bool {
        user_id == agent_owner_user_id
            || session.can_prompt_agent_owned_by(
                &self.room_member_user_id_in(session.owner_user_id(), user_id),
                agent_owner_user_id,
            )
    }
}

impl KernelRuntimeState {
    pub(in crate::runtime) fn room_member_user_id(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> String {
        self.owned.room_member_user_id(session_id, user_id)
    }

    pub(in crate::runtime) fn room_member_user_id_in(
        &self,
        room_owner_user_id: &str,
        user_id: &str,
    ) -> String {
        self.owned
            .room_member_user_id_in(room_owner_user_id, user_id)
    }

    pub(in crate::runtime) fn acts_for_owner(
        &self,
        session_id: &str,
        user_id: &str,
        resource_owner_user_id: &str,
    ) -> bool {
        self.owned
            .acts_for_owner(session_id, user_id, resource_owner_user_id)
    }

    pub(in crate::runtime) fn is_room_member(
        &self,
        session: &RuntimeSession,
        user_id: &str,
    ) -> bool {
        self.owned.is_room_member(session, user_id)
    }

    pub(in crate::runtime) fn can_prompt_agent_owned_by(
        &self,
        session: &RuntimeSession,
        user_id: &str,
        agent_owner_user_id: &str,
    ) -> bool {
        self.owned
            .can_prompt_agent_owned_by(session, user_id, agent_owner_user_id)
    }

    /// A provider run carries no session scope of its own; it belongs to the room it ran in.
    pub(in crate::runtime) fn provider_run_session_id(
        &self,
        provider_run_id: &str,
    ) -> Option<String> {
        match self.owned.provider_run_projection.get(provider_run_id) {
            Some(run) => Some(run.session_id().to_owned()),
            None => self
                .owned
                .provider_store
                .get_run(provider_run_id)
                .ok()
                .map(|run| run.session_id().to_owned()),
        }
    }
}
