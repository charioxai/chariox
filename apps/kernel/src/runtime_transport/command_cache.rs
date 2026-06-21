use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{oneshot, Mutex};

use crate::local::LocalDaemonRequest;
use crate::runtime::command::KernelCommand;
use crate::transport::kernel_protocol::{KernelOutgoingFrame, KernelTransportError};

pub(crate) const COMMAND_RESULT_CACHE_LIMIT: usize = 512;
const COMMAND_RESULT_CACHE_MAX_BYTES: u64 = 50 * 1024 * 1024;
const COMMAND_RESULT_CACHE_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
const COMMAND_RESULT_COMPACTION_SKIP_LIMIT: u64 = 1_024;
const COMMAND_RESULT_COMPACTION_FILE_GROWTH_MULTIPLIER: u64 = 2;
const COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommandResultRetentionPolicy {
    max_entries: usize,
    max_total_bytes: Option<u64>,
    max_age_ms: Option<u64>,
}

impl CommandResultRetentionPolicy {
    fn memory() -> Self {
        Self {
            max_entries: COMMAND_RESULT_CACHE_LIMIT,
            max_total_bytes: None,
            max_age_ms: None,
        }
    }

    fn persistent() -> Self {
        Self {
            max_entries: COMMAND_RESULT_CACHE_LIMIT,
            max_total_bytes: Some(COMMAND_RESULT_CACHE_MAX_BYTES),
            max_age_ms: Some(COMMAND_RESULT_CACHE_MAX_AGE_MS),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedCommandResult {
    pub(crate) response: Box<Option<Value>>,
    pub(crate) error: Option<KernelTransportError>,
    #[serde(default)]
    completed_at_ms: u64,
    fingerprint: CommandFingerprint,
}

#[derive(Debug)]
enum CommandResultEntry {
    Pending {
        fingerprint: CommandFingerprint,
        waiters: Vec<oneshot::Sender<CachedCommandResult>>,
    },
    Completed(CachedCommandResult),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CommandFingerprint {
    command_type: String,
    source: String,
    session_id: Option<String>,
    attachment_id: Option<String>,
    request_hash: u64,
}

impl CommandFingerprint {
    pub(crate) fn from_command_and_request(
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Self {
        let request_bytes = serde_json::to_vec(request).unwrap_or_default();
        Self {
            command_type: command.command_type.clone(),
            source: serde_json::to_string(&command.source)
                .unwrap_or_else(|_| "unknown".to_string()),
            session_id: command.session_id.clone(),
            attachment_id: command.attachment_id.clone(),
            request_hash: stable_hash64(&request_bytes),
        }
    }
}

pub(crate) enum CommandReservation {
    Dispatch,
    Wait(oneshot::Receiver<CachedCommandResult>),
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentCommandResult {
    command_id: String,
    #[serde(default)]
    completed_at_ms: u64,
    result: CachedCommandResult,
}

#[derive(Debug)]
struct PersistentCommandResultWithBytes {
    entry: PersistentCommandResult,
    jsonl_bytes: u64,
}

#[derive(Debug, Default)]
struct LoadedPersistentCommandResults {
    entries: Vec<PersistentCommandResultWithBytes>,
    compact_after_load: bool,
}

#[derive(Debug)]
struct CommandResultPersistence {
    path: PathBuf,
    io_lock: Mutex<()>,
    skipped_compactions: AtomicU64,
    max_file_bytes_before_compaction: Option<u64>,
}

#[derive(Debug, Default)]
struct CommandResultByteAccounting {
    by_command_id: BTreeMap<String, u64>,
    total_jsonl_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct CommandResultCache {
    results: Mutex<BTreeMap<String, CommandResultEntry>>,
    order: Mutex<VecDeque<String>>,
    byte_accounting: Mutex<CommandResultByteAccounting>,
    retention: CommandResultRetentionPolicy,
    persistence: Option<CommandResultPersistence>,
}

impl Default for CommandResultCache {
    fn default() -> Self {
        Self {
            results: Mutex::new(BTreeMap::new()),
            order: Mutex::new(VecDeque::new()),
            byte_accounting: Mutex::new(CommandResultByteAccounting::default()),
            retention: CommandResultRetentionPolicy::memory(),
            persistence: None,
        }
    }
}

impl CommandResultCache {
    pub(crate) fn new_with_persistent_path(path: impl Into<PathBuf>) -> io::Result<Self> {
        Self::new_with_persistent_path_and_retention(
            path,
            CommandResultRetentionPolicy::persistent(),
        )
    }

    fn new_with_persistent_path_and_retention(
        path: impl Into<PathBuf>,
        retention: CommandResultRetentionPolicy,
    ) -> io::Result<Self> {
        let path = path.into();
        let mut cache = Self {
            results: Mutex::new(BTreeMap::new()),
            order: Mutex::new(VecDeque::new()),
            byte_accounting: Mutex::new(CommandResultByteAccounting::default()),
            retention,
            persistence: Some(CommandResultPersistence {
                path: path.clone(),
                io_lock: Mutex::new(()),
                skipped_compactions: AtomicU64::new(0),
                max_file_bytes_before_compaction: retention.max_total_bytes.map(|bytes| {
                    bytes.saturating_mul(COMMAND_RESULT_COMPACTION_FILE_GROWTH_MULTIPLIER)
                }),
            }),
        };
        let retained = read_persistent_results(&path, retention)?;
        if retained.compact_after_load {
            let entries = retained
                .entries
                .iter()
                .map(|entry| entry.entry.clone())
                .collect::<Vec<_>>();
            rewrite_persistent_results(&path, &entries)?;
        }
        let mut results = BTreeMap::new();
        let mut order = VecDeque::new();
        let mut byte_accounting = CommandResultByteAccounting::default();
        for retained in retained.entries {
            let entry = retained.entry;
            let entry_bytes = retained.jsonl_bytes;
            byte_accounting.total_jsonl_bytes = byte_accounting
                .total_jsonl_bytes
                .saturating_add(entry_bytes);
            byte_accounting
                .by_command_id
                .insert(entry.command_id.clone(), entry_bytes);
            order.push_back(entry.command_id.clone());
            results.insert(
                entry.command_id,
                CommandResultEntry::Completed(entry.result),
            );
        }
        cache.results = Mutex::new(results);
        cache.order = Mutex::new(order);
        cache.byte_accounting = Mutex::new(byte_accounting);
        Ok(cache)
    }

    pub(crate) async fn reserve(
        &self,
        command_id: &str,
        fingerprint: &CommandFingerprint,
    ) -> CommandReservation {
        let mut results = self.results.lock().await;
        match results.get_mut(command_id) {
            Some(CommandResultEntry::Completed(cached)) => {
                if cached.fingerprint == *fingerprint {
                    let (tx, rx) = oneshot::channel();
                    let _ = tx.send(cached.clone());
                    CommandReservation::Wait(rx)
                } else {
                    CommandReservation::Conflict
                }
            }
            Some(CommandResultEntry::Pending {
                fingerprint: existing,
                waiters,
            }) => {
                if existing == fingerprint {
                    let (tx, rx) = oneshot::channel();
                    waiters.push(tx);
                    CommandReservation::Wait(rx)
                } else {
                    CommandReservation::Conflict
                }
            }
            None => {
                results.insert(
                    command_id.to_string(),
                    CommandResultEntry::Pending {
                        fingerprint: fingerprint.clone(),
                        waiters: Vec::new(),
                    },
                );
                CommandReservation::Dispatch
            }
        }
    }

    pub(crate) async fn complete(
        &self,
        command_id: String,
        fingerprint: CommandFingerprint,
        frame: &KernelOutgoingFrame,
    ) {
        let KernelOutgoingFrame::Response {
            response, error, ..
        } = frame
        else {
            return;
        };
        let cached = CachedCommandResult {
            fingerprint,
            completed_at_ms: crate::session::unix_epoch_ms(),
            response: response.clone(),
            error: error.clone(),
        };
        let waiters = {
            let mut results = self.results.lock().await;
            match results.insert(
                command_id.clone(),
                CommandResultEntry::Completed(cached.clone()),
            ) {
                Some(CommandResultEntry::Pending { waiters, .. }) => waiters,
                _ => Vec::new(),
            }
        };
        for waiter in waiters {
            let _ = waiter.send(cached.clone());
        }
        self.record_completed_order(command_id, cached).await;
    }

    async fn record_completed_order(&self, command_id: String, cached: CachedCommandResult) {
        let persisted_bytes = if should_persist_completed_result(&cached.fingerprint) {
            let persisted = PersistentCommandResult {
                command_id: command_id.clone(),
                completed_at_ms: cached.completed_at_ms,
                result: cached.clone(),
            };
            persistent_result_jsonl_bytes(&persisted)
                .ok()
                .filter(|bytes| *bytes <= COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES)
        } else {
            None
        };
        let should_persist = persisted_bytes.is_some();
        let compact_after_append = self
            .apply_retention_to_completed_results(&command_id, persisted_bytes)
            .await;
        if !should_persist {
            return;
        }
        if let Err(error) = self
            .persist_completed_result(command_id, cached, compact_after_append)
            .await
        {
            crate::logging::warn_with_fields(
                "daemon.runtime_transport",
                "failed to persist command result cache",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
        }
    }

    async fn persist_completed_result(
        &self,
        command_id: String,
        cached: CachedCommandResult,
        compact_after_append: bool,
    ) -> io::Result<()> {
        let Some(persistence) = &self.persistence else {
            return Ok(());
        };
        let persisted = PersistentCommandResult {
            command_id,
            completed_at_ms: cached.completed_at_ms,
            result: cached,
        };
        let next_append_bytes = persistent_result_jsonl_bytes(&persisted).unwrap_or(0);
        if next_append_bytes > COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES {
            return Ok(());
        }
        let compact_snapshot =
            if compact_after_append && persistence.should_compact_now(next_append_bytes)? {
                Some(self.completed_results_snapshot().await)
            } else {
                None
            };
        let _guard = persistence.io_lock.lock().await;
        if let Some(parent) = persistence.path.parent() {
            fs::create_dir_all(parent)?;
        }
        append_persistent_result(&persistence.path, &persisted)?;
        if let Some(snapshot) = compact_snapshot {
            rewrite_persistent_results(&persistence.path, &snapshot)?;
            persistence.skipped_compactions.store(0, Ordering::Release);
        }
        Ok(())
    }

    async fn apply_retention_to_completed_results(
        &self,
        completed_command_id: &str,
        completed_jsonl_bytes: Option<u64>,
    ) -> bool {
        let mut order = self.order.lock().await;
        let mut results = self.results.lock().await;
        let mut byte_accounting = self.byte_accounting.lock().await;

        if let Some(existing_index) = order.iter().position(|entry| entry == completed_command_id) {
            order.remove(existing_index);
        }
        order.push_back(completed_command_id.to_string());

        if let Some(bytes) = completed_jsonl_bytes {
            if let Some(previous_bytes) = byte_accounting
                .by_command_id
                .insert(completed_command_id.to_string(), bytes)
            {
                byte_accounting.total_jsonl_bytes = byte_accounting
                    .total_jsonl_bytes
                    .saturating_sub(previous_bytes);
            }
            byte_accounting.total_jsonl_bytes =
                byte_accounting.total_jsonl_bytes.saturating_add(bytes);
        }

        let mut compacted = false;
        let now_ms = crate::session::unix_epoch_ms();

        if let Some(max_age_ms) = self.retention.max_age_ms {
            while order.front().is_some_and(|command_id| {
                results
                    .get(command_id)
                    .and_then(|entry| match entry {
                        CommandResultEntry::Completed(result) => Some(result.completed_at_ms),
                        CommandResultEntry::Pending { .. } => None,
                    })
                    .is_some_and(|completed_at_ms| {
                        completed_at_ms != 0 && now_ms.saturating_sub(completed_at_ms) > max_age_ms
                    })
            }) {
                compacted |=
                    remove_oldest_completed_result(&mut order, &mut results, &mut byte_accounting);
            }
        }

        while order.len() > self.retention.max_entries {
            compacted |=
                remove_oldest_completed_result(&mut order, &mut results, &mut byte_accounting);
        }

        if let Some(max_total_bytes) = self.retention.max_total_bytes {
            while byte_accounting.total_jsonl_bytes > max_total_bytes {
                if !remove_oldest_completed_result(&mut order, &mut results, &mut byte_accounting) {
                    break;
                }
                compacted = true;
            }
        }

        compacted
    }

    async fn completed_results_snapshot(&self) -> Vec<PersistentCommandResult> {
        let order = self.order.lock().await;
        let results = self.results.lock().await;
        order
            .iter()
            .filter_map(|command_id| {
                let Some(CommandResultEntry::Completed(result)) = results.get(command_id) else {
                    return None;
                };
                Some(PersistentCommandResult {
                    command_id: command_id.clone(),
                    completed_at_ms: result.completed_at_ms,
                    result: result.clone(),
                })
            })
            .collect()
    }

    #[cfg(test)]
    pub(super) async fn completed_count(&self) -> usize {
        self.completed_results_snapshot().await.len()
    }

    #[cfg(test)]
    pub(super) async fn insert_completed_for_test(
        &self,
        command_id: String,
        fingerprint: CommandFingerprint,
        response: Option<Value>,
    ) {
        let cached = CachedCommandResult {
            fingerprint,
            completed_at_ms: crate::session::unix_epoch_ms(),
            response: Box::new(response),
            error: None,
        };
        self.results.lock().await.insert(
            command_id.clone(),
            CommandResultEntry::Completed(cached.clone()),
        );
        self.record_completed_order(command_id, cached).await;
    }

    #[cfg(test)]
    pub(super) fn fingerprint_for_test(request: &LocalDaemonRequest) -> CommandFingerprint {
        let command = KernelCommand::from_local_request_with_caller(
            "test-command".to_string(),
            crate::runtime::command::KernelCommandSource::LocalCli,
            crate::runtime::command::KernelCaller::for_source(
                &crate::runtime::command::KernelCommandSource::LocalCli,
            ),
            None,
            None,
            request,
        );
        CommandFingerprint::from_command_and_request(&command, request)
    }

    #[cfg(test)]
    pub(super) fn fingerprint_from_bytes_for_test(bytes: &[u8]) -> CommandFingerprint {
        CommandFingerprint {
            command_type: "test".to_string(),
            source: "test".to_string(),
            session_id: None,
            attachment_id: None,
            request_hash: stable_hash64(bytes),
        }
    }

    #[cfg(test)]
    pub(super) fn fingerprint_for_command_type_test(command_type: &str) -> CommandFingerprint {
        CommandFingerprint {
            command_type: command_type.to_string(),
            source: "test".to_string(),
            session_id: None,
            attachment_id: None,
            request_hash: stable_hash64(command_type.as_bytes()),
        }
    }

    #[cfg(test)]
    pub(super) fn request_hash_for_test(fingerprint: &CommandFingerprint) -> u64 {
        fingerprint.request_hash
    }

    pub(crate) async fn forget_pending(&self, command_id: &str) {
        let mut results = self.results.lock().await;
        if matches!(
            results.get(command_id),
            Some(CommandResultEntry::Pending { .. })
        ) {
            results.remove(command_id);
        }
    }
}

impl CommandResultPersistence {
    fn should_compact_now(&self, next_append_bytes: u64) -> io::Result<bool> {
        let skipped = self.skipped_compactions.fetch_add(1, Ordering::AcqRel) + 1;
        if skipped >= COMMAND_RESULT_COMPACTION_SKIP_LIMIT {
            return Ok(true);
        }
        if let Some(max_file_bytes) = self.max_file_bytes_before_compaction {
            let file_bytes = match fs::metadata(&self.path) {
                Ok(metadata) => metadata.len(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
                Err(error) => return Err(error),
            };
            return Ok(file_bytes.saturating_add(next_append_bytes) > max_file_bytes);
        }
        Ok(false)
    }
}

fn remove_oldest_completed_result(
    order: &mut VecDeque<String>,
    results: &mut BTreeMap<String, CommandResultEntry>,
    byte_accounting: &mut CommandResultByteAccounting,
) -> bool {
    let Some(command_id) = order.pop_front() else {
        return false;
    };
    results.remove(&command_id);
    if let Some(bytes) = byte_accounting.by_command_id.remove(&command_id) {
        byte_accounting.total_jsonl_bytes = byte_accounting.total_jsonl_bytes.saturating_sub(bytes);
    }
    true
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    bytes.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

fn read_persistent_results(
    path: &PathBuf,
    retention: CommandResultRetentionPolicy,
) -> io::Result<LoadedPersistentCommandResults> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LoadedPersistentCommandResults::default())
        }
        Err(error) => return Err(error),
    };
    if let Some(max_total_bytes) = retention.max_total_bytes {
        let max_load_bytes =
            max_total_bytes.saturating_mul(COMMAND_RESULT_COMPACTION_FILE_GROWTH_MULTIPLIER);
        if metadata.len() > max_load_bytes {
            return Ok(LoadedPersistentCommandResults {
                entries: Vec::new(),
                compact_after_load: true,
            });
        }
    }

    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut results = BTreeMap::<String, PersistentCommandResultWithBytes>::new();
    let mut order = VecDeque::<String>::new();
    let mut compact_after_load = false;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let jsonl_bytes = line.as_bytes().len().saturating_add(1) as u64;
        if jsonl_bytes > COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES {
            compact_after_load = true;
            continue;
        }
        let Ok(mut entry) = serde_json::from_str::<PersistentCommandResult>(&line) else {
            compact_after_load = true;
            continue;
        };
        let completed_at_ms = persistent_result_completed_at_ms(&entry);
        entry.completed_at_ms = completed_at_ms;
        entry.result.completed_at_ms = completed_at_ms;
        if let Some(existing_index) = order
            .iter()
            .position(|command_id| command_id == &entry.command_id)
        {
            order.remove(existing_index);
            compact_after_load = true;
        }
        order.push_back(entry.command_id.clone());
        if results
            .insert(
                entry.command_id.clone(),
                PersistentCommandResultWithBytes { entry, jsonl_bytes },
            )
            .is_some()
        {
            compact_after_load = true;
        }
    }
    let mut entries = order
        .into_iter()
        .filter_map(|command_id| results.remove(&command_id))
        .collect::<Vec<_>>();
    compact_after_load |= apply_persistent_retention(&mut entries, retention);
    Ok(LoadedPersistentCommandResults {
        entries,
        compact_after_load,
    })
}

fn apply_persistent_retention(
    entries: &mut Vec<PersistentCommandResultWithBytes>,
    retention: CommandResultRetentionPolicy,
) -> bool {
    let original_len = entries.len();
    let now_ms = crate::session::unix_epoch_ms();
    if let Some(max_age_ms) = retention.max_age_ms {
        entries.retain(|entry| {
            let completed_at_ms = persistent_result_completed_at_ms(&entry.entry);
            completed_at_ms == 0 || now_ms.saturating_sub(completed_at_ms) <= max_age_ms
        });
    }
    let mut compacted = entries.len() != original_len;
    if entries.len() > retention.max_entries {
        let drop_count = entries.len().saturating_sub(retention.max_entries);
        entries.drain(0..drop_count);
        compacted = true;
    }
    if let Some(max_total_bytes) = retention.max_total_bytes {
        let mut total_bytes = entries.iter().fold(0_u64, |total, entry| {
            total.saturating_add(entry.jsonl_bytes)
        });
        while total_bytes > max_total_bytes {
            if entries.is_empty() {
                break;
            }
            let removed = entries.remove(0);
            total_bytes = total_bytes.saturating_sub(removed.jsonl_bytes);
            compacted = true;
        }
    }
    compacted
}

fn persistent_result_completed_at_ms(entry: &PersistentCommandResult) -> u64 {
    if entry.completed_at_ms != 0 {
        entry.completed_at_ms
    } else {
        entry.result.completed_at_ms
    }
}

fn persistent_result_jsonl_bytes(entry: &PersistentCommandResult) -> io::Result<u64> {
    let bytes = serde_json::to_vec(entry).map_err(io::Error::other)?;
    Ok(bytes.len().saturating_add(1) as u64)
}

fn should_persist_completed_result(fingerprint: &CommandFingerprint) -> bool {
    !matches!(
        fingerprint.command_type.as_str(),
        "external_provider_session.list"
            | "provider.catalog.get"
            | "session.state.get"
            | "slice.list"
            | "waiting_room.inventory.get"
            | "waiting_room.public_snapshot.get"
    )
}

fn append_persistent_result(path: &PathBuf, entry: &PersistentCommandResult) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, entry).map_err(io::Error::other)?;
    file.write_all(b"\n")
}

fn rewrite_persistent_results(
    path: &PathBuf,
    entries: &[PersistentCommandResult],
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("jsonl.tmp");
    let mut file = fs::File::create(&tmp_path)?;
    for entry in entries {
        serde_json::to_writer(&mut file, entry).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
    }
    fs::rename(tmp_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{ListSessionsRequest, LocalDaemonRequest};

    #[tokio::test]
    async fn persistent_command_cache_recovers_completed_results() {
        let path = temp_cache_path("recover-completed");
        let request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let fingerprint = CommandResultCache::fingerprint_for_test(&request);
        let cache = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should initialize");
        cache
            .insert_completed_for_test(
                "command-1".to_string(),
                fingerprint.clone(),
                Some(serde_json::json!({"ok": true})),
            )
            .await;

        let restored = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should reload");
        let wait = match restored.reserve("command-1", &fingerprint).await {
            CommandReservation::Wait(wait) => wait,
            _ => panic!("completed command should be replayable after reload"),
        };
        let result = wait.await.expect("cached result should resolve");
        assert_eq!(*result.response, Some(serde_json::json!({"ok": true})));

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn persistent_command_cache_rejects_conflicting_reuse_after_reload() {
        let path = temp_cache_path("reject-conflict");
        let first = CommandResultCache::fingerprint_from_bytes_for_test(b"first");
        let second = CommandResultCache::fingerprint_from_bytes_for_test(b"second");
        let cache = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should initialize");
        cache
            .insert_completed_for_test(
                "command-1".to_string(),
                first,
                Some(serde_json::json!({"ok": true})),
            )
            .await;

        let restored = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should reload");
        assert!(matches!(
            restored.reserve("command-1", &second).await,
            CommandReservation::Conflict
        ));

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn persistent_command_cache_compacts_to_retention_limit() {
        let path = temp_cache_path("compact-retention");
        let cache = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should initialize");
        for index in 0..(COMMAND_RESULT_CACHE_LIMIT + 8) {
            let fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(
                format!("request-{index}").as_bytes(),
            );
            cache
                .insert_completed_for_test(
                    format!("command-{index}"),
                    fingerprint,
                    Some(serde_json::json!({ "index": index })),
                )
                .await;
        }
        assert_eq!(cache.completed_count().await, COMMAND_RESULT_CACHE_LIMIT);

        let restored = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should reload");
        assert_eq!(restored.completed_count().await, COMMAND_RESULT_CACHE_LIMIT);

        let first_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"request-0");
        assert!(matches!(
            restored.reserve("command-0", &first_fingerprint).await,
            CommandReservation::Dispatch
        ));
        let retained_fingerprint =
            CommandResultCache::fingerprint_from_bytes_for_test(b"request-8");
        assert!(matches!(
            restored.reserve("command-8", &retained_fingerprint).await,
            CommandReservation::Wait(_)
        ));

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn persistent_command_cache_compacts_by_age_on_load() {
        let path = temp_cache_path("compact-age");
        let now_ms = crate::session::unix_epoch_ms();
        let old = persistent_result_for_test(
            "command-old",
            CommandResultCache::fingerprint_from_bytes_for_test(b"old"),
            now_ms.saturating_sub(10_000),
            Some(serde_json::json!({ "value": "old" })),
        );
        let fresh = persistent_result_for_test(
            "command-fresh",
            CommandResultCache::fingerprint_from_bytes_for_test(b"fresh"),
            now_ms,
            Some(serde_json::json!({ "value": "fresh" })),
        );
        rewrite_persistent_results(&path, &[old, fresh]).expect("cache fixture should write");
        let retention = CommandResultRetentionPolicy {
            max_entries: COMMAND_RESULT_CACHE_LIMIT,
            max_total_bytes: None,
            max_age_ms: Some(1_000),
        };

        let cache =
            CommandResultCache::new_with_persistent_path_and_retention(path.clone(), retention)
                .expect("persistent cache should reload");

        assert_eq!(cache.completed_count().await, 1);
        let old_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"old");
        assert!(matches!(
            cache.reserve("command-old", &old_fingerprint).await,
            CommandReservation::Dispatch
        ));
        let fresh_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"fresh");
        assert!(matches!(
            cache.reserve("command-fresh", &fresh_fingerprint).await,
            CommandReservation::Wait(_)
        ));
        let stored = fs::read_to_string(&path).expect("compacted cache should exist");
        assert!(!stored.contains("command-old"));
        assert!(stored.contains("command-fresh"));

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn persistent_command_cache_compacts_by_total_bytes() {
        let path = temp_cache_path("compact-bytes");
        let first_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"first");
        let second_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"second");
        let third_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"third");
        let first_response = Some(serde_json::json!({ "payload": "x".repeat(120) }));
        let second_response = Some(serde_json::json!({ "payload": "y".repeat(120) }));
        let third_response = Some(serde_json::json!({ "payload": "z".repeat(120) }));
        let second_entry = persistent_result_for_test(
            "command-second",
            second_fingerprint.clone(),
            crate::session::unix_epoch_ms(),
            second_response.clone(),
        );
        let retention = CommandResultRetentionPolicy {
            max_entries: COMMAND_RESULT_CACHE_LIMIT,
            max_total_bytes: Some(
                persistent_result_jsonl_bytes(&second_entry).expect("entry should serialize"),
            ),
            max_age_ms: None,
        };
        let cache =
            CommandResultCache::new_with_persistent_path_and_retention(path.clone(), retention)
                .expect("persistent cache should initialize");
        cache
            .insert_completed_for_test(
                "command-first".to_string(),
                first_fingerprint.clone(),
                first_response,
            )
            .await;
        cache
            .insert_completed_for_test(
                "command-second".to_string(),
                second_fingerprint.clone(),
                second_response,
            )
            .await;

        let stored = fs::read_to_string(&path).expect("cache should exist");
        assert!(
            stored.contains("command-first"),
            "disk compaction may be deferred until file growth is material"
        );
        assert!(matches!(
            cache.reserve("command-first", &first_fingerprint).await,
            CommandReservation::Dispatch
        ));
        cache.forget_pending("command-first").await;
        assert!(matches!(
            cache.reserve("command-second", &second_fingerprint).await,
            CommandReservation::Wait(_)
        ));

        cache
            .insert_completed_for_test(
                "command-third".to_string(),
                third_fingerprint.clone(),
                third_response,
            )
            .await;

        let stored = fs::read_to_string(&path).expect("cache should exist");
        assert!(
            stored.len() as u64
                <= retention.max_total_bytes.unwrap()
                    * COMMAND_RESULT_COMPACTION_FILE_GROWTH_MULTIPLIER,
            "cache should compact once file growth crosses the byte budget multiplier: {stored}"
        );
        assert!(!stored.contains("command-first"));
        assert!(!stored.contains("command-second"));
        assert!(stored.contains("command-third"));
        assert!(matches!(
            cache.reserve("command-first", &first_fingerprint).await,
            CommandReservation::Dispatch
        ));
        assert!(matches!(
            cache.reserve("command-second", &second_fingerprint).await,
            CommandReservation::Dispatch
        ));
        assert!(matches!(
            cache.reserve("command-third", &third_fingerprint).await,
            CommandReservation::Wait(_)
        ));

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn persistent_command_cache_drops_oversized_records_on_load() {
        let path = temp_cache_path("drop-oversized-records");
        let oversized_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"huge");
        let small_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"small");
        let oversized = persistent_result_for_test(
            "command-huge",
            oversized_fingerprint.clone(),
            crate::session::unix_epoch_ms(),
            Some(serde_json::json!({
                "payload": "x".repeat(
                    COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES as usize
                )
            })),
        );
        let small = persistent_result_for_test(
            "command-small",
            small_fingerprint.clone(),
            crate::session::unix_epoch_ms(),
            Some(serde_json::json!({"ok": true})),
        );
        rewrite_persistent_results(&path, &[oversized, small]).expect("cache fixture should write");

        let cache = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should initialize");

        assert!(matches!(
            cache.reserve("command-huge", &oversized_fingerprint).await,
            CommandReservation::Dispatch
        ));
        cache.forget_pending("command-huge").await;
        assert!(matches!(
            cache.reserve("command-small", &small_fingerprint).await,
            CommandReservation::Wait(_)
        ));
        let stored = fs::read_to_string(&path).expect("compacted cache should exist");
        assert!(!stored.contains("command-huge"));
        assert!(stored.contains("command-small"));

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn persistent_command_cache_does_not_persist_oversized_results() {
        let path = temp_cache_path("skip-oversized-persist");
        let fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"huge-result");
        let cache = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should initialize");
        cache
            .insert_completed_for_test(
                "command-huge".to_string(),
                fingerprint.clone(),
                Some(serde_json::json!({
                    "payload": "x".repeat(
                        COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES as usize
                    )
                })),
            )
            .await;

        assert!(matches!(
            cache.reserve("command-huge", &fingerprint).await,
            CommandReservation::Wait(_)
        ));
        let restored = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should reload");
        assert!(matches!(
            restored.reserve("command-huge", &fingerprint).await,
            CommandReservation::Dispatch
        ));

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn persistent_command_cache_skips_noisy_read_commands_on_disk() {
        let noisy_command_types = [
            "external_provider_session.list",
            "provider.catalog.get",
            "session.state.get",
            "slice.list",
            "waiting_room.inventory.get",
            "waiting_room.public_snapshot.get",
        ];
        let path = temp_cache_path("skip-read-commands");
        let cache = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should initialize");

        for command_type in noisy_command_types {
            let command_id = format!("command-{}", command_type.replace('.', "-"));
            let fingerprint = CommandResultCache::fingerprint_for_command_type_test(command_type);
            cache
                .insert_completed_for_test(
                    command_id.clone(),
                    fingerprint.clone(),
                    Some(serde_json::json!({ "command_type": command_type })),
                )
                .await;

            assert!(matches!(
                cache.reserve(&command_id, &fingerprint).await,
                CommandReservation::Wait(_)
            ));
        }
        assert!(
            fs::read_to_string(&path).unwrap_or_default().is_empty(),
            "high-frequency read command results should not be serialized to disk"
        );

        let restored = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("persistent cache should reload");
        for command_type in noisy_command_types {
            let command_id = format!("command-{}", command_type.replace('.', "-"));
            let fingerprint = CommandResultCache::fingerprint_for_command_type_test(command_type);
            assert!(matches!(
                restored.reserve(&command_id, &fingerprint).await,
                CommandReservation::Dispatch
            ));
        }

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn persistent_command_cache_ignores_malformed_lines() {
        let path = temp_cache_path("ignore-malformed");
        fs::write(
            &path,
            [
                "{not json}",
                r#"{"command_id":"command-1","result":{"response":{"ok":true},"error":null,"fingerprint":{"command_type":"test","source":"test","session_id":null,"attachment_id":null,"request_hash":42}}}"#,
            ]
            .join("\n"),
        )
        .expect("cache fixture should write");

        let cache = CommandResultCache::new_with_persistent_path(path.clone())
            .expect("malformed lines should not prevent cache load");

        assert_eq!(cache.completed_count().await, 1);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn command_fingerprint_hash_is_stable() {
        let first = CommandResultCache::fingerprint_from_bytes_for_test(b"same request");
        let second = CommandResultCache::fingerprint_from_bytes_for_test(b"same request");
        let different = CommandResultCache::fingerprint_from_bytes_for_test(b"different request");

        assert_eq!(
            CommandResultCache::request_hash_for_test(&first),
            CommandResultCache::request_hash_for_test(&second)
        );
        assert_ne!(
            CommandResultCache::request_hash_for_test(&first),
            CommandResultCache::request_hash_for_test(&different)
        );
    }

    fn temp_cache_path(label: &str) -> PathBuf {
        let unique = crate::session::unix_epoch_ms();
        std::env::temp_dir().join(format!(
            "arroba-command-cache-{label}-{}-{unique}.jsonl",
            std::process::id()
        ))
    }

    fn persistent_result_for_test(
        command_id: &str,
        fingerprint: CommandFingerprint,
        completed_at_ms: u64,
        response: Option<Value>,
    ) -> PersistentCommandResult {
        PersistentCommandResult {
            command_id: command_id.to_string(),
            completed_at_ms,
            result: CachedCommandResult {
                response: Box::new(response),
                error: None,
                completed_at_ms,
                fingerprint,
            },
        }
    }
}
