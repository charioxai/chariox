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
        self.kernel_sessions()
            .delete_session_ref(session_ref, workspace_id)
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
