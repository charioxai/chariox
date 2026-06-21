use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const DEFAULT_EVENT_ID_RESERVATION_BLOCK: u64 = 100_000;
pub const DEFAULT_PERSISTENT_EVENT_MAX_BYTES: u64 = 50 * 1024 * 1024;
pub const DEFAULT_PERSISTENT_EVENT_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
const PERSISTENT_COMPACTION_SKIP_LIMIT: u64 = 1_024;
const PERSISTENT_COMPACTION_FILE_GROWTH_MULTIPLIER: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedEvent<E> {
    pub event_id: u64,
    pub stream_id: String,
    pub stream_seq: u64,
    #[serde(default)]
    pub recorded_at_ms: u64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventRetentionPolicy {
    pub max_stream_events: usize,
    pub max_total_bytes: Option<u64>,
    pub max_age_ms: Option<u64>,
}

impl EventRetentionPolicy {
    pub fn memory(max_stream_events: usize) -> Self {
        Self {
            max_stream_events,
            max_total_bytes: None,
            max_age_ms: None,
        }
    }

    pub fn persistent(max_stream_events: usize) -> Self {
        Self {
            max_stream_events,
            max_total_bytes: Some(DEFAULT_PERSISTENT_EVENT_MAX_BYTES),
            max_age_ms: Some(DEFAULT_PERSISTENT_EVENT_MAX_AGE_MS),
        }
    }
}

#[derive(Debug)]
struct EventStream<E> {
    next_stream_seq: u64,
    retained: VecDeque<LoggedEvent<E>>,
    retained_jsonl_bytes: u64,
    latest_event_id: Option<u64>,
}

impl<E> Default for EventStream<E> {
    fn default() -> Self {
        Self {
            next_stream_seq: 1,
            retained: VecDeque::new(),
            retained_jsonl_bytes: 0,
            latest_event_id: None,
        }
    }
}

#[derive(Debug)]
pub struct EventLog<E> {
    event_ids: EventIdAllocator,
    retention: EventRetentionPolicy,
    streams: Mutex<BTreeMap<String, EventStream<E>>>,
    persistence: Option<PersistentEventStore>,
}

impl<E: Clone + Serialize> EventLog<E> {
    pub fn new(retention_limit: usize) -> Self {
        Self {
            event_ids: EventIdAllocator::memory(1),
            retention: EventRetentionPolicy::memory(retention_limit),
            streams: Mutex::new(BTreeMap::new()),
            persistence: None,
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
            retention: EventRetentionPolicy::memory(retention_limit),
            streams: Mutex::new(BTreeMap::new()),
            persistence: None,
        })
    }

    pub async fn append(
        &self,
        stream_id: impl Into<String>,
        event: E,
    ) -> io::Result<LoggedEvent<E>> {
        let stream_id = stream_id.into();
        let event_id = self.event_ids.next()?;
        let (logged, compact_snapshot) = {
            let mut streams = self.streams.lock().await;
            let stream = streams.entry(stream_id.clone()).or_default();
            let logged = LoggedEvent {
                event_id,
                stream_id,
                stream_seq: stream.next_stream_seq,
                recorded_at_ms: unix_epoch_ms(),
                event,
            };
            let logged_jsonl_bytes = logged_event_jsonl_bytes(&logged)?;
            stream.next_stream_seq += 1;
            stream.latest_event_id = Some(event_id);
            stream.retained.push_back(logged.clone());
            stream.retained_jsonl_bytes = stream
                .retained_jsonl_bytes
                .saturating_add(logged_jsonl_bytes);
            let compact_after_append =
                apply_retention(&mut streams, self.retention, unix_epoch_ms());
            let compact_snapshot = if compact_after_append {
                match &self.persistence {
                    Some(persistence) if persistence.should_compact_now()? => {
                        Some(retained_events_snapshot(&streams))
                    }
                    _ => None,
                }
            } else {
                None
            };
            (logged, compact_snapshot)
        };
        if let Some(persistence) = &self.persistence {
            persistence
                .persist_event(&logged, compact_snapshot.as_deref())
                .await?;
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

        if stream.retained.is_empty()
            && stream
                .latest_event_id
                .is_some_and(|latest| cursor_event_id < latest)
        {
            return ReplayOutcome::Gap(ReplayGap {
                stream_id: stream_id.to_string(),
                requested_from_event_id: cursor_event_id,
                first_retained_event_id: None,
                latest_event_id: stream.latest_event_id,
            });
        }

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

impl<E> EventLog<E>
where
    E: Clone + Serialize + DeserializeOwned,
{
    pub fn new_with_persistent_event_store(
        retention_limit: usize,
        event_counter_path: impl Into<PathBuf>,
        event_store_path: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        Self::new_with_persistent_event_store_and_retention(
            EventRetentionPolicy::persistent(retention_limit),
            event_counter_path,
            event_store_path,
        )
    }

    pub fn new_with_persistent_event_store_and_retention(
        retention: EventRetentionPolicy,
        event_counter_path: impl Into<PathBuf>,
        event_store_path: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        let event_counter_path = event_counter_path.into();
        let event_store_path = event_store_path.into();
        let (streams, compact_after_load) = load_retained_streams(&event_store_path, retention)?;
        if compact_after_load {
            let snapshot = retained_events_snapshot(&streams);
            rewrite_logged_events(&event_store_path, &snapshot)?;
        }
        Ok(Self {
            event_ids: EventIdAllocator::persistent(
                event_counter_path,
                DEFAULT_EVENT_ID_RESERVATION_BLOCK,
            )?,
            retention,
            streams: Mutex::new(streams),
            persistence: Some(PersistentEventStore {
                path: event_store_path,
                io_lock: Mutex::new(()),
                skipped_compactions: AtomicU64::new(0),
                max_file_bytes_before_compaction: retention.max_total_bytes.map(|bytes| {
                    bytes.saturating_mul(PERSISTENT_COMPACTION_FILE_GROWTH_MULTIPLIER)
                }),
            }),
        })
    }
}

#[derive(Debug)]
struct PersistentEventStore {
    path: PathBuf,
    io_lock: Mutex<()>,
    skipped_compactions: AtomicU64,
    max_file_bytes_before_compaction: Option<u64>,
}

impl PersistentEventStore {
    async fn persist_event<E>(
        &self,
        logged: &LoggedEvent<E>,
        compact_snapshot: Option<&[LoggedEvent<E>]>,
    ) -> io::Result<()>
    where
        E: Clone + Serialize,
    {
        let _guard = self.io_lock.lock().await;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        append_logged_event(&self.path, logged)?;
        if let Some(snapshot) = compact_snapshot {
            rewrite_logged_events(&self.path, snapshot)?;
            self.skipped_compactions.store(0, Ordering::Release);
        }
        Ok(())
    }

    fn should_compact_now(&self) -> io::Result<bool> {
        let skipped = self.skipped_compactions.fetch_add(1, Ordering::AcqRel) + 1;
        if skipped >= PERSISTENT_COMPACTION_SKIP_LIMIT {
            return Ok(true);
        }
        if let Some(max_file_bytes) = self.max_file_bytes_before_compaction {
            let file_bytes = match fs::metadata(&self.path) {
                Ok(metadata) => metadata.len(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
                Err(error) => return Err(error),
            };
            return Ok(file_bytes > max_file_bytes);
        }
        Ok(false)
    }
}

fn load_retained_streams<E>(
    path: &Path,
    retention: EventRetentionPolicy,
) -> io::Result<(BTreeMap<String, EventStream<E>>, bool)>
where
    E: Clone + DeserializeOwned + Serialize,
{
    let payload = match fs::read_to_string(path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((BTreeMap::new(), false))
        }
        Err(error) => return Err(error),
    };
    let mut streams = BTreeMap::<String, EventStream<E>>::new();
    for line in payload.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(logged) = serde_json::from_str::<LoggedEvent<E>>(line) else {
            continue;
        };
        let stream = streams.entry(logged.stream_id.clone()).or_default();
        stream.next_stream_seq = stream
            .next_stream_seq
            .max(logged.stream_seq.saturating_add(1));
        stream.latest_event_id = Some(stream.latest_event_id.unwrap_or(0).max(logged.event_id));
        let logged_jsonl_bytes = logged_event_jsonl_bytes(&logged)?;
        stream.retained_jsonl_bytes = stream
            .retained_jsonl_bytes
            .saturating_add(logged_jsonl_bytes);
        stream.retained.push_back(logged);
    }
    let compacted = apply_retention(&mut streams, retention, unix_epoch_ms());
    Ok((streams, compacted))
}

fn apply_retention<E>(
    streams: &mut BTreeMap<String, EventStream<E>>,
    retention: EventRetentionPolicy,
    now_ms: u64,
) -> bool
where
    E: Serialize,
{
    let mut compacted = false;
    for stream in streams.values_mut() {
        while stream.retained.len() > retention.max_stream_events {
            compacted |= pop_front_retained_event(stream);
        }
        if let Some(max_age_ms) = retention.max_age_ms {
            while stream.retained.front().is_some_and(|event| {
                event.recorded_at_ms != 0
                    && now_ms.saturating_sub(event.recorded_at_ms) > max_age_ms
            }) {
                compacted |= pop_front_retained_event(stream);
            }
        }
    }

    if let Some(max_total_bytes) = retention.max_total_bytes {
        while retained_events_jsonl_bytes(streams) > max_total_bytes {
            if !remove_oldest_retained_event(streams) {
                break;
            }
            compacted = true;
        }
    }

    compacted
}

fn remove_oldest_retained_event<E>(streams: &mut BTreeMap<String, EventStream<E>>) -> bool
where
    E: Serialize,
{
    let oldest_stream_id = streams
        .iter()
        .filter_map(|(stream_id, stream)| {
            stream
                .retained
                .front()
                .map(|event| (stream_id.clone(), event.event_id))
        })
        .min_by_key(|(_, event_id)| *event_id)
        .map(|(stream_id, _)| stream_id);
    let Some(stream_id) = oldest_stream_id else {
        return false;
    };
    streams
        .get_mut(&stream_id)
        .is_some_and(pop_front_retained_event)
}

fn pop_front_retained_event<E>(stream: &mut EventStream<E>) -> bool
where
    E: Serialize,
{
    let Some(event) = stream.retained.pop_front() else {
        return false;
    };
    let event_bytes = logged_event_jsonl_bytes(&event).unwrap_or(0);
    stream.retained_jsonl_bytes = stream.retained_jsonl_bytes.saturating_sub(event_bytes);
    true
}

fn retained_events_snapshot<E: Clone>(
    streams: &BTreeMap<String, EventStream<E>>,
) -> Vec<LoggedEvent<E>> {
    let mut events = streams
        .values()
        .flat_map(|stream| stream.retained.iter().cloned())
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.event_id);
    events
}

fn retained_events_jsonl_bytes<E>(streams: &BTreeMap<String, EventStream<E>>) -> u64 {
    streams
        .values()
        .map(|stream| stream.retained_jsonl_bytes)
        .fold(0_u64, u64::saturating_add)
}

fn logged_event_jsonl_bytes<E>(event: &LoggedEvent<E>) -> io::Result<u64>
where
    E: Serialize,
{
    let bytes = serde_json::to_vec(event).map_err(io::Error::other)?;
    Ok(bytes.len().saturating_add(1) as u64)
}

fn append_logged_event<E>(path: &Path, logged: &LoggedEvent<E>) -> io::Result<()>
where
    E: Serialize,
{
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, logged).map_err(io::Error::other)?;
    file.write_all(b"\n")
}

fn rewrite_logged_events<E>(path: &Path, events: &[LoggedEvent<E>]) -> io::Result<()>
where
    E: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("jsonl.tmp");
    let mut file = fs::File::create(&tmp_path)?;
    for event in events {
        serde_json::to_writer(&mut file, event).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
    }
    fs::rename(tmp_path, path)
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

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde::{Serialize, Serializer};

    use super::{
        EventLog, EventRetentionPolicy, LoggedEvent, ReplayOutcome,
        DEFAULT_EVENT_ID_RESERVATION_BLOCK,
    };

    static CLONE_COUNT: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    struct CloneCountedEvent(&'static str);

    impl Clone for CloneCountedEvent {
        fn clone(&self) -> Self {
            CLONE_COUNT.fetch_add(1, Ordering::SeqCst);
            Self(self.0)
        }
    }

    impl Serialize for CloneCountedEvent {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(self.0)
        }
    }

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
    async fn retention_does_not_clone_snapshot_without_persistent_compaction() {
        CLONE_COUNT.store(0, Ordering::SeqCst);
        let log = EventLog::new(1);
        log.append("session:a", CloneCountedEvent("first"))
            .await
            .expect("first event should append");
        CLONE_COUNT.store(0, Ordering::SeqCst);

        log.append("session:a", CloneCountedEvent("second"))
            .await
            .expect("second event should append");

        assert_eq!(
            CLONE_COUNT.load(Ordering::SeqCst),
            1,
            "retention should clone only the appended event, not a retained snapshot"
        );
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

    #[tokio::test]
    async fn persistent_event_store_replays_after_restart() {
        let root = std::env::temp_dir().join(format!(
            "arroba-event-store-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        let counter_path = root.join("counter.json");
        let events_path = root.join("events.jsonl");
        let first_log =
            EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
                .expect("first persistent event store should initialize");
        let first = first_log
            .append("session:a", "first".to_string())
            .await
            .expect("first event should append");
        let second = first_log
            .append("session:a", "second".to_string())
            .await
            .expect("second event should append");

        let restarted_log =
            EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
                .expect("restarted persistent event store should initialize");
        let replay = restarted_log
            .replay_after("session:a", first.event_id)
            .await;

        match replay {
            ReplayOutcome::Replayed(events) => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].event_id, second.event_id);
                assert_eq!(events[0].event, "second");
            }
            ReplayOutcome::Gap(gap) => panic!("unexpected replay gap: {gap:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn persistent_event_store_skips_malformed_lines() {
        let root = std::env::temp_dir().join(format!(
            "arroba-event-store-malformed-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        let counter_path = root.join("counter.json");
        let events_path = root.join("events.jsonl");
        std::fs::create_dir_all(&root).expect("event store root should create");
        std::fs::write(
            &events_path,
            concat!(
                "not-json\n",
                "{\"event_id\":7,\"stream_id\":\"session:a\",\"stream_seq\":1,\"event\":\"first\"}\n",
                "{\"event_id\":8,\"stream_id\":\"session:a\",\"stream_seq\":2,\"event\":\"second\"}\n"
            ),
        )
        .expect("event store should seed");

        let log =
            EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
                .expect("persistent event store should tolerate malformed lines");
        let replay = log.replay_after("session:a", 7).await;

        match replay {
            ReplayOutcome::Replayed(events) => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].event_id, 8);
                assert_eq!(events[0].event, "second");
            }
            ReplayOutcome::Gap(gap) => panic!("unexpected replay gap: {gap:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn persistent_event_store_compacts_by_total_bytes() {
        let root = std::env::temp_dir().join(format!(
            "arroba-event-store-bytes-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        let counter_path = root.join("counter.json");
        let events_path = root.join("events.jsonl");
        let retention = EventRetentionPolicy {
            max_stream_events: 16,
            max_total_bytes: Some(160),
            max_age_ms: None,
        };
        let log = EventLog::<String>::new_with_persistent_event_store_and_retention(
            retention,
            &counter_path,
            &events_path,
        )
        .expect("persistent event store should initialize");
        let first = log
            .append("session:a", "first".to_string())
            .await
            .expect("first event should append");
        let second = log
            .append("session:a", "second".to_string())
            .await
            .expect("second event should append");

        let replay = log.replay_after("session:a", first.event_id).await;
        match replay {
            ReplayOutcome::Gap(gap) => {
                assert_eq!(gap.first_retained_event_id, Some(second.event_id));
                assert_eq!(gap.latest_event_id, Some(second.event_id));
            }
            ReplayOutcome::Replayed(events) => panic!("expected replay gap, got {events:?}"),
        }
        for index in 0..8 {
            log.append("session:a", format!("extra-{index}"))
                .await
                .expect("extra event should append");
            let stored = std::fs::read_to_string(&events_path).expect("event store should exist");
            if !stored.contains("\"first\"") {
                assert!(
                    stored.len() as u64 <= retention.max_total_bytes.unwrap(),
                    "event store should compact after bounded file growth: {stored}"
                );
                let _ = std::fs::remove_dir_all(root);
                return;
            }
        }
        panic!("event store should compact after bounded file growth");
    }

    #[tokio::test]
    async fn persistent_event_store_compacts_by_event_age_on_load() {
        let root = std::env::temp_dir().join(format!(
            "arroba-event-store-age-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        let counter_path = root.join("counter.json");
        let events_path = root.join("events.jsonl");
        std::fs::create_dir_all(&root).expect("event store root should create");
        let now_ms = super::unix_epoch_ms();
        let old = LoggedEvent {
            event_id: 7,
            stream_id: "session:a".to_string(),
            stream_seq: 1,
            recorded_at_ms: now_ms.saturating_sub(10_000),
            event: "old".to_string(),
        };
        let fresh = LoggedEvent {
            event_id: 8,
            stream_id: "session:a".to_string(),
            stream_seq: 2,
            recorded_at_ms: now_ms,
            event: "fresh".to_string(),
        };
        super::rewrite_logged_events(&events_path, &[old, fresh]).expect("event store should seed");
        let retention = EventRetentionPolicy {
            max_stream_events: 16,
            max_total_bytes: None,
            max_age_ms: Some(1_000),
        };

        let log = EventLog::<String>::new_with_persistent_event_store_and_retention(
            retention,
            &counter_path,
            &events_path,
        )
        .expect("persistent event store should initialize");

        let stored = std::fs::read_to_string(&events_path).expect("event store should exist");
        assert!(!stored.contains("\"old\""));
        assert!(stored.contains("\"fresh\""));

        let replay = log.replay_after("session:a", 7).await;
        match replay {
            ReplayOutcome::Gap(gap) => {
                assert_eq!(gap.first_retained_event_id, Some(8));
                assert_eq!(gap.latest_event_id, Some(8));
            }
            ReplayOutcome::Replayed(events) => panic!("expected replay gap, got {events:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
