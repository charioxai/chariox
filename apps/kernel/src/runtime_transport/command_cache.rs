use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{oneshot, Mutex};

use crate::local::LocalDaemonRequest;
use crate::runtime::command::KernelCommand;
use crate::transport::kernel_protocol::{KernelOutgoingFrame, KernelTransportError};

pub(crate) const COMMAND_RESULT_CACHE_LIMIT: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedCommandResult {
    pub(crate) response: Box<Option<Value>>,
    pub(crate) error: Option<KernelTransportError>,
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

#[derive(Debug, Serialize, Deserialize)]
struct PersistentCommandResult {
    command_id: String,
    result: CachedCommandResult,
}

#[derive(Debug)]
struct CommandResultPersistence {
    path: PathBuf,
    io_lock: Mutex<()>,
}

#[derive(Debug, Default)]
pub(crate) struct CommandResultCache {
    results: Mutex<BTreeMap<String, CommandResultEntry>>,
    order: Mutex<VecDeque<String>>,
    persistence: Option<CommandResultPersistence>,
}

impl CommandResultCache {
    pub(crate) fn new_with_persistent_path(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let mut cache = Self {
            results: Mutex::new(BTreeMap::new()),
            order: Mutex::new(VecDeque::new()),
            persistence: Some(CommandResultPersistence {
                path: path.clone(),
                io_lock: Mutex::new(()),
            }),
        };
        let retained = read_persistent_results(&path)?;
        let start = retained.len().saturating_sub(COMMAND_RESULT_CACHE_LIMIT);
        let mut results = BTreeMap::new();
        let mut order = VecDeque::new();
        for entry in retained.into_iter().skip(start) {
            order.push_back(entry.command_id.clone());
            results.insert(
                entry.command_id,
                CommandResultEntry::Completed(entry.result),
            );
        }
        cache.results = Mutex::new(results);
        cache.order = Mutex::new(order);
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
        let evicted = {
            let mut order = self.order.lock().await;
            if let Some(existing_index) = order.iter().position(|entry| entry == &command_id) {
                order.remove(existing_index);
            }
            order.push_back(command_id.clone());
            let mut evicted = Vec::new();
            while order.len() > COMMAND_RESULT_CACHE_LIMIT {
                if let Some(expired) = order.pop_front() {
                    evicted.push(expired);
                }
            }
            evicted
        };
        let compact_after_append = !evicted.is_empty();
        if compact_after_append {
            let mut results = self.results.lock().await;
            for expired in evicted {
                results.remove(&expired);
            }
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
        let _guard = persistence.io_lock.lock().await;
        if let Some(parent) = persistence.path.parent() {
            fs::create_dir_all(parent)?;
        }
        append_persistent_result(
            &persistence.path,
            &PersistentCommandResult {
                command_id,
                result: cached,
            },
        )?;
        if compact_after_append {
            let retained = self.completed_results_snapshot().await;
            rewrite_persistent_results(&persistence.path, &retained)?;
        }
        Ok(())
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

fn stable_hash64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    bytes.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

fn read_persistent_results(path: &PathBuf) -> io::Result<Vec<PersistentCommandResult>> {
    let payload = match fs::read_to_string(path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut results = BTreeMap::<String, PersistentCommandResult>::new();
    let mut order = VecDeque::<String>::new();
    for line in payload.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<PersistentCommandResult>(line) else {
            continue;
        };
        if let Some(existing_index) = order
            .iter()
            .position(|command_id| command_id == &entry.command_id)
        {
            order.remove(existing_index);
        }
        order.push_back(entry.command_id.clone());
        results.insert(entry.command_id.clone(), entry);
    }
    Ok(order
        .into_iter()
        .filter_map(|command_id| results.remove(&command_id))
        .collect())
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
}
