//! Runtime notice fan-out to terminal streams and history projections.

use super::*;

impl KernelRuntimeState {
    pub(crate) fn record_meta_migration_notice(&self, session_id: &str, agent_id: &str) {
        self.owned.record_notice_for_agent(session_id, None, Some(agent_id),
            self.owned.attachment_store.list_session_attachment_ids(session_id),
            "/sudo <prompt> replaces /meta. /meta remains delegation-only for one release and needs no passkey. /sudo requires a fresh passkey in the kernel popup and grants a finite window for the same owner-authorized work (one hour by default). Agents cannot answer owner approvals.");
    }
}

impl KernelRuntimeOwnedState {
    /// Tell every attached session once when kernel storage fills, has space
    /// again, or stops saving, so the owner sees why changes are refused. With
    /// no session attached the change waits for one.
    pub(super) fn announce_durable_writer_condition(&self) {
        let sessions = self.attachment_store.list_attached_session_ids();
        if sessions.is_empty() {
            return;
        }
        let Some(condition) = self.durable_state_store.take_writer_condition_change() else {
            return;
        };
        for session_id in sessions {
            let recipients = self
                .attachment_store
                .list_session_attachment_ids(&session_id);
            self.record_notice(&session_id, None, recipients, condition.notice());
        }
    }

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
        let recipient_attachment_ids = self.agent_trace_recipient_attachment_ids(
            session_id,
            agent_id,
            recipient_attachment_ids,
        );
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
        if let Err(error) = self.session_store.get_session(session_id) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "skipping notice history append because session lookup failed",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
            return;
        }
        let entry = SessionHistoryEntry::notice(session_id, provider_run_id, agent_id, message);
        self.append_operational_history_entry(&entry, None, None, None);
    }
}
