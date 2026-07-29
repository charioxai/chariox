use crate::app::DaemonApp;
use crate::config::HistoryArchiveMode;
use crate::error::DaemonError;
use crate::history::SessionHistoryEntry;
use crate::session::RuntimeSession;

impl DaemonApp {
    pub(crate) fn history_archive_enabled(&self) -> bool {
        self.config.user_config.history.archive.mode == HistoryArchiveMode::External
    }

    pub fn load_session_history_entries(
        &self,
        session: &RuntimeSession,
        agent_id: Option<&str>,
    ) -> Result<Vec<SessionHistoryEntry>, DaemonError> {
        if !self
            .operational_history
            .legacy_fallback_disabled(session.id())?
        {
            let _import_guard = self.operational_history.lock_legacy_import()?;
            if !self
                .operational_history
                .legacy_fallback_disabled(session.id())?
            {
                let legacy_entries = match self.history.load(session) {
                    Ok(entries) => entries,
                    Err(error) if self.operational_history.has_session_events(session.id())? => {
                        crate::logging::warn_with_fields(
                            "daemon.history",
                            "legacy transcript migration deferred; serving operational history",
                            serde_json::json!({
                                "session_id": session.id(),
                                "error": error.to_string(),
                            }),
                        );
                        return self
                            .operational_history
                            .load_session_history_entries(session.id(), agent_id);
                    }
                    Err(error) => return Err(error),
                };
                let imported = self
                    .operational_history
                    .append_missing_legacy_transcripts(&legacy_entries)?;
                self.operational_history
                    .mark_legacy_fallback_disabled(session.id())?;
                if !imported.is_empty() {
                    crate::logging::info_with_fields(
                        "daemon.history",
                        "migrated legacy session transcript into operational history",
                        serde_json::json!({
                            "session_id": session.id(),
                            "imported_event_count": imported.len(),
                        }),
                    );
                }
            }
        }
        self.operational_history
            .load_session_history_entries(session.id(), agent_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::HistoryEventTurnContext;
    use crate::session::CreateSessionRequest;

    #[test]
    fn history_read_imports_legacy_transcript_once_then_uses_operational_history_only() {
        let config = crate::config::DaemonConfig::for_tests();
        let app = DaemonApp::bootstrap(config).expect("daemon should bootstrap");
        let session = app
            .sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should create");
        let operational_entry =
            SessionHistoryEntry::user_prompt(session.id(), "attachment-1", "agent-1", "current");
        app.operational_history
            .append_transcript(&operational_entry, HistoryEventTurnContext::default())
            .expect("operational history should append");
        let legacy_entry =
            SessionHistoryEntry::user_prompt(session.id(), "attachment-1", "agent-1", "legacy");
        app.history
            .append(&session, &legacy_entry)
            .expect("legacy history should append");

        assert_eq!(
            app.load_session_history_entries(&session, Some("agent-1"))
                .expect("history should load"),
            vec![operational_entry.clone(), legacy_entry.clone()]
        );
        assert!(app
            .operational_history
            .legacy_fallback_disabled(session.id())
            .expect("migration marker should load"));

        let late_legacy_entry =
            SessionHistoryEntry::user_prompt(session.id(), "attachment-1", "agent-1", "late");
        app.history
            .append(&session, &late_legacy_entry)
            .expect("late legacy history should append");
        assert_eq!(
            app.load_session_history_entries(&session, Some("agent-1"))
                .expect("history should load after migration"),
            vec![operational_entry, legacy_entry],
            "legacy JSONL must not remain a live fallback after its one-time import"
        );
    }
}
