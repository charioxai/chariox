//! Runtime notice fan-out to terminal streams and history projections.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn record_notice(
        &self,
        session_id: &str,
        provider_run_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) {
        let agent_id = provider_run_id.and_then(|run_id| {
            self.provider_store
                .get_run(run_id)
                .ok()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        self.record_notice_for_agent(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message,
        );
    }

    pub(super) fn record_notice_for_agent(
        &self,
        session_id: &str,
        provider_run_id: Option<&str>,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) {
        let message = message.into();
        let recipient_attachment_ids =
            self.private_recipient_attachment_ids(agent_id, recipient_attachment_ids);
        let recipient_attachment_ids =
            self.with_metaagent_trace_recipient_ids(session_id, agent_id, recipient_attachment_ids);
        self.terminal_stream.record_notice(
            session_id,
            provider_run_id,
            agent_id,
            recipient_attachment_ids,
            message.clone(),
        );
        self.notify_metaagent_trace_activity(session_id, agent_id);
        let session = match self.session_store.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping history append because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        let entry = SessionHistoryEntry::notice(session_id, provider_run_id, agent_id, message);
        if let Err(error) = self.history_store.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        }
        self.append_operational_history_entry(&entry, None, None, None);
    }
}
