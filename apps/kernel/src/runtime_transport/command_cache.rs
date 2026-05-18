use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, VecDeque};
use std::hash::{Hash, Hasher};

use serde_json::Value;
use tokio::sync::{oneshot, Mutex};

use crate::local::LocalDaemonRequest;
use crate::runtime::command::KernelCommand;
use crate::transport::kernel_protocol::{KernelOutgoingFrame, KernelTransportError};

pub(crate) const COMMAND_RESULT_CACHE_LIMIT: usize = 512;

#[derive(Debug, Clone)]
pub(super) struct CachedCommandResult {
    pub(super) response: Box<Option<Value>>,
    pub(super) error: Option<KernelTransportError>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CommandFingerprint {
    command_type: String,
    source: String,
    session_id: Option<String>,
    attachment_id: Option<String>,
    request_hash: u64,
}

impl CommandFingerprint {
    pub(super) fn from_command_and_request(
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Self {
        let mut hasher = DefaultHasher::new();
        serde_json::to_vec(request)
            .unwrap_or_default()
            .hash(&mut hasher);
        Self {
            command_type: command.command_type.clone(),
            source: serde_json::to_string(&command.source)
                .unwrap_or_else(|_| "unknown".to_string()),
            session_id: command.session_id.clone(),
            attachment_id: command.attachment_id.clone(),
            request_hash: hasher.finish(),
        }
    }
}

pub(super) enum CommandReservation {
    Dispatch,
    Wait(oneshot::Receiver<CachedCommandResult>),
    Conflict,
}

#[derive(Debug, Default)]
pub(super) struct CommandResultCache {
    results: Mutex<BTreeMap<String, CommandResultEntry>>,
    order: Mutex<VecDeque<String>>,
}

impl CommandResultCache {
    pub(super) async fn reserve(
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

    pub(super) async fn complete(
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
        let mut order = self.order.lock().await;
        order.push_back(command_id);
        while order.len() > COMMAND_RESULT_CACHE_LIMIT {
            if let Some(expired) = order.pop_front() {
                self.results.lock().await.remove(&expired);
            }
        }
    }

    pub(super) async fn forget_pending(&self, command_id: &str) {
        let mut results = self.results.lock().await;
        if matches!(
            results.get(command_id),
            Some(CommandResultEntry::Pending { .. })
        ) {
            results.remove(command_id);
        }
    }
}
