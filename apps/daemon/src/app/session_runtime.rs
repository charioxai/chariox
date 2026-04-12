use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, RuntimeAttachment};
use crate::error::DaemonError;
use crate::session::{RuntimeSession, SessionStatus};

impl DaemonApp {
    pub fn attach(&mut self, request: AttachRequest) -> Result<RuntimeAttachment, DaemonError> {
        self.kernel_sessions().attach(request)
    }

    pub fn detach(&mut self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        self.kernel_sessions().detach(attachment_id)
    }

    pub fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let session = self.sessions.get_session(session_id)?;

        if session.status() == SessionStatus::Ended {
            return self.sessions.end_session(session_id);
        }

        let removed_attachments = self.attachments.remove_session_attachments(session_id);
        let terminated_runs = self
            .providers
            .terminate_session_runs(&mut self.sessions, session_id)?;
        let terminated_run_ids = terminated_runs
            .iter()
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        for run in terminated_runs {
            self.remove_tracked_provider_process_for_run(run.id())?;
        }

        let removed_agents = self.agents.remove_session_agents(session_id);
        let removed_agent_ids: Vec<_> = removed_agents
            .iter()
            .map(|agent| format!("{} ({})", agent.agent_ref(), agent.id()))
            .collect();

        for run in self.providers.list_runs() {
            if run.session_id() == session_id {
                crate::transport::flow_control::clear_prompt_activity(self, run.id());
            }
        }
        let ended = self.sessions.end_session(session_id)?;
        crate::logging::info_with_fields(
            "daemon.session",
            "session ended",
            serde_json::json!({
                "session_id": session_id,
                "removed_attachment_ids": removed_attachments.iter().map(|attachment| attachment.id().to_string()).collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "removed_agents": removed_agent_ids,
            }),
        );
        Ok(ended)
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
