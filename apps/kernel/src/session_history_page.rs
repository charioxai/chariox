use serde::{Deserialize, Serialize};

use crate::history::SessionHistoryEntry;
mod compact;
pub(crate) use compact::coalesce_history_page_entries;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryPageEntry {
    pub entry_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence_end: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence_byte_lengths: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence_indices: Vec<usize>,
    pub fragment_start: usize,
    pub fragment_end: usize,
    pub total_chars: usize,
    pub entry: SessionHistoryEntry,
}
