//! Room stars are persisted and published through the ordinary session projection.
use super::*;
use crate::session::RuntimeSession;

impl KernelRuntimeState {
    pub(crate) async fn set_room_pinned(
        &self,
        session_id: &str,
        pinned: bool,
        caller_user_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        {
            let mut sessions = self.owned.session_store.write();
            let mut session = sessions.get_session(session_id)?;
            if session.owner_user_id() != caller_user_id {
                return Err(DaemonError::OwnershipAccessDenied {
                    user_id: caller_user_id.to_string(),
                    owner_user_id: session.owner_user_id().to_string(),
                    resource: session_id.to_string(),
                    operation: "pin Room",
                });
            }
            session.set_pinned(pinned);
            sessions.restore_session(session);
        }
        // Agents, tasks, wakes and runtime state are hydrated from their own stores.
        let session = self
            .owned
            .session_snapshot_without_projection_update(session_id)?;
        self.append_session_durable_event("session.updated", &session, "room_pin_changed")
            .await?;
        self.owned.runtime_projection_changes.record_change();
        Ok(session)
    }
}
