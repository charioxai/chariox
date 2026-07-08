use crate::app::provider_output_fanout::ProviderOutputFanout;
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
            SessionHistoryEntry::user_prompt_with_attachments(
                session_id,
                source_attachment_id,
                agent_id,
                render_prompt_transcript(prompt, attachments),
                attachments,
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
        let session = crate::app::KernelSessionReadService::new(self)
            .session_snapshot_without_projection_update(session_id)?;
        let entry = SessionHistoryEntry::user_prompt_with_attachments(
            session_id,
            source_attachment_id,
            agent_id,
            render_prompt_transcript(prompt, attachments),
            attachments,
        );
        self.spawn_history_append(session, entry);
        Ok(())
    }

    pub(crate) fn spawn_user_prompt_history_append_with_prompt_id(
        &self,
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
        prompt_id: &str,
        _workflow_run_id: Option<&str>,
        _workflow_node_run_id: Option<&str>,
    ) -> Result<(), crate::error::DaemonError> {
        let session = crate::app::KernelSessionReadService::new(self)
            .session_snapshot_without_projection_update(session_id)?;
        let mut entry = SessionHistoryEntry::user_prompt_with_attachments(
            session_id,
            source_attachment_id,
            agent_id,
            render_prompt_transcript(prompt, attachments),
            attachments,
        );
        entry.merge_key = Some(format!("prompt:{prompt_id}"));
        self.spawn_history_append(session, entry);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fan_out_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        ProviderOutputFanout::new(self).fan_out(
            session_id,
            provider_run_id,
            kind,
            merge_key,
            recipient_attachment_ids,
            bytes,
        )
    }

    pub(crate) fn fan_out_output_for_agent(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        kind: TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> TerminalOutputRecord {
        ProviderOutputFanout::new(self).fan_out_for_agent(
            session_id,
            provider_run_id,
            agent_id,
            kind,
            merge_key,
            recipient_attachment_ids,
            bytes,
        )
    }

    pub(crate) fn record_notice(
        &mut self,
        session_id: &str,
        provider_run_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) -> RuntimeNoticeRecord {
        ProviderOutputFanout::new(self).record_notice(
            session_id,
            provider_run_id,
            recipient_attachment_ids,
            message,
        )
    }

    pub(crate) fn record_notice_for_agent(
        &mut self,
        session_id: &str,
        provider_run_id: Option<&str>,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) -> RuntimeNoticeRecord {
        ProviderOutputFanout::new(self).record_notice_for_agent(
            session_id,
            provider_run_id,
            agent_id,
            recipient_attachment_ids,
            message,
        )
    }

    pub(crate) fn record_assistant_message_completion(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) {
        ProviderOutputFanout::new(self).record_assistant_message_completion(
            session_id,
            provider_run_id,
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
    }

    pub(crate) fn record_assistant_message_completion_for_agent(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) {
        ProviderOutputFanout::new(self).record_assistant_message_completion_for_agent(
            session_id,
            provider_run_id,
            agent_id,
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
    }

    #[cfg(test)]
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
        }
        self.append_operational_history_entry(&entry);
    }

    pub(crate) fn replace_history_entry_by_merge_key_or_append(
        &self,
        session_id: &str,
        merge_key: &str,
        entry: SessionHistoryEntry,
    ) {
        let session = match self.sessions.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping history replacement because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "merge_key": merge_key,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        match self
            .history
            .replace_by_merge_key(&session, merge_key, &entry)
        {
            Ok(true) => {}
            Ok(false) => {
                if let Err(error) = self.history.append(&session, &entry) {
                    crate::logging::warn_with_fields(
                        "daemon.history",
                        "failed to append legacy session history entry after replacement miss",
                        serde_json::json!({
                            "session_id": session_id,
                            "merge_key": merge_key,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "failed to replace legacy session history entry",
                    serde_json::json!({
                        "session_id": session_id,
                        "merge_key": merge_key,
                        "error": error.to_string(),
                    }),
                );
            }
        }

        let context = self.history_event_context(&entry);
        match self.operational_history.replace_transcript_by_merge_key(
            session_id,
            entry.agent_id.as_deref(),
            merge_key,
            &entry,
            context,
        ) {
            Ok(Some(event)) => {
                if self.history_archive_enabled() {
                    self.enqueue_history_archive_event(&event);
                }
            }
            Ok(None) => {
                self.append_operational_history_entry(&entry);
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "failed to replace operational session history entry",
                    serde_json::json!({
                        "session_id": session_id,
                        "merge_key": merge_key,
                        "error": error.to_string(),
                    }),
                );
            }
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
        let agent_id = entry.agent_id.clone().or_else(|| {
            provider_run
                .as_ref()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        let session = self.sessions.get_session(&entry.session_id).ok();
        let active_prompt = session.as_ref().and_then(|session| {
            agent_id.as_deref().and_then(|agent_id| {
                self.prompt_state_owner
                    .active_prompt_for_agent(session, agent_id)
            })
        });
        let prompt_id = active_prompt.as_ref().map(|prompt| prompt.id().to_string());
        let active_turn = entry
            .provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.active_turns.get(provider_run_id));
        let prompt_id = active_turn
            .as_ref()
            .map(|turn| turn.prompt_id.clone())
            .or(prompt_id);
        let external_turn_id = entry
            .external_provider_observed_turn_id()
            .map(str::to_string);
        let turn_id = external_turn_id
            .clone()
            .or_else(|| active_turn.as_ref().map(|turn| turn.trace_id.clone()))
            .or_else(|| prompt_id.clone());
        HistoryEventTurnContext {
            session_id: Some(entry.session_id.clone()),
            agent_id,
            provider: provider_run.as_ref().map(|run| run.provider().to_string()),
            model: provider_run.as_ref().map(|run| run.model().to_string()),
            turn_id,
            prompt_id,
            provider_run_id: entry.provider_run_id.clone(),
            provider_session_id: provider_run
                .as_ref()
                .and_then(|run| run.provider_session_id().map(str::to_string)),
            workflow_run_id: active_prompt
                .as_ref()
                .and_then(|prompt| prompt.workflow_run_id().map(str::to_string)),
            workflow_node_id: active_prompt
                .as_ref()
                .and_then(|prompt| prompt.workflow_node_run_id().map(str::to_string)),
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
            }
            match operational_history.append_transcript(&entry, context) {
                Ok(event) => {
                    if archive_enabled {
                        if let Err(error) = operational_history.enqueue_archive_events(&[event]) {
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
        prompt_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) {
        let recipient_attachment_ids = self.other_attachment_ids(session_id, source_attachment_id);
        self.echo_prompt_to_attachments(
            session_id,
            provider_run_id,
            prompt_id,
            source_attachment_id,
            prompt,
            attachments,
            recipient_attachment_ids,
        );
    }

    pub(crate) fn echo_promoted_queued_prompt_to_attachments(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        prompt_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) {
        let recipient_attachment_ids = self.attachments.list_session_attachment_ids(session_id);
        self.echo_prompt_to_attachments(
            session_id,
            provider_run_id,
            prompt_id,
            source_attachment_id,
            prompt,
            attachments,
            recipient_attachment_ids,
        );
    }

    fn echo_prompt_to_attachments(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        prompt_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
        recipient_attachment_ids: Vec<String>,
    ) {
        if recipient_attachment_ids.is_empty() {
            return;
        }
        let mut bytes = render_prompt_transcript(prompt, attachments).into_bytes();
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        let agent_id = self
            .providers
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        let prompt_origin = agent_id.as_deref().and_then(|agent_id| {
            self.sessions
                .get_session(session_id)
                .ok()
                .and_then(|session| {
                    self.prompt_state_owner
                        .active_prompt_for_agent(&session, agent_id)
                })
                .filter(|prompt| prompt.id() == prompt_id)
                .map(|prompt| prompt.prompt_origin())
        });
        self.terminal.fan_out_prompt_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            prompt_id,
            prompt_origin,
            source_attachment_id,
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
    use std::fs;

    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::HistoryArchiveMode;
    use crate::session::{CreateSessionRequest, PromptStatus};
    use crate::terminal::TerminalOutputKind;
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

    #[test]
    fn user_prompt_history_persists_operational_when_legacy_append_fails() {
        let config = DaemonConfig::for_tests();
        let legacy_history_root = config.session_history_root.clone();
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "cli-legacy-history-fail",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should create");

        let _ = fs::remove_dir_all(&legacy_history_root);
        fs::write(&legacy_history_root, b"not a directory")
            .expect("fixture should block legacy history writes");

        app.append_user_prompt_history(session.id(), attachment.id(), agent.id(), "reload me", &[]);

        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("canonical operational history should load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text.trim_end(), "reload me");

        let _ = fs::remove_file(&legacy_history_root);
    }

    #[test]
    fn spawned_user_prompt_history_append_does_not_publish_session_projection() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "cli-history-read-only",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should create");
        let projection = app.session_state_projection_store();
        let before_sequence = projection.change_sequence();

        app.spawn_user_prompt_history_append_with_prompt_id(
            session.id(),
            attachment.id(),
            agent.id(),
            "history without projection churn",
            &[],
            "prompt-history-read-only",
            None,
            None,
        )
        .expect("history append should prepare");

        assert_eq!(
            projection.change_sequence(),
            before_sequence,
            "history-only prompt persistence must not wake session projection subscribers"
        );
        let entries = app
            .load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load");
        assert!(
            entries
                .iter()
                .any(|entry| entry.text.contains("history without projection churn")),
            "prompt history should still be persisted: {entries:?}"
        );
    }

    #[test]
    fn provider_output_fanout_history_uses_active_turn_trace_id() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-app-fanout-turn",
                "worktree-app-fanout-turn",
            ))
            .expect("session should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "cli-app-fanout-turn",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should create");
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run should launch");
        app.prompt_owner_activate_prompt(
            session.id(),
            crate::session::PromptQueueItem::new(
                "prompt-app-fanout-turn",
                attachment.id(),
                agent.id(),
                "prompt",
                PromptStatus::Queued,
            ),
        )
        .expect("prompt should activate");
        app.active_turns.start(
            crate::app::ActiveTurnState::new(
                session.id().to_string(),
                agent.id().to_string(),
                "prompt-app-fanout-turn".to_string(),
                run.id().to_string(),
            )
            .with_trace_id("trace-app-fanout-turn"),
        );

        app.fan_out_output(
            session.id(),
            run.id(),
            TerminalOutputKind::ProviderOutput,
            Some("app-fanout-turn-output".to_string()),
            vec![attachment.id().to_string()],
            b"output",
        );

        let events = app
            .operational_history_store()
            .load_session_events(session.id(), Some(agent.id()))
            .expect("operational history should load");
        let event = events
            .iter()
            .find(|event| {
                event
                    .metadata
                    .get("merge_key")
                    .and_then(|value| value.as_str())
                    == Some("app-fanout-turn-output")
            })
            .expect("provider output event should exist");
        assert_eq!(event.turn_id.as_deref(), Some("trace-app-fanout-turn"));
        assert_eq!(event.prompt_id.as_deref(), Some("prompt-app-fanout-turn"));
    }

    #[test]
    fn direct_history_append_uses_active_turn_trace_id() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-direct-history-turn",
                "worktree-direct-history-turn",
            ))
            .expect("session should create");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "cli-direct-history-turn",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should create");
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run should launch");
        app.prompt_owner_activate_prompt(
            session.id(),
            crate::session::PromptQueueItem::new(
                "prompt-direct-history-turn",
                attachment.id(),
                agent.id(),
                "prompt",
                PromptStatus::Queued,
            ),
        )
        .expect("prompt should activate");
        app.active_turns.start(
            crate::app::ActiveTurnState::new(
                session.id().to_string(),
                agent.id().to_string(),
                "prompt-direct-history-turn".to_string(),
                run.id().to_string(),
            )
            .with_trace_id("trace-direct-history-turn"),
        );

        let mut entry = crate::history::SessionHistoryEntry::provider_output(
            session.id(),
            run.id(),
            Some(agent.id()),
            TerminalOutputKind::ProviderOutput,
            Some("direct-history-turn-output".to_string()),
            "output",
        );
        entry.prompt_origin = Some(crate::session::PromptOrigin::Arroba);
        app.append_history_entry(session.id(), entry);

        let events = app
            .operational_history_store()
            .load_session_events(session.id(), Some(agent.id()))
            .expect("operational history should load");
        let event = events
            .iter()
            .find(|event| {
                event
                    .metadata
                    .get("merge_key")
                    .and_then(|value| value.as_str())
                    == Some("direct-history-turn-output")
            })
            .expect("provider output event should exist");
        assert_eq!(event.turn_id.as_deref(), Some("trace-direct-history-turn"));
        assert_eq!(
            event.prompt_id.as_deref(),
            Some("prompt-direct-history-turn")
        );
    }
}
