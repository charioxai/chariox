//! Events of an agent's sequence range, kept between reads so that a transcript outline read of a
//! growing reply loads only the rows written since the previous read.
//!
//! Rows are appended with a growing SQLite `rowid` whatever their `sequence` (producers reserve a
//! sequence before their write commits), so a reader that remembers the highest `rowid` it saw
//! misses nothing. Only a rewrite or a delete changes rows already seen; those bump the epoch and
//! the next read loads the range again.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use super::HistoryEvent;

const MAX_ENTRIES: usize = 32;
const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct RangeCache {
    epoch: AtomicU64,
    entries: Mutex<VecDeque<Entry>>,
    #[cfg(test)]
    rows_decoded: AtomicU64,
}

#[derive(Debug)]
struct Entry {
    key: RangeKey,
    epoch: u64,
    /// The highest `rowid` among the events: rows written later have a higher one.
    frontier: i64,
    bytes: usize,
    events: Vec<HistoryEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RangeKey {
    pub session_id: String,
    pub agent_id: String,
    pub sequence_start: u64,
    pub sequence_end: u64,
}

/// A range taken out of the cache for one read: the events seen so far and where to continue.
pub(super) struct Cached {
    epoch: u64,
    pub frontier: Option<i64>,
    bytes: usize,
    events: Vec<HistoryEvent>,
}

impl RangeCache {
    /// A row that was already read changed or went: forget every range.
    pub(super) fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }

    pub(super) fn take(&self, key: &RangeKey) -> Cached {
        let epoch = self.epoch.load(Ordering::Acquire);
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let found = entries
            .iter()
            .position(|entry| entry.key == *key && entry.epoch == epoch)
            .and_then(|index| entries.remove(index));
        match found {
            Some(entry) => Cached {
                epoch,
                frontier: Some(entry.frontier),
                bytes: entry.bytes,
                events: entry.events,
            },
            None => Cached {
                epoch,
                frontier: None,
                bytes: 0,
                events: Vec::new(),
            },
        }
    }

    #[cfg(test)]
    pub(super) fn count_decoded(&self, count: usize) {
        self.rows_decoded.fetch_add(count as u64, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(super) fn rows_decoded(&self) -> u64 {
        self.rows_decoded.load(Ordering::Relaxed)
    }

    /// Keeps the range with its newly read rows, unless a rewrite or delete happened meanwhile.
    pub(super) fn put(
        &self,
        key: RangeKey,
        mut cached: Cached,
        found: Vec<(i64, usize, HistoryEvent)>,
    ) -> Vec<HistoryEvent> {
        for (rowid, size, event) in found {
            cached.frontier = Some(cached.frontier.map_or(rowid, |seen| seen.max(rowid)));
            cached.bytes += size;
            // Producers reserve sequences before they write, so a row can arrive behind a later one.
            let at = cached
                .events
                .partition_point(|seen| seen.sequence <= event.sequence);
            cached.events.insert(at, event);
        }
        let events = cached.events.clone();
        let Some(frontier) = cached.frontier else {
            return events;
        };
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if cached.epoch != self.epoch.load(Ordering::Acquire) || cached.bytes > MAX_TOTAL_BYTES / 4
        {
            return events;
        }
        entries.retain(|entry| entry.key != key);
        let mut total = cached.bytes + entries.iter().map(|entry| entry.bytes).sum::<usize>();
        entries.push_back(Entry {
            key,
            epoch: cached.epoch,
            frontier,
            bytes: cached.bytes,
            events: cached.events,
        });
        while entries.len() > MAX_ENTRIES || total > MAX_TOTAL_BYTES {
            let Some(oldest) = entries.pop_front() else {
                break;
            };
            total -= oldest.bytes;
        }
        events
    }
}
