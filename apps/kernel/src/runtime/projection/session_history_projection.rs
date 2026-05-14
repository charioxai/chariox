use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::history::SessionHistoryEntry;
use crate::session_history_page::{paginate_session_history, SessionHistoryPage};

#[derive(Clone, Default)]
pub(crate) struct SessionHistoryProjectionStore {
    entries: Arc<StdMutex<HashMap<String, Vec<SessionHistoryEntry>>>>,
}

impl SessionHistoryProjectionStore {
    pub(crate) fn page(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
        round_count: Option<usize>,
        max_chars: Option<usize>,
        before_entry_index: Option<usize>,
        before_entry_char_offset: Option<usize>,
    ) -> Option<SessionHistoryPage> {
        let entries = self
            .entries
            .lock()
            .expect("session history projection lock should not be poisoned")
            .get(session_id)
            .cloned()?;
        Some(page_history_entries(
            entries,
            agent_id,
            round_count,
            max_chars,
            before_entry_index,
            before_entry_char_offset,
        ))
    }

    pub(crate) fn update_entries(&self, session_id: &str, entries: Vec<SessionHistoryEntry>) {
        self.entries
            .lock()
            .expect("session history projection lock should not be poisoned")
            .insert(session_id.to_string(), entries);
    }

    pub(crate) fn append(&self, entry: SessionHistoryEntry) {
        let mut entries_by_session = self
            .entries
            .lock()
            .expect("session history projection lock should not be poisoned");
        if let Some(entries) = entries_by_session.get_mut(&entry.session_id) {
            entries.push(entry);
        }
    }

    pub(crate) fn remove(&self, session_id: &str) {
        self.entries
            .lock()
            .expect("session history projection lock should not be poisoned")
            .remove(session_id);
    }
}

pub(crate) fn page_history_entries(
    mut entries: Vec<SessionHistoryEntry>,
    agent_id: Option<&str>,
    round_count: Option<usize>,
    max_chars: Option<usize>,
    before_entry_index: Option<usize>,
    before_entry_char_offset: Option<usize>,
) -> SessionHistoryPage {
    if let Some(agent_id) = agent_id {
        entries.retain(|entry| {
            entry.agent_id.is_none() || entry.agent_id.as_deref() == Some(agent_id)
        });
    }
    paginate_session_history(
        &entries,
        round_count,
        max_chars,
        before_entry_index,
        before_entry_char_offset,
    )
}
