use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::sync::{mpsc, Mutex};

pub const DEFAULT_EVENT_ID_RESERVATION_BLOCK: u64 = 100_000;
pub const DEFAULT_PERSISTENT_EVENT_MAX_BYTES: u64 = 50 * 1024 * 1024;
pub const DEFAULT_PERSISTENT_EVENT_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
const PERSISTENT_COMPACTION_SKIP_LIMIT: u64 = 1_024;
const PERSISTENT_COMPACTION_FILE_GROWTH_MULTIPLIER: u64 = 1;
const PERSISTENT_COMPACTION_TARGET_NUMERATOR: u64 = 3;
const PERSISTENT_COMPACTION_TARGET_DENOMINATOR: u64 = 4;
const PERSISTENT_EVENT_WRITE_QUEUE_CAPACITY: usize = 32;
const PERSISTENT_WRITE_MAX_ATTEMPTS: usize = 3;
const PERSISTENT_WRITE_RETRY_BASE_DELAY_MS: u64 = 25;
#[cfg(test)]
const PERSISTENT_WRITER_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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
        let mut streams = self.streams.lock().await;
        let stream = streams.entry(stream_id.clone()).or_default();
        let previous_latest_event_id = stream.latest_event_id;
        let logged = LoggedEvent {
            event_id,
            stream_id: stream_id.clone(),
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
        if let Some(persistence) = &self.persistence {
            if let Err(error) = persistence.persist_event(&logged).await {
                let stream = streams
                    .get_mut(&stream_id)
                    .expect("appended event stream should remain present");
                let removed = stream
                    .retained
                    .pop_back()
                    .expect("failed appended event should remain at the back");
                debug_assert_eq!(removed.event_id, event_id);
                stream.next_stream_seq = logged.stream_seq;
                stream.latest_event_id = previous_latest_event_id;
                stream.retained_jsonl_bytes = stream
                    .retained_jsonl_bytes
                    .saturating_sub(logged_jsonl_bytes);
                if previous_latest_event_id.is_none() && stream.retained.is_empty() {
                    streams.remove(&stream_id);
                }
                return Err(error);
            }
        }
        let compact_after_append = apply_retention(&mut streams, self.retention, unix_epoch_ms());
        if compact_after_append {
            if let Some(persistence) = &self.persistence {
                if persistence.should_compact_now() {
                    let compact_snapshot = retained_events_snapshot(&streams);
                    if let Err(error) = persistence.persist_compaction(&compact_snapshot).await {
                        crate::logging::warn_with_fields(
                            "daemon.event_log",
                            "persistent event log compaction failed",
                            serde_json::json!({
                                "path": persistence.path.display().to_string(),
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
            }
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

    #[cfg(test)]
    async fn flush_persistence_for_tests(&self) -> io::Result<()> {
        match &self.persistence {
            Some(persistence) => persistence.flush_for_tests().await,
            None => Ok(()),
        }
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
        truncate_torn_jsonl_tail(&event_store_path)?;
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
            persistence: Some(PersistentEventStore::new(event_store_path, retention)?),
        })
    }
}

#[derive(Debug)]
struct PersistentEventStore {
    path: PathBuf,
    write_tx: mpsc::Sender<PersistentEventWrite>,
    skipped_compactions: AtomicU64,
    max_file_bytes_before_compaction: Option<u64>,
    estimated_file_bytes: AtomicU64,
}

#[derive(Debug)]
enum PersistentEventWrite {
    Append {
        append_jsonl: Vec<u8>,
        reply_tx: oneshot::Sender<io::Result<()>>,
    },
    Compact {
        compact_jsonl: Vec<u8>,
        reply_tx: oneshot::Sender<io::Result<()>>,
    },
    #[cfg(test)]
    Flush(oneshot::Sender<io::Result<()>>),
}

impl PersistentEventStore {
    fn new(path: PathBuf, retention: EventRetentionPolicy) -> io::Result<Self> {
        let estimated_file_bytes = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        let (write_tx, write_rx) = mpsc::channel(PERSISTENT_EVENT_WRITE_QUEUE_CAPACITY);
        spawn_persistent_event_writer(path.clone(), write_rx)?;
        Ok(Self {
            path,
            write_tx,
            skipped_compactions: AtomicU64::new(0),
            max_file_bytes_before_compaction: retention
                .max_total_bytes
                .map(|bytes| bytes.saturating_mul(PERSISTENT_COMPACTION_FILE_GROWTH_MULTIPLIER)),
            estimated_file_bytes: AtomicU64::new(estimated_file_bytes),
        })
    }

    async fn persist_event<E>(&self, logged: &LoggedEvent<E>) -> io::Result<()>
    where
        E: Clone + Serialize,
    {
        let append_jsonl = logged_event_jsonl_payload(logged)?;
        let append_jsonl_len = append_jsonl.len() as u64;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.write_tx
            .send(PersistentEventWrite::Append {
                append_jsonl,
                reply_tx,
            })
            .await
            .map_err(|_| io::Error::other("persistent event writer stopped"))?;
        reply_rx
            .await
            .map_err(|_| io::Error::other("persistent event writer stopped before append"))??;
        self.estimated_file_bytes
            .fetch_add(append_jsonl_len, Ordering::AcqRel);
        Ok(())
    }

    async fn persist_compaction<E>(&self, compact_snapshot: &[LoggedEvent<E>]) -> io::Result<()>
    where
        E: Serialize,
    {
        let compact_jsonl = logged_events_jsonl_payload(compact_snapshot)?;
        let compact_jsonl_len = compact_jsonl.len() as u64;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.write_tx
            .send(PersistentEventWrite::Compact {
                compact_jsonl,
                reply_tx,
            })
            .await
            .map_err(|_| io::Error::other("persistent event writer stopped"))?;
        reply_rx
            .await
            .map_err(|_| io::Error::other("persistent event writer stopped before compaction"))??;
        self.estimated_file_bytes
            .store(compact_jsonl_len, Ordering::Release);
        self.skipped_compactions.store(0, Ordering::Release);
        Ok(())
    }

    fn should_compact_now(&self) -> bool {
        let skipped = self.skipped_compactions.fetch_add(1, Ordering::AcqRel) + 1;
        if skipped >= PERSISTENT_COMPACTION_SKIP_LIMIT {
            return true;
        }
        if let Some(max_file_bytes) = self.max_file_bytes_before_compaction {
            let file_bytes = self.estimated_file_bytes.load(Ordering::Acquire);
            return file_bytes > max_file_bytes;
        }
        false
    }

    #[cfg(test)]
    async fn flush_for_tests(&self) -> io::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.write_tx
            .send(PersistentEventWrite::Flush(tx))
            .await
            .map_err(|_| io::Error::other("persistent event writer stopped"))?;
        tokio::time::timeout(PERSISTENT_WRITER_FLUSH_TIMEOUT, rx)
            .await
            .map_err(|_| io::Error::other("persistent event writer flush timed out"))?
            .map_err(|_| io::Error::other("persistent event writer stopped before flush"))?
    }
}

fn spawn_persistent_event_writer(
    path: PathBuf,
    write_rx: mpsc::Receiver<PersistentEventWrite>,
) -> io::Result<()> {
    thread::Builder::new()
        .name("chariox-event-log-writer".to_string())
        .spawn(move || run_persistent_event_writer(path, write_rx))
        .map(|_| ())
}

fn run_persistent_event_writer(
    path: PathBuf,
    mut write_rx: mpsc::Receiver<PersistentEventWrite>,
) {
    while let Some(write) = write_rx.blocking_recv() {
        match write {
            PersistentEventWrite::Append {
                append_jsonl,
                reply_tx,
            } => {
                let result = retry_persistent_write(|| {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    append_logged_event_jsonl(&path, &append_jsonl)
                });
                if let Err(error) = &result {
                    crate::logging::warn_with_fields(
                        "daemon.event_log",
                        "persistent event append failed after bounded retries",
                        serde_json::json!({
                            "path": path.display().to_string(),
                            "attempts": PERSISTENT_WRITE_MAX_ATTEMPTS,
                            "error": error.to_string(),
                        }),
                    );
                }
                let _ = reply_tx.send(result);
            }
            PersistentEventWrite::Compact {
                compact_jsonl,
                reply_tx,
            } => {
                let result = retry_persistent_write(|| {
                    rewrite_logged_events_jsonl(&path, &compact_jsonl)
                });
                let _ = reply_tx.send(result);
            }
            #[cfg(test)]
            PersistentEventWrite::Flush(reply_tx) => {
                let _ = reply_tx.send(Ok(()));
            }
        }
    }
}

fn retry_persistent_write(mut write: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut attempt = 0_usize;
    loop {
        attempt += 1;
        match write() {
            Ok(()) => return Ok(()),
            Err(_) if attempt < PERSISTENT_WRITE_MAX_ATTEMPTS => {
                let retry_delay_ms = PERSISTENT_WRITE_RETRY_BASE_DELAY_MS
                    .saturating_mul(1_u64 << (attempt - 1));
                thread::sleep(Duration::from_millis(retry_delay_ms));
            }
            Err(error) => return Err(error),
        }
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
            return Ok((BTreeMap::new(), false));
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
        if retained_events_jsonl_bytes(streams) > max_total_bytes {
            let target_total_bytes = persistent_compaction_target_bytes(max_total_bytes);
            while retained_events_jsonl_bytes(streams) > target_total_bytes {
                if !remove_oldest_retained_event(streams) {
                    break;
                }
                compacted = true;
            }
        }
    }

    compacted
}

fn persistent_compaction_target_bytes(max_total_bytes: u64) -> u64 {
    max_total_bytes
        .saturating_mul(PERSISTENT_COMPACTION_TARGET_NUMERATOR)
        .checked_div(PERSISTENT_COMPACTION_TARGET_DENOMINATOR)
        .unwrap_or(0)
        .max(1)
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
    Ok(logged_event_jsonl_payload(event)?.len() as u64)
}

fn logged_event_jsonl_payload<E>(event: &LoggedEvent<E>) -> io::Result<Vec<u8>>
where
    E: Serialize,
{
    let mut bytes = serde_json::to_vec(event).map_err(io::Error::other)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn logged_events_jsonl_payload<E>(events: &[LoggedEvent<E>]) -> io::Result<Vec<u8>>
where
    E: Serialize,
{
    let mut bytes = Vec::new();
    for event in events {
        serde_json::to_writer(&mut bytes, event).map_err(io::Error::other)?;
        bytes.write_all(b"\n")?;
    }
    Ok(bytes)
}

fn append_logged_event_jsonl(path: &Path, payload: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    truncate_torn_jsonl_file_tail(&mut file)?;
    file.write_all(payload)
}

fn truncate_torn_jsonl_tail(path: &Path) -> io::Result<()> {
    let mut file = match fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    truncate_torn_jsonl_file_tail(&mut file)
}

fn truncate_torn_jsonl_file_tail(file: &mut fs::File) -> io::Result<()> {
    const SCAN_CHUNK_BYTES: u64 = 8 * 1024;

    let file_len = file.metadata()?.len();
    if file_len == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::End(-1))?;
    let mut final_byte = [0_u8; 1];
    file.read_exact(&mut final_byte)?;
    if final_byte[0] == b'\n' {
        return Ok(());
    }

    let mut search_end = file_len;
    let mut buffer = vec![0_u8; SCAN_CHUNK_BYTES as usize];
    while search_end > 0 {
        let search_start = search_end.saturating_sub(SCAN_CHUNK_BYTES);
        let read_len = (search_end - search_start) as usize;
        file.seek(SeekFrom::Start(search_start))?;
        file.read_exact(&mut buffer[..read_len])?;
        if let Some(newline_offset) = buffer[..read_len].iter().rposition(|byte| *byte == b'\n') {
            file.set_len(search_start + newline_offset as u64 + 1)?;
            file.seek(SeekFrom::End(0))?;
            return Ok(());
        }
        search_end = search_start;
    }
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

fn rewrite_logged_events<E>(path: &Path, events: &[LoggedEvent<E>]) -> io::Result<()>
where
    E: Serialize,
{
    rewrite_logged_events_jsonl(path, &logged_events_jsonl_payload(events)?)
}

fn rewrite_logged_events_jsonl(path: &Path, payload: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("jsonl.tmp");
    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(payload)?;
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
mod tests;
