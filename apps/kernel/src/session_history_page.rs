use serde::{Deserialize, Serialize};

use crate::history::SessionHistoryEntry;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryPageEntry {
    pub entry_index: usize,
    pub fragment_start: usize,
    pub fragment_end: usize,
    pub total_chars: usize,
    pub entry: SessionHistoryEntry,
}
