use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedEvent<E> {
    pub event_id: u64,
    pub stream_id: String,
    pub stream_seq: u64,
    pub event: E,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayGap {
    pub stream_id: String,
    pub requested_from_event_id: u64,
    pub first_retained_event_id: Option<u64>,
    pub latest_event_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayOutcome<E> {
    Replayed(Vec<LoggedEvent<E>>),
    Gap(ReplayGap),
}

#[derive(Debug)]
struct EventStream<E> {
    next_stream_seq: u64,
    retained: VecDeque<LoggedEvent<E>>,
    latest_event_id: Option<u64>,
}

impl<E> Default for EventStream<E> {
    fn default() -> Self {
        Self {
            next_stream_seq: 1,
            retained: VecDeque::new(),
            latest_event_id: None,
        }
    }
}

#[derive(Debug)]
pub struct EventLog<E> {
    next_event_id: AtomicU64,
    retention_limit: usize,
    streams: Mutex<BTreeMap<String, EventStream<E>>>,
}

impl<E: Clone> EventLog<E> {
    pub fn new(retention_limit: usize) -> Self {
        Self {
            next_event_id: AtomicU64::new(1),
            retention_limit,
            streams: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn append(&self, stream_id: impl Into<String>, event: E) -> LoggedEvent<E> {
        let stream_id = stream_id.into();
        let event_id = self.next_event_id.fetch_add(1, Ordering::Relaxed);
        let mut streams = self.streams.lock().await;
        let stream = streams.entry(stream_id.clone()).or_default();
        let logged = LoggedEvent {
            event_id,
            stream_id,
            stream_seq: stream.next_stream_seq,
            event,
        };
        stream.next_stream_seq += 1;
        stream.latest_event_id = Some(event_id);
        stream.retained.push_back(logged.clone());
        while stream.retained.len() > self.retention_limit {
            stream.retained.pop_front();
        }
        logged
    }

    pub async fn replay_after(&self, stream_id: &str, cursor_event_id: u64) -> ReplayOutcome<E> {
        let streams = self.streams.lock().await;
        let Some(stream) = streams.get(stream_id) else {
            return ReplayOutcome::Gap(ReplayGap {
                stream_id: stream_id.to_string(),
                requested_from_event_id: cursor_event_id,
                first_retained_event_id: None,
                latest_event_id: None,
            });
        };

        if let Some(first_retained) = stream.retained.front() {
            if cursor_event_id < first_retained.event_id {
                return ReplayOutcome::Gap(ReplayGap {
                    stream_id: stream_id.to_string(),
                    requested_from_event_id: cursor_event_id,
                    first_retained_event_id: Some(first_retained.event_id),
                    latest_event_id: stream.latest_event_id,
                });
            }
        }

        ReplayOutcome::Replayed(
            stream
                .retained
                .iter()
                .filter(|event| event.event_id > cursor_event_id)
                .cloned()
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{EventLog, ReplayOutcome};

    #[tokio::test]
    async fn appends_monotonic_event_and_stream_sequences() {
        let log = EventLog::new(16);

        let first = log.append("session:a", "first").await;
        let second = log.append("session:a", "second").await;
        let other = log.append("session:b", "other").await;

        assert_eq!(first.event_id, 1);
        assert_eq!(first.stream_seq, 1);
        assert_eq!(second.event_id, 2);
        assert_eq!(second.stream_seq, 2);
        assert_eq!(other.event_id, 3);
        assert_eq!(other.stream_seq, 1);
    }

    #[tokio::test]
    async fn replays_events_after_retained_cursor() {
        let log = EventLog::new(16);
        let first = log.append("session:a", "first").await;
        let second = log.append("session:a", "second").await;
        let third = log.append("session:a", "third").await;

        let replay = log.replay_after("session:a", first.event_id).await;

        match replay {
            ReplayOutcome::Replayed(events) => {
                assert_eq!(events.len(), 2);
                assert_eq!(events[0].event_id, second.event_id);
                assert_eq!(events[1].event_id, third.event_id);
            }
            ReplayOutcome::Gap(gap) => panic!("unexpected replay gap: {gap:?}"),
        }
    }

    #[tokio::test]
    async fn reports_replay_gap_when_cursor_is_older_than_window() {
        let log = EventLog::new(2);
        let first = log.append("session:a", "first").await;
        let second = log.append("session:a", "second").await;
        let third = log.append("session:a", "third").await;

        let replay = log.replay_after("session:a", first.event_id).await;

        match replay {
            ReplayOutcome::Gap(gap) => {
                assert_eq!(gap.stream_id, "session:a");
                assert_eq!(gap.requested_from_event_id, first.event_id);
                assert_eq!(gap.first_retained_event_id, Some(second.event_id));
                assert_eq!(gap.latest_event_id, Some(third.event_id));
            }
            ReplayOutcome::Replayed(events) => panic!("expected replay gap, got {events:?}"),
        }
    }
}
