use super::*;

impl SessionService {
    pub(crate) fn retire_ended_session(&mut self, session_id: &str) {
        if self.durable_state.is_some()
            && self
                .store
                .get(session_id)
                .is_some_and(|session| session.status() == SessionStatus::Ended)
        {
            self.store.remove(session_id);
            self.room_environments.remove(session_id);
            self.ephemeral_session_ids.remove(session_id);
        }
    }

    pub(super) fn ensure_resident_session(&mut self, session_id: &str) -> Result<(), DaemonError> {
        if self.store.get(session_id).is_none() {
            let session = self.get_session(session_id)?;
            self.store.insert(session);
        }
        Ok(())
    }

    pub(super) fn retired_session(
        &self,
        session_id: &str,
    ) -> Result<Option<RuntimeSession>, DaemonError> {
        self.durable_state
            .as_ref()
            .map(|state| state.retired_session(&self.host_daemon_id, session_id))
            .transpose()
            .map(Option::flatten)
    }
}
