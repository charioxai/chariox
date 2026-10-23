//! MP-08 / MP-10 / MP-11: all terminal transports share the scoped Skills feed.
use super::*;
use crate::skill::{v2::SkillCatalogEvent, watch::SkillWatchCursor};
impl KernelRuntimeState {
    pub(crate) async fn skill_subscription_events(
        &self,
        session_id: &str,
        attachment_id: &str,
        cursor: SkillWatchCursor,
    ) -> Result<(SkillWatchCursor, Vec<SkillCatalogEvent>), DaemonError> {
        if !self.skills_v2_runtime_available() {
            return Ok((cursor, vec![]));
        }
        let state = self.clone();
        let session_id = session_id.to_owned();
        let attachment_id = attachment_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let attachment = state
                .owned
                .attachment_store
                .get_attachment(&attachment_id)?;
            let session = state.owned.session_store.get_session(&session_id)?;
            if attachment.session_id() != session_id
                || (!state.skills_host_owner(attachment.owner_user_id())
                    && !session.has_member(attachment.owner_user_id()))
            {
                return Err(DaemonError::SessionAccessDenied {
                    session_id,
                    user_id: attachment.owner_user_id().into(),
                });
            }
            state.skills_catalog_service()?.storage.watch_changes(
                &session_id,
                session.project_id(),
                cursor,
            )
        })
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "skills.v2.watch",
            message: "Skills event projection did not complete".into(),
        })?
    }
}
