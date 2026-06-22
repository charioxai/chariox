use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::history::SessionHistoryEntry;

const SESSION_HISTORY_PROJECTION_ENTRY_LIMIT: usize = 1_000;

#[derive(Clone, Default)]
pub(crate) struct SessionHistoryProjectionStore {
    entries: Arc<StdMutex<HashMap<String, SessionHistoryProjection>>>,
}

#[derive(Clone, Debug, Default)]
struct SessionHistoryProjection {
    entries: Vec<SessionHistoryEntry>,
}

impl SessionHistoryProjectionStore {
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

    pub(crate) fn replace_by_merge_key_or_append(&self, entry: SessionHistoryEntry) {
        let mut entries_by_session = self
            .entries
            .lock()
            .expect("session history projection lock should not be poisoned");
        if let Some(projection) = entries_by_session.get_mut(&entry.session_id) {
            if projection.replace_by_merge_key(&entry) {
                return;
            }
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
        if original_len <= SESSION_HISTORY_PROJECTION_ENTRY_LIMIT {
            return Self { entries };
        }
        let base_entry_index = original_len - SESSION_HISTORY_PROJECTION_ENTRY_LIMIT;
        Self {
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
        }
    }

    fn replace_by_merge_key(&mut self, entry: &SessionHistoryEntry) -> bool {
        let Some(merge_key) = entry.merge_key.as_deref() else {
            return false;
        };
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|existing| existing.merge_key.as_deref() == Some(merge_key))
        {
            *existing = entry.clone();
            return true;
        }
        false
    }
}
