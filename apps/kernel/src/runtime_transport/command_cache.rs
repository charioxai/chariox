use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Value};
use tokio::sync::{oneshot, Mutex};

use crate::local::LocalDaemonRequest;
use crate::runtime::command::KernelCommand;
use crate::transport::kernel_protocol::{KernelOutgoingFrame, KernelTransportError};

mod at_most_once;
mod receipt_retention;
pub(crate) use at_most_once::{is_receipt_capacity_error, is_receipt_expired_error};
use receipt_retention::ReceiptRetention;

pub(crate) const COMMAND_RESULT_CACHE_LIMIT: usize = 512;
const COMMAND_RESULT_CACHE_MAX_MEMORY_BYTES: u64 = 128 * 1024 * 1024;
const COMMAND_RESULT_CACHE_MAX_BYTES: u64 = 50 * 1024 * 1024;
const COMMAND_RESULT_CACHE_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
/// App receipts reuse the existing durable command receipt retention window.
pub(crate) const APP_RECEIPT_RETENTION_MS: u64 = COMMAND_RESULT_CACHE_MAX_AGE_MS;
// Legacy at-most-once readers reject this record and therefore fail closed.
const RECEIPT_EXPIRY_FENCE: &str = "chariox.app-receipts.requires-protocol-416";
const COMMAND_RESULT_COMPACTION_SKIP_LIMIT: u64 = 1_024;
const COMMAND_RESULT_COMPACTION_FILE_GROWTH_MULTIPLIER: u64 = 2;
const COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommandResultRetentionPolicy {
    max_entries: usize,
    max_memory_bytes: u64,
    max_total_bytes: Option<u64>,
    max_age_ms: Option<u64>,
    at_most_once: bool,
}

impl CommandResultRetentionPolicy {
    fn memory() -> Self {
        Self {
            max_entries: COMMAND_RESULT_CACHE_LIMIT,
            max_memory_bytes: COMMAND_RESULT_CACHE_MAX_MEMORY_BYTES,
            max_total_bytes: None,
            max_age_ms: None,
            at_most_once: false,
        }
    }

    fn persistent() -> Self {
        Self {
            max_entries: COMMAND_RESULT_CACHE_LIMIT,
            max_memory_bytes: COMMAND_RESULT_CACHE_MAX_MEMORY_BYTES,
            max_total_bytes: Some(COMMAND_RESULT_CACHE_MAX_BYTES),
            max_age_ms: Some(COMMAND_RESULT_CACHE_MAX_AGE_MS),
            at_most_once: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedCommandResult {
    response: Option<Arc<RawValue>>,
    pub(crate) error: Option<KernelTransportError>,
    #[serde(default)]
    completed_at_ms: u64,
    fingerprint: CommandFingerprint,
}

impl CachedCommandResult {
    pub(crate) fn response_value(&self) -> Box<Option<Value>> {
        Box::new(self.response.as_ref().map(|response| {
            let mut deserializer = serde_json::Deserializer::from_str(response.get());
            // Responses are already validated Values; generated trees may exceed
            // the parser's default input nesting limit.
            deserializer.disable_recursion_limit();
            Value::deserialize(&mut deserializer).expect("cached response is valid JSON")
        }))
    }
}

fn serialized_response(response: &Option<Value>) -> Option<Arc<RawValue>> {
    response
        .as_ref()
        .map(|value| Arc::from(serde_json::value::to_raw_value(value).expect("response is JSON")))
}

#[derive(Debug)]
enum CommandResultEntry {
    Pending {
        fingerprint: CommandFingerprint,
        accepted_at_ms: u64,
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
    // MD-N3 / MP-11: browser/note receipts belong to the authenticated terminal identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    browser_caller: Option<crate::runtime::command::KernelCaller>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    browser_protection_revision: Option<u64>,
}

impl CommandFingerprint {
    pub(crate) fn with_browser_protection_revision(mut self, revision: Option<u64>) -> Self {
        self.browser_protection_revision = revision;
        self
    }
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
            browser_caller: matches!(
                request,
                LocalDaemonRequest::KernelBrowser(_) | LocalDaemonRequest::Notes(_)
            )
            .then(|| command.caller.clone()),
            browser_protection_revision: None,
        }
    }
}

pub(crate) enum CommandReservation {
    Dispatch,
    Wait(oneshot::Receiver<CachedCommandResult>),
    Conflict,
}

pub(crate) fn request_is_cacheable(request: &LocalDaemonRequest) -> bool {
    if matches!(
        request,
        LocalDaemonRequest::CaptureVisibleRegion(_)
            | LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
                command: crate::local::KernelBrowserCommand::DisplayNext { .. }
                    | crate::local::KernelBrowserCommand::ListGrants
                    | crate::local::KernelBrowserCommand::SubscribeGrants { .. }
                    | crate::local::KernelBrowserCommand::GrantRoomComputer { .. }
                    | crate::local::KernelBrowserCommand::RevokeGrants { .. }
                    | crate::local::KernelBrowserCommand::MirrorNext { .. }
            })
    ) {
        return false;
    }
    // App requests and browser-import consent must reach owner authorization and
    // current durable state. Their own owner-scoped ledgers deduplicate retries;
    // this older transport fingerprint does not carry the caller, and cached
    // results could outlive live authority. Other commands retain in-memory
    // deduplication; disk exclusions are separate. An excluded request with no
    // ledger that a replay would run again must be on the kernel client's
    // KERNEL_REQUESTS_RUN_AGAIN_ON_REPLAY (packages/kernel-client/src/ipc.ts),
    // which never resends it once written; the tests hold the two lists equal.
    !matches!(
        request,
        LocalDaemonRequest::RequestKernelSudo(_)
            | LocalDaemonRequest::RequestKernelAccess(_)
            | LocalDaemonRequest::ListKernelAccessGrants(_)
            | LocalDaemonRequest::RevokeKernelAccessGrant(_)
            | LocalDaemonRequest::ListAppInstallations(_)
            | LocalDaemonRequest::BeginAppPublisherEnrollment(_)
            | LocalDaemonRequest::GetAppPublisherEnrollment(_)
            | LocalDaemonRequest::CancelAppPublisherEnrollment(_)
            | LocalDaemonRequest::GetAppInstallation(_)
            | LocalDaemonRequest::BeginAppInstall(_)
            | LocalDaemonRequest::BeginAppUpdate(_)
            | LocalDaemonRequest::GetAppInstallOperation(_)
            | LocalDaemonRequest::CancelAppInstallOperation(_)
            | LocalDaemonRequest::GetAppInstallationJournal(_)
            | LocalDaemonRequest::BeginAppPackageUpload(_)
            | LocalDaemonRequest::PutAppPackageUploadChunk(_)
            | LocalDaemonRequest::GetAppPackageUpload(_)
            | LocalDaemonRequest::AbortAppPackageUpload(_)
            | LocalDaemonRequest::GetAppWorker(_)
            | LocalDaemonRequest::ControlAppWorker(_)
            | LocalDaemonRequest::RestoreAppDataSnapshot(_)
            | LocalDaemonRequest::ListAppAutomations(_)
            | LocalDaemonRequest::ConfigureAppAutomation(_)
            | LocalDaemonRequest::DisableAppAutomation(_)
            | LocalDaemonRequest::OpenUserAppView(_)
            | LocalDaemonRequest::ListUserAppViews(_)
            | LocalDaemonRequest::CloseUserAppView(_)
            | LocalDaemonRequest::GetUserAppViewFrontend(_)
            | LocalDaemonRequest::CallUserAppView(_)
            | LocalDaemonRequest::SubscribeUserAppViews(_)
            | LocalDaemonRequest::AnswerUserDomainInteraction(_)
            | LocalDaemonRequest::OpenAppView(_)
            | LocalDaemonRequest::SetAppViewPanel(_)
            | LocalDaemonRequest::UninstallApp(_)
            | LocalDaemonRequest::GetAppLogs(_)
            | LocalDaemonRequest::CreateAppInboxRoute(_)
            | LocalDaemonRequest::RemoveAppInboxRoute(_)
            | LocalDaemonRequest::ListAppInboxRoutes(_)
            | LocalDaemonRequest::TestAppInboxRoute(_)
            | LocalDaemonRequest::GrantAppConnection(_)
            | LocalDaemonRequest::GetAppSet(_)
            | LocalDaemonRequest::PreviewDeploymentApps(_)
            | LocalDaemonRequest::PrepareDeploymentApps(_)
            | LocalDaemonRequest::RevokeAppConnection(_)
            | LocalDaemonRequest::ListAppConnections(_)
            | LocalDaemonRequest::GrantAppFile(_)
            | LocalDaemonRequest::SaveAppFileExport(_)
            | LocalDaemonRequest::RevokeAppFileGrants(_)
            | LocalDaemonRequest::AcceptAppHostAction(_)
            | LocalDaemonRequest::PrepareBrowserImport(_)
            | LocalDaemonRequest::ApproveBrowserImport(_)
            | LocalDaemonRequest::ClaimBrowserImportSource(_)
            | LocalDaemonRequest::AuthorizeBrowserImportSource(_)
            | LocalDaemonRequest::CancelBrowserImport(_)
    )
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
    receipt_retention: Mutex<ReceiptRetention>,
    persistence: Option<CommandResultPersistence>,
    #[cfg(test)]
    fail_settlement_sync: AtomicBool,
}

impl Default for CommandResultCache {
    fn default() -> Self {
        Self {
            results: Mutex::new(BTreeMap::new()),
            order: Mutex::new(VecDeque::new()),
            memory_accounting: Mutex::new(CommandResultMemoryAccounting::default()),
            retention: CommandResultRetentionPolicy::memory(),
            receipt_retention: Mutex::new(ReceiptRetention::default()),
            persistence: None,
            #[cfg(test)]
            fail_settlement_sync: AtomicBool::new(false),
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

    /// Keep 512 owner-scoped identities; only old completed receipts can be evicted.
    /// Durable identity markers always refuse an evicted replay.
    pub(crate) fn new_at_most_once(path: impl Into<PathBuf>) -> io::Result<Self> {
        Self::new_with_persistent_path_and_retention(
            path,
            CommandResultRetentionPolicy {
                at_most_once: true,
                max_age_ms: None,
                ..CommandResultRetentionPolicy::persistent()
            },
        )
    }

    fn new_with_persistent_path_and_retention(
        path: impl Into<PathBuf>,
        retention: CommandResultRetentionPolicy,
    ) -> io::Result<Self> {
        Self::new_with_retention_loader(path, retention, ReceiptRetention::load)
    }

    fn new_with_retention_loader(
        path: impl Into<PathBuf>,
        retention: CommandResultRetentionPolicy,
        load: impl FnOnce(&std::path::Path) -> io::Result<ReceiptRetention>,
    ) -> io::Result<Self> {
        let path = path.into();
        let mut cache = Self {
            results: Mutex::new(BTreeMap::new()),
            order: Mutex::new(VecDeque::new()),
            memory_accounting: Mutex::new(CommandResultMemoryAccounting::default()),
            retention,
            receipt_retention: Mutex::new(ReceiptRetention::default()),
            #[cfg(test)]
            fail_settlement_sync: AtomicBool::new(false),
            persistence: Some(CommandResultPersistence {
                path: path.clone(),
                io_lock: Mutex::new(()),
                skipped_compactions: AtomicU64::new(0),
                max_file_bytes_before_compaction: retention.max_total_bytes.map(|bytes| {
                    bytes.saturating_mul(COMMAND_RESULT_COMPACTION_FILE_GROWTH_MULTIPLIER)
                }),
            }),
        };
        let expired = if retention.at_most_once {
            load(&path)?
        } else {
            ReceiptRetention::default()
        };
        let mut retained = read_persistent_results(&path, retention)?;
        let count = retained.entries.len();
        retained
            .entries
            .retain(|entry| !expired.contains(&entry.entry.command_id));
        retained.compact_after_load |= retained.entries.len() != count;
        cache.receipt_retention = Mutex::new(expired);
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
                ..
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
                        accepted_at_ms: crate::session::unix_epoch_ms(),
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
            response: serialized_response(response),
            error: error.clone(),
        };
        self.publish_completed_result(&command_id, &cached).await;
        self.record_completed_order(command_id, cached).await;
    }

    async fn publish_completed_result(&self, command_id: &str, cached: &CachedCommandResult) {
        let waiters = {
            let mut results = self.results.lock().await;
            match results.insert(
                command_id.to_owned(),
                CommandResultEntry::Completed(cached.clone()),
            ) {
                Some(CommandResultEntry::Pending { waiters, .. }) => waiters,
                _ => Vec::new(),
            }
        };
        for waiter in waiters {
            let _ = waiter.send(cached.clone());
        }
    }

    async fn record_completed_order(&self, command_id: String, cached: CachedCommandResult) {
        // Account serialized response bytes even when the result is excluded from disk.
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
        self.apply_retention_to_completed_results(&command_id, result_memory_bytes)
            .await;
        if !should_persist {
            return;
        }
        if let Err(error) = self.persist_completed_result(command_id, cached).await {
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
            if !self.retention.at_most_once && persistence.should_compact_now(next_append_bytes)? {
                Some(self.persistable_completed_results_snapshot().await)
            } else {
                None
            };
        let _guard = persistence.io_lock.lock().await;
        if let Some(parent) = persistence.path.parent() {
            fs::create_dir_all(parent)?;
        }
        if self.retention.at_most_once && persistence.should_compact_now(next_append_bytes)? {
            // Reloading preserves durable reservations for effects still pending.
            let loaded = read_persistent_results(&persistence.path, self.retention)?;
            let entries = loaded
                .entries
                .into_iter()
                .map(|entry| entry.entry)
                .collect::<Vec<_>>();
            rewrite_persistent_results(&persistence.path, &entries)?;
            persistence.skipped_compactions.store(0, Ordering::Release);
        }
        append_persistent_result(&persistence.path, &persisted)?;
        if self.retention.at_most_once {
            #[cfg(test)]
            if self.fail_settlement_sync.swap(false, Ordering::SeqCst) {
                return Err(io::Error::other("injected settlement sync failure"));
            }
            fs::OpenOptions::new()
                .write(true)
                .open(&persistence.path)?
                .sync_all()?;
        }
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
    ) {
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

        let now_ms = crate::session::unix_epoch_ms();

        if self.retention.at_most_once {
            return;
        }

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
                remove_oldest_completed_result(&mut order, &mut results, &mut memory_accounting);
            }
        }

        while order.len() > self.retention.max_entries {
            remove_oldest_completed_result(&mut order, &mut results, &mut memory_accounting);
        }

        while memory_accounting.total_estimated_bytes > self.retention.max_memory_bytes {
            if !remove_oldest_completed_result(&mut order, &mut results, &mut memory_accounting) {
                break;
            }
        }
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
    pub(super) async fn completed_browser_caller(
        &self,
        command_id: &str,
    ) -> Option<crate::runtime::command::KernelCaller> {
        self.completed_results_snapshot()
            .await
            .into_iter()
            .find(|entry| entry.command_id == command_id)?
            .result
            .fingerprint
            .browser_caller
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
            response: serialized_response(&response),
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
            browser_caller: None,
            browser_protection_revision: None,
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
            browser_caller: None,
            browser_protection_revision: None,
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
    if let Some(response) = &result.response {
        bytes = bytes
            .saturating_add((2 * std::mem::size_of::<usize>()) as u64)
            .saturating_add(response.get().len() as u64);
    }
    bytes
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
    read_persistent_results_with_receipt_fence(path, retention, true)
}

fn read_persistent_results_with_receipt_fence(
    path: &PathBuf,
    retention: CommandResultRetentionPolicy,
    supports_expiry: bool,
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
            if retention.at_most_once {
                return Err(io::Error::other(
                    "at-most-once receipts exceed the load limit",
                ));
            }
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
        if retention.at_most_once && supports_expiry && line == RECEIPT_EXPIRY_FENCE {
            continue;
        }
        let jsonl_bytes = line.as_bytes().len().saturating_add(1) as u64;
        if jsonl_bytes > COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES {
            if retention.at_most_once {
                return Err(io::Error::other("oversized at-most-once receipt"));
            }
            compact_after_load = true;
            continue;
        }
        let Ok(mut entry) = serde_json::from_str::<PersistentCommandResult>(&line) else {
            if retention.at_most_once {
                return Err(io::Error::other("corrupt at-most-once receipt"));
            }
            compact_after_load = true;
            continue;
        };
        if !should_persist_completed_result(&entry.result.fingerprint) {
            if retention.at_most_once {
                return Err(io::Error::other("unexpected at-most-once receipt type"));
            }
            compact_after_load = true;
            continue;
        }
        let mut completed_at_ms = persistent_result_completed_at_ms(&entry);
        if retention.at_most_once {
            if let Some(previous) = results.get(&entry.command_id) {
                let prior = persistent_result_completed_at_ms(&previous.entry);
                // All receipt timestamps must be outside retention, including
                // acceptance preceding a wall-clock regression at settlement.
                completed_at_ms = if prior == 0 || completed_at_ms == 0 {
                    0
                } else {
                    prior.max(completed_at_ms)
                };
            }
        }
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
    if retention.at_most_once {
        return false;
    }
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
        "kernel_browser"
            | "notes"
            | "credential_enrollment.interaction.request"
            | "external_provider_session.list"
            | "interaction.respond"
            | "native_provider.interaction.request"
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
    // A downgraded kernel must not treat a missing expired identity as new.
    // Preserve this fail-closed fence through every payload compaction.
    match fs::metadata(ReceiptRetention::marker_path(path)) {
        Ok(metadata) if metadata.len() != 0 => writeln!(file, "{RECEIPT_EXPIRY_FENCE}")?,
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    for entry in entries {
        serde_json::to_writer(&mut file, entry).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
    }
    file.sync_all()?;
    fs::rename(tmp_path, path)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod notes_tests;
