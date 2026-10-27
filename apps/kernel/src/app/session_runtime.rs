use crate::app::DaemonApp;

impl DaemonApp {
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
