use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn persist_session_activity(
        &self,
        session: &mut crate::session::RuntimeSession,
        timestamp_ms: u64,
    ) -> Result<(), DaemonError> {
        if session.status() == crate::session::SessionStatus::Ended
            || timestamp_ms <= session.last_activity_at_ms()
        {
            return Ok(());
        }
        self.durable_state_store.append_event(
            "session.activity.updated",
            Some(session.id().to_owned()),
            serde_json::json!({"last_activity_at_ms": timestamp_ms}),
        )?;
        session.note_activity_at(timestamp_ms);
        Ok(())
    }

    pub(super) fn record_session_activity(
        &self,
        session_id: &str,
        timestamp_ms: u64,
    ) -> Result<(), DaemonError> {
        {
            let mut sessions = self.session_store.write();
            let mut session = sessions.get_session(session_id)?;
            if session.status() == crate::session::SessionStatus::Ended {
                return Ok(());
            }
            self.persist_session_activity(&mut session, timestamp_ms)?;
            sessions.restore_session(session);
        }
        self.session_projection
            .note_session_activity(session_id, timestamp_ms);
        self.runtime_projection_changes.record_change();
        Ok(())
    }
}
