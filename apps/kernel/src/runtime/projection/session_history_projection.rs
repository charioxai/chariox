use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::history::SessionHistoryEntry;
use crate::session_history_page::{
    paginate_session_history, paginate_session_history_from_index, SessionHistoryPage,
};

const SESSION_HISTORY_PROJECTION_ENTRY_LIMIT: usize = 1_000;

#[derive(Clone, Default)]
pub(crate) struct SessionHistoryProjectionStore {
    entries: Arc<StdMutex<HashMap<String, SessionHistoryProjection>>>,
}

#[derive(Clone, Debug, Default)]
struct SessionHistoryProjection {
    base_entry_index: usize,
    complete: bool,
    entries: Vec<SessionHistoryEntry>,
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
        let projection = self
            .entries
            .lock()
            .expect("session history projection lock should not be poisoned")
            .get(session_id)
            .cloned()?;
        if agent_id.is_some() && !projection.complete {
            return None;
        }
        if !projection.covers_request(round_count, before_entry_index) {
            return None;
        }
        if let Some(agent_id) = agent_id {
            return Some(page_history_entries(
                projection.entries,
                Some(agent_id),
                round_count,
                max_chars,
                before_entry_index,
                before_entry_char_offset,
            ));
        }
        Some(paginate_session_history_from_index(
            &projection.entries,
            projection.base_entry_index,
            round_count,
            max_chars,
            before_entry_index,
            before_entry_char_offset,
        ))
    }

    pub(crate) fn update_entries(&self, session_id: &str, entries: Vec<SessionHistoryEntry>) {
        let projection = SessionHistoryProjection::from_entries(entries);
        self.entries
            .lock()
            .expect("session history projection lock should not be poisoned")
            .insert(session_id.to_string(), projection);
    }

    pub(crate) fn append(&self, entry: SessionHistoryEntry) {
        let mut entries_by_session = self
            .entries
            .lock()
            .expect("session history projection lock should not be poisoned");
        if let Some(projection) = entries_by_session.get_mut(&entry.session_id) {
            projection.push(entry);
        }
    }

    pub(crate) fn remove(&self, session_id: &str) {
        self.entries
            .lock()
            .expect("session history projection lock should not be poisoned")
            .remove(session_id);
    }
}

impl SessionHistoryProjection {
    fn from_entries(entries: Vec<SessionHistoryEntry>) -> Self {
        let original_len = entries.len();
        let complete = original_len <= SESSION_HISTORY_PROJECTION_ENTRY_LIMIT;
        if complete {
            return Self {
                base_entry_index: 0,
                complete,
                entries,
            };
        }
        let base_entry_index = original_len - SESSION_HISTORY_PROJECTION_ENTRY_LIMIT;
        Self {
            base_entry_index,
            complete,
            entries: entries
                .into_iter()
                .skip(base_entry_index)
                .collect::<Vec<_>>(),
        }
    }

    fn push(&mut self, entry: SessionHistoryEntry) {
        self.entries.push(entry);
        if self.entries.len() > SESSION_HISTORY_PROJECTION_ENTRY_LIMIT {
            let overflow = self.entries.len() - SESSION_HISTORY_PROJECTION_ENTRY_LIMIT;
            self.entries.drain(0..overflow);
            self.base_entry_index = self.base_entry_index.saturating_add(overflow);
            self.complete = false;
        }
    }

    fn covers_request(
        &self,
        round_count: Option<usize>,
        before_entry_index: Option<usize>,
    ) -> bool {
        if self.complete {
            return true;
        }
        if before_entry_index.is_some_and(|index| index <= self.base_entry_index) {
            return false;
        }
        let requested_rounds = round_count.unwrap_or(1);
        if requested_rounds == 0 {
            return false;
        }
        self.entries
            .iter()
            .filter(|entry| entry.kind == crate::history::SessionHistoryEntryKind::UserPrompt)
            .count()
            >= requested_rounds
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};

    #[test]
    fn projection_bounds_large_updates_and_preserves_recent_entry_indices() {
        let store = SessionHistoryProjectionStore::default();
        let mut entries = (0..=SESSION_HISTORY_PROJECTION_ENTRY_LIMIT)
            .map(|index| history_entry(SessionHistoryEntryKind::ProviderOutput, index))
            .collect::<Vec<_>>();
        let prompt_index = SESSION_HISTORY_PROJECTION_ENTRY_LIMIT - 1;
        entries[prompt_index] = history_entry(SessionHistoryEntryKind::UserPrompt, prompt_index);
        store.update_entries("session-1", entries);

        let page = store
            .page("session-1", None, Some(1), None, None, None)
            .expect("recent retained history should be pageable");

        assert_eq!(page.entries[0].entry_index, prompt_index);
        assert_eq!(
            page.entries[0].entry.kind,
            SessionHistoryEntryKind::UserPrompt
        );
        assert_eq!(
            page.entries.last().map(|entry| entry.entry_index),
            Some(SESSION_HISTORY_PROJECTION_ENTRY_LIMIT)
        );
    }

    #[test]
    fn projection_declines_cursor_before_retained_window() {
        let store = SessionHistoryProjectionStore::default();
        let entries = (0..=SESSION_HISTORY_PROJECTION_ENTRY_LIMIT)
            .map(|index| history_entry(SessionHistoryEntryKind::ProviderOutput, index))
            .collect::<Vec<_>>();
        store.update_entries("session-1", entries);

        assert!(store
            .page("session-1", None, Some(1), None, Some(1), None)
            .is_none());
    }

    #[test]
    fn projection_declines_agent_filter_when_truncated() {
        let store = SessionHistoryProjectionStore::default();
        let entries = (0..=SESSION_HISTORY_PROJECTION_ENTRY_LIMIT)
            .map(|index| history_entry(SessionHistoryEntryKind::ProviderOutput, index))
            .collect::<Vec<_>>();
        store.update_entries("session-1", entries);

        assert!(store
            .page("session-1", Some("agent-1"), Some(1), None, None, None)
            .is_none());
    }

    fn history_entry(kind: SessionHistoryEntryKind, index: usize) -> SessionHistoryEntry {
        SessionHistoryEntry {
            session_id: "session-1".to_string(),
            provider_run_id: Some("provider-run-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            source_attachment_id: Some("attachment-1".to_string()),
            kind,
            merge_key: None,
            text: format!("entry {index}"),
            timestamp_ms: index as u64,
        }
    }
}
