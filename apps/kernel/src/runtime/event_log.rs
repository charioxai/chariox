use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const DEFAULT_EVENT_ID_RESERVATION_BLOCK: u64 = 100_000;

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
    event_ids: EventIdAllocator,
    retention_limit: usize,
    streams: Mutex<BTreeMap<String, EventStream<E>>>,
}

impl<E: Clone> EventLog<E> {
    pub fn new(retention_limit: usize) -> Self {
        Self {
            event_ids: EventIdAllocator::memory(1),
            retention_limit,
            streams: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn new_with_persistent_event_ids(
        retention_limit: usize,
        path: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        Ok(Self {
            event_ids: EventIdAllocator::persistent(
                path.into(),
                DEFAULT_EVENT_ID_RESERVATION_BLOCK,
            )?,
            retention_limit,
            streams: Mutex::new(BTreeMap::new()),
        })
    }

    pub async fn append(
        &self,
        stream_id: impl Into<String>,
        event: E,
    ) -> io::Result<LoggedEvent<E>> {
        let stream_id = stream_id.into();
        let event_id = self.event_ids.next()?;
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
        Ok(logged)
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

#[derive(Debug)]
struct EventIdAllocator {
    next_event_id: AtomicU64,
    reserved_until: AtomicU64,
    persistent: Option<PersistentEventIdReservation>,
}

impl EventIdAllocator {
    fn memory(next_event_id: u64) -> Self {
        Self {
            next_event_id: AtomicU64::new(next_event_id),
            reserved_until: AtomicU64::new(u64::MAX),
            persistent: None,
        }
    }

    fn persistent(path: PathBuf, block_size: u64) -> io::Result<Self> {
        let reservation = PersistentEventIdReservation::new(path, block_size);
        let reserved_until = reservation.reserve_after(0)?;
        Ok(Self {
            next_event_id: AtomicU64::new(reserved_until - block_size + 1),
            reserved_until: AtomicU64::new(reserved_until),
            persistent: Some(reservation),
        })
    }

    fn next(&self) -> io::Result<u64> {
        loop {
            let event_id = self.next_event_id.fetch_add(1, Ordering::Relaxed);
            if event_id <= self.reserved_until.load(Ordering::Acquire) {
                return Ok(event_id);
            }
            let Some(persistent) = &self.persistent else {
                return Ok(event_id);
            };
            let _guard = persistent
                .reservation_lock
                .lock()
                .map_err(|_| io::Error::other("event id reservation lock was poisoned"))?;
            if event_id <= self.reserved_until.load(Ordering::Acquire) {
                return Ok(event_id);
            }
            let reserved_until = persistent.reserve_after(event_id.saturating_sub(1))?;
            self.reserved_until.store(reserved_until, Ordering::Release);
        }
    }
}

#[derive(Debug)]
struct PersistentEventIdReservation {
    path: PathBuf,
    block_size: u64,
    reservation_lock: Arc<StdMutex<()>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PersistentEventIdCounter {
    high_water_event_id: u64,
}

impl PersistentEventIdReservation {
    fn new(path: PathBuf, block_size: u64) -> Self {
        Self {
            reservation_lock: persistent_event_id_lock_for_path(&path),
            path,
            block_size: block_size.max(1),
        }
    }

    fn reserve_after(&self, minimum_previous_event_id: u64) -> io::Result<u64> {
        let current = read_counter(&self.path)?.unwrap_or(0);
        let baseline = current.max(minimum_previous_event_id);
        let next = baseline
            .checked_add(self.block_size)
            .ok_or_else(|| io::Error::other("event id reservation overflow"))?;
        write_counter(&self.path, next)?;
        Ok(next)
    }
}

fn persistent_event_id_lock_for_path(path: &Path) -> Arc<StdMutex<()>> {
    static LOCKS: OnceLock<StdMutex<BTreeMap<PathBuf, Arc<StdMutex<()>>>>> = OnceLock::new();

    let locks = LOCKS.get_or_init(|| StdMutex::new(BTreeMap::new()));
    let mut guard = locks
        .lock()
        .expect("persistent event id lock map should not be poisoned");
    Arc::clone(
        guard
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(StdMutex::new(()))),
    )
}

fn read_counter(path: &Path) -> io::Result<Option<u64>> {
    match fs::read_to_string(path) {
        Ok(payload) => {
            let counter: PersistentEventIdCounter = serde_json::from_str(&payload)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            Ok(Some(counter.high_water_event_id))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn write_counter(path: &Path, high_water_event_id: u64) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let payload = serde_json::to_vec(&PersistentEventIdCounter {
        high_water_event_id,
    })
    .map_err(io::Error::other)?;
    fs::write(&tmp_path, payload)?;
    fs::rename(tmp_path, path)
}

#[cfg(test)]
mod tests {
    use super::{EventLog, ReplayOutcome, DEFAULT_EVENT_ID_RESERVATION_BLOCK};

    #[tokio::test]
    async fn appends_monotonic_event_and_stream_sequences() {
        let log = EventLog::new(16);

        let first = log.append("session:a", "first").await.unwrap();
        let second = log.append("session:a", "second").await.unwrap();
        let other = log.append("session:b", "other").await.unwrap();

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
        let first = log.append("session:a", "first").await.unwrap();
        let second = log.append("session:a", "second").await.unwrap();
        let third = log.append("session:a", "third").await.unwrap();

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
        let first = log.append("session:a", "first").await.unwrap();
        let second = log.append("session:a", "second").await.unwrap();
        let third = log.append("session:a", "third").await.unwrap();

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

    #[tokio::test]
    async fn persistent_event_ids_resume_above_previous_high_water() {
        let root = std::env::temp_dir().join(format!(
            "arroba-event-log-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        let path = root.join("kernel.json");
        let first_log = EventLog::new_with_persistent_event_ids(16, &path)
            .expect("first persistent event log should initialize");
        let first = first_log
            .append("session:a", "first")
            .await
            .expect("first event should append");
        assert_eq!(first.event_id, 1);

        let restarted_log = EventLog::new_with_persistent_event_ids(16, &path)
            .expect("restarted persistent event log should initialize");
        let restarted = restarted_log
            .append("session:a", "restarted")
            .await
            .expect("restarted event should append");

        assert!(
            restarted.event_id > DEFAULT_EVENT_ID_RESERVATION_BLOCK,
            "restarted kernel must not emit event ids below the previous browser cursor"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
