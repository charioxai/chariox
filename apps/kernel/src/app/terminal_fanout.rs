use crate::app::DaemonApp;
use crate::history::{HistoryEventTurnContext, SessionHistoryEntry, SessionHistoryEntryKind};
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
            self.append_operational_history_entry(&entry);
            self.history_projection.append(entry);
        }
    }

    fn append_operational_history_entry(&self, entry: &SessionHistoryEntry) {
        let context = self.history_event_context(entry);
        match self.operational_history.append_transcript(entry, context) {
            Ok(event) => {
                if is_unread_output_history_entry(entry) {
                    if let Some(agent_id) = entry.agent_id.as_deref() {
                        let _ = self.sessions.note_agent_output_sequence(
                            entry.session_id.as_str(),
                            agent_id,
                            event.sequence,
                        );
                    }
                }
                if self.history_archive_enabled() {
                    self.enqueue_history_archive_event(&event);
                }
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "failed to append operational history",
                    serde_json::json!({
                        "session_id": entry.session_id.as_str(),
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    fn history_event_context(&self, entry: &SessionHistoryEntry) -> HistoryEventTurnContext {
        let provider_run = entry
            .provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.providers.get_run(provider_run_id).ok());
        HistoryEventTurnContext {
            session_id: Some(entry.session_id.clone()),
            agent_id: entry.agent_id.clone().or_else(|| {
                provider_run
                    .as_ref()
                    .and_then(|run| run.agent_instance_id().map(str::to_string))
            }),
            provider: provider_run.as_ref().map(|run| run.provider().to_string()),
            model: provider_run.as_ref().map(|run| run.model().to_string()),
            provider_run_id: entry.provider_run_id.clone(),
            provider_session_id: provider_run
                .as_ref()
                .and_then(|run| run.provider_session_id().map(str::to_string)),
            worktree_path: provider_run.as_ref().and_then(|run| {
                run.working_directory()
                    .map(|path| path.display().to_string())
            }),
            ..HistoryEventTurnContext::default()
        }
    }

    fn enqueue_history_archive_event(&self, event: &crate::history::HistoryEvent) {
        if let Err(error) = self
            .operational_history
            .enqueue_archive_events(std::slice::from_ref(event))
        {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to enqueue history archive event",
                serde_json::json!({
                    "session_id": event.session_id.as_deref(),
                    "error": error.to_string(),
                }),
            );
        }
    }

    fn spawn_history_append(
        &self,
        session: crate::session::RuntimeSession,
        entry: SessionHistoryEntry,
    ) {
        let history = self.history.clone();
        let operational_history = self.operational_history.clone();
        let archive_enabled = self.history_archive_enabled();
        let history_projection = self.history_projection.clone();
        let context = self.history_event_context(&entry);
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
                match operational_history.append_transcript(&entry, context) {
                    Ok(event) => {
                        if archive_enabled {
                            if let Err(error) = operational_history.enqueue_archive_events(&[event])
                            {
                                crate::logging::warn_with_fields(
                                    "daemon.history",
                                    "failed to enqueue history archive event",
                                    serde_json::json!({
                                        "session_id": session_id,
                                        "error": error.to_string(),
                                    }),
                                );
                            }
                        }
                    }
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "daemon.history",
                            "failed to append operational history",
                            serde_json::json!({
                                "session_id": session_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
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

fn is_unread_output_history_entry(entry: &SessionHistoryEntry) -> bool {
    matches!(
        entry.kind,
        SessionHistoryEntryKind::ProviderOutput
            | SessionHistoryEntryKind::ProviderReasoning
            | SessionHistoryEntryKind::ProviderTool
            | SessionHistoryEntryKind::ProviderError
            | SessionHistoryEntryKind::Notice
    )
}

#[cfg(test)]
mod tests {
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::HistoryArchiveMode;
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn user_prompt_history_enqueues_archive_outbox_when_external_archive_enabled() {
        let mut config = DaemonConfig::for_tests();
        config.user_config.history.archive.mode = HistoryArchiveMode::External;
        config.user_config.history.archive.url = Some("http://127.0.0.1:9".to_string());
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "cli-archive-outbox",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should create");

        app.append_user_prompt_history(
            session.id(),
            attachment.id(),
            agent.id(),
            "archive me",
            &[],
        );

        let pending = app
            .operational_history_store()
            .load_pending_archive_events(10)
            .expect("pending archive events should load");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event.session_id.as_deref(), Some(session.id()));
        assert_eq!(pending[0].event.agent_id.as_deref(), Some(agent.id()));
        assert_eq!(
            pending[0].event.content.as_deref().map(str::trim_end),
            Some("archive me")
        );
    }
}
