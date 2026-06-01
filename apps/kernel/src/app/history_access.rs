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
        let operational_entries = self
            .operational_history
            .load_session_history_entries(session.id(), agent_id)?;
        if !operational_entries.is_empty() {
            return Ok(operational_entries);
        }
        if self.operational_history.has_session_events(session.id())?
            || self
                .operational_history
                .legacy_fallback_disabled(session.id())?
        {
            return Ok(Vec::new());
        }
        let legacy_entries = self.history.load(session)?;
        Ok(match agent_id {
            Some(agent_id) => legacy_entries
                .into_iter()
                .filter(|entry| entry.agent_id.as_deref() == Some(agent_id))
                .collect(),
            None => legacy_entries,
        })
    }
}
