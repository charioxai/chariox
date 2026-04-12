use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, RuntimeAttachment};
use crate::error::DaemonError;
use crate::session::RuntimeSession;

impl DaemonApp {
    pub fn attach(&mut self, request: AttachRequest) -> Result<RuntimeAttachment, DaemonError> {
        self.kernel_sessions().attach(request)
    }

    pub fn detach(&mut self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        self.kernel_sessions().detach(attachment_id)
    }

    pub fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.kernel_sessions().end_session(session_id)
    }

    pub fn delete_session_ref(
        &mut self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self
            .sessions
            .resolve_session_ref(session_ref, workspace_id)?;
        let ended = self.end_session(session.id())?;
        let deleted = self.sessions.delete_session(ended.id())?;
        crate::logging::info_with_fields(
            "daemon.session",
            "session deleted",
            serde_json::json!({
                "session_id": deleted.id(),
                "session_alias": deleted.alias(),
            }),
        );
        Ok(deleted)
    }

    pub(crate) fn other_attachment_ids(
        &self,
        session_id: &str,
        source_attachment_id: &str,
    ) -> Vec<String> {
        self.attachments
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|attachment_id| attachment_id != source_attachment_id)
            .collect()
    }
}
