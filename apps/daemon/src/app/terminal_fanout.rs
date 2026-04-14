use crate::app::DaemonApp;
use crate::history::SessionHistoryEntry;
use crate::prompt_transcript::render_prompt_transcript;
use crate::session::PromptAttachment;
use crate::terminal::{RuntimeNoticeRecord, TerminalOutputKind, TerminalOutputRecord};

impl DaemonApp {
    #[cfg(test)]
    pub(crate) fn append_user_prompt_history(
        &self,
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) {
        self.append_history_entry(
            session_id,
            SessionHistoryEntry::user_prompt(
                session_id,
                source_attachment_id,
                agent_id,
                render_prompt_transcript(prompt, attachments),
            ),
        );
    }

    pub(crate) fn spawn_user_prompt_history_append(
        &self,
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) -> Result<(), crate::error::DaemonError> {
        let session =
            crate::app::KernelSessionReadService::new(self).session_snapshot(session_id)?;
        let entry = SessionHistoryEntry::user_prompt(
            session_id,
            source_attachment_id,
            agent_id,
            render_prompt_transcript(prompt, attachments),
        );
        self.spawn_history_append(session, entry);
        Ok(())
    }

    pub(crate) fn fan_out_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        let agent_id = self
            .providers
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        let record = self.terminal.fan_out_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            kind.clone(),
            merge_key.clone(),
            recipient_attachment_ids,
            bytes,
        );
        if kind != TerminalOutputKind::PromptEcho {
            self.append_history_entry(
                session_id,
                SessionHistoryEntry::provider_output(
                    session_id,
                    provider_run_id,
                    agent_id.as_deref(),
                    kind,
                    merge_key,
                    String::from_utf8_lossy(bytes).into_owned(),
                ),
            );
        }
        record
    }

    pub(crate) fn record_notice(
        &mut self,
        session_id: &str,
        provider_run_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) -> RuntimeNoticeRecord {
        let message = message.into();
        let agent_id = provider_run_id.and_then(|run_id| {
            self.providers
                .get_run(run_id)
                .ok()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        let record = self.terminal.record_notice(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message.clone(),
        );
        self.append_history_entry(
            session_id,
            SessionHistoryEntry::notice(session_id, provider_run_id, agent_id.as_deref(), message),
        );
        record
    }

    pub(crate) fn record_assistant_message_completion(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) {
        let agent_id = self
            .providers
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        self.terminal.record_assistant_message_completion(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
    }

    pub(crate) fn append_history_entry(&self, session_id: &str, entry: SessionHistoryEntry) {
        let session = match self.sessions.get_session(session_id) {
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
        if let Err(error) = self.history.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        } else {
            self.history_projection.append(entry);
        }
    }

    fn spawn_history_append(
        &self,
        session: crate::session::RuntimeSession,
        entry: SessionHistoryEntry,
    ) {
        let history = self.history.clone();
        let history_projection = self.history_projection.clone();
        let session_id = session.id().to_string();
        let append = move || {
            if let Err(error) = history.append(&session, &entry) {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "failed to append session history",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
            } else {
                history_projection.append(entry);
            }
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::spawn_blocking(append);
        } else {
            append();
        }
    }

    pub(crate) fn echo_prompt_to_other_attachments(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) {
        let recipient_attachment_ids = self.other_attachment_ids(session_id, source_attachment_id);
        if recipient_attachment_ids.is_empty() {
            return;
        }
        let mut bytes = render_prompt_transcript(prompt, attachments).into_bytes();
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        self.fan_out_output(
            session_id,
            provider_run_id,
            TerminalOutputKind::PromptEcho,
            None,
            recipient_attachment_ids,
            &bytes,
        );
    }
}
