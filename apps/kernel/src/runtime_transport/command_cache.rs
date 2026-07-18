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
const COMMAND_RESULT_CACHE_MAX_MEMORY_BYTES: u64 = 128 * 1024 * 1024;
const COMMAND_RESULT_CACHE_MAX_BYTES: u64 = 50 * 1024 * 1024;
const COMMAND_RESULT_CACHE_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
const COMMAND_RESULT_COMPACTION_SKIP_LIMIT: u64 = 1_024;
const COMMAND_RESULT_COMPACTION_FILE_GROWTH_MULTIPLIER: u64 = 2;
const COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommandResultRetentionPolicy {
    max_entries: usize,
    max_memory_bytes: u64,
    max_total_bytes: Option<u64>,
    max_age_ms: Option<u64>,
}

impl CommandResultRetentionPolicy {
    fn memory() -> Self {
        Self {
            max_entries: COMMAND_RESULT_CACHE_LIMIT,
            max_memory_bytes: COMMAND_RESULT_CACHE_MAX_MEMORY_BYTES,
            max_total_bytes: None,
            max_age_ms: None,
        }
    }

    fn persistent() -> Self {
        Self {
            max_entries: COMMAND_RESULT_CACHE_LIMIT,
            max_memory_bytes: COMMAND_RESULT_CACHE_MAX_MEMORY_BYTES,
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
struct CommandResultMemoryAccounting {
    by_command_id: BTreeMap<String, u64>,
    total_estimated_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct CommandResultCache {
    results: Mutex<BTreeMap<String, CommandResultEntry>>,
    order: Mutex<VecDeque<String>>,
    memory_accounting: Mutex<CommandResultMemoryAccounting>,
    retention: CommandResultRetentionPolicy,
    persistence: Option<CommandResultPersistence>,
}

impl Default for CommandResultCache {
    fn default() -> Self {
        Self {
            results: Mutex::new(BTreeMap::new()),
            order: Mutex::new(VecDeque::new()),
            memory_accounting: Mutex::new(CommandResultMemoryAccounting::default()),
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
            memory_accounting: Mutex::new(CommandResultMemoryAccounting::default()),
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
        let mut memory_accounting = CommandResultMemoryAccounting::default();
        for retained in retained.entries {
            let entry = retained.entry;
            let entry_bytes = cached_command_result_memory_bytes(&entry.command_id, &entry.result);
            memory_accounting.total_estimated_bytes = memory_accounting
                .total_estimated_bytes
                .saturating_add(entry_bytes);
            memory_accounting
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
        cache.memory_accounting = Mutex::new(memory_accounting);
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
        // Account every completed result in memory, including responses that are too large or
        // too noisy to persist. A Vec<u8> represented as serde_json::Value is especially costly,
        // so entry-count retention alone is not a meaningful memory bound.
        // Do not clone and serialize large read-only responses merely to decide that they should
        // not be written. History outlines are intentionally paged and may still be large enough
        // for this work to become visible on every browser refresh.
        let result_jsonl_bytes = should_persist_completed_result(&cached.fingerprint)
            .then(|| PersistentCommandResult {
                command_id: command_id.clone(),
                completed_at_ms: cached.completed_at_ms,
                result: cached.clone(),
            })
            .and_then(|persisted| persistent_result_jsonl_bytes(&persisted).ok());
        let result_memory_bytes = cached_command_result_memory_bytes(&command_id, &cached);
        let persisted_bytes = result_jsonl_bytes
            .filter(|bytes| *bytes <= COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES);
        let should_persist = persisted_bytes.is_some();
        let compact_after_append = self
            .apply_retention_to_completed_results(&command_id, result_memory_bytes)
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
            if compact_after_append || persistence.should_compact_now(next_append_bytes)? {
                Some(self.persistable_completed_results_snapshot().await)
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
        completed_memory_bytes: u64,
    ) -> bool {
        let mut order = self.order.lock().await;
        let mut results = self.results.lock().await;
        let mut memory_accounting = self.memory_accounting.lock().await;

        if let Some(existing_index) = order.iter().position(|entry| entry == completed_command_id) {
            order.remove(existing_index);
        }
        order.push_back(completed_command_id.to_string());

        if let Some(previous_bytes) = memory_accounting
            .by_command_id
            .insert(completed_command_id.to_string(), completed_memory_bytes)
        {
            memory_accounting.total_estimated_bytes = memory_accounting
                .total_estimated_bytes
                .saturating_sub(previous_bytes);
        }
        memory_accounting.total_estimated_bytes = memory_accounting
            .total_estimated_bytes
            .saturating_add(completed_memory_bytes);

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
                compacted |= remove_oldest_completed_result(
                    &mut order,
                    &mut results,
                    &mut memory_accounting,
                );
            }
        }

        while order.len() > self.retention.max_entries {
            compacted |=
                remove_oldest_completed_result(&mut order, &mut results, &mut memory_accounting);
        }

        while memory_accounting.total_estimated_bytes > self.retention.max_memory_bytes {
            if !remove_oldest_completed_result(&mut order, &mut results, &mut memory_accounting) {
                break;
            }
            compacted = true;
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

    async fn persistable_completed_results_snapshot(&self) -> Vec<PersistentCommandResult> {
        let mut entries = self
            .completed_results_snapshot()
            .await
            .into_iter()
            .filter(|entry| should_persist_completed_result(&entry.result.fingerprint))
            .filter_map(|entry| {
                let jsonl_bytes = persistent_result_jsonl_bytes(&entry).ok()?;
                (jsonl_bytes <= COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES)
                    .then_some(PersistentCommandResultWithBytes { entry, jsonl_bytes })
            })
            .collect::<Vec<_>>();
        apply_persistent_retention(&mut entries, self.retention);
        entries.into_iter().map(|entry| entry.entry).collect()
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
    memory_accounting: &mut CommandResultMemoryAccounting,
) -> bool {
    let Some(command_id) = order.pop_front() else {
        return false;
    };
    results.remove(&command_id);
    if let Some(bytes) = memory_accounting.by_command_id.remove(&command_id) {
        memory_accounting.total_estimated_bytes = memory_accounting
            .total_estimated_bytes
            .saturating_sub(bytes);
    }
    true
}

fn cached_command_result_memory_bytes(command_id: &str, result: &CachedCommandResult) -> u64 {
    let mut bytes = std::mem::size_of::<CachedCommandResult>() as u64;
    // The command id is owned once by the result map and once by the completion order.
    bytes = bytes.saturating_add((command_id.len() as u64).saturating_mul(2));
    bytes = bytes
        .saturating_add(result.fingerprint.command_type.capacity() as u64)
        .saturating_add(result.fingerprint.source.capacity() as u64)
        .saturating_add(
            result
                .fingerprint
                .session_id
                .as_ref()
                .map_or(0, |value| value.capacity() as u64),
        )
        .saturating_add(
            result
                .fingerprint
                .attachment_id
                .as_ref()
                .map_or(0, |value| value.capacity() as u64),
        );
    if let Some(error) = &result.error {
        bytes = bytes
            .saturating_add(error.code.capacity() as u64)
            .saturating_add(error.message.capacity() as u64);
    }
    if let Some(response) = result.response.as_ref().as_ref() {
        bytes = bytes
            .saturating_add(std::mem::size_of::<Option<Value>>() as u64)
            .saturating_add(value_heap_bytes(response));
    }
    bytes
}

fn value_heap_bytes(value: &Value) -> u64 {
    match value {
        Value::String(value) => value.capacity() as u64,
        Value::Array(values) => (values.capacity() as u64)
            .saturating_mul(std::mem::size_of::<Value>() as u64)
            .saturating_add(values.iter().fold(0_u64, |total, value| {
                total.saturating_add(value_heap_bytes(value))
            })),
        Value::Object(values) => values.iter().fold(
            (values.len() as u64).saturating_mul(
                (std::mem::size_of::<String>()
                    + std::mem::size_of::<Value>()
                    + 3 * std::mem::size_of::<usize>()) as u64,
            ),
            |total, (key, value)| {
                total
                    .saturating_add(key.capacity() as u64)
                    .saturating_add(value_heap_bytes(value))
            },
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) => 0,
    }
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
            return Ok(LoadedPersistentCommandResults::default());
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
        if !should_persist_completed_result(&entry.result.fingerprint) {
            compact_after_load = true;
            continue;
        }
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
            | "prompt_input_history.get"
            | "session.state.get"
            | "session.history.blob"
            | "session.history.outline"
            | "slice.list"
            | "terminal.command_catalog.get"
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
mod tests;
