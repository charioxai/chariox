//! Target side of at-most-once: a state-changing request forwarded by a gateway
//! is reserved durably BEFORE it runs and settled after. A retry of an
//! interrupted reservation is answered "outcome unknown" and never run again.
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime_transport::command_cache::{
    is_receipt_capacity_error, CommandFingerprint, CommandReservation, CommandResultCache,
};

/// A refusal or failure the request itself ended with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) retryable: bool,
}

pub(crate) enum Settled {
    Done(Result<Value, Failure>),
    /// The reservation exists without a receipt: the request may or may not have run.
    OutcomeUnknown,
    /// The command id was used for a different request.
    Conflict,
    /// The ledger is full or unreadable; nothing was run.
    Unavailable(String),
}

/// A retry of a forwarded request comes within a minute (the origin retries once, at
/// once), so a settled receipt is kept 15 minutes. The ordinary 24 hours filled the ledger
/// after 512 terminal inputs and locked an agent out of state-changing requests for a day.
const RECEIPT_WINDOW_MS: u64 = 15 * 60 * 1_000;
const LEDGER_ENTRIES: usize = 4096;

pub(crate) struct GatewayLedger(Option<Arc<CommandResultCache>>);

fn interrupted() -> Value {
    json!({"gateway": "outcome_unknown"})
}

impl GatewayLedger {
    pub(crate) fn open(path: PathBuf) -> Self {
        Self::open_expiring(path, RECEIPT_WINDOW_MS, LEDGER_ENTRIES)
    }

    fn open_expiring(path: PathBuf, receipt_window_ms: u64, max_entries: usize) -> Self {
        match CommandResultCache::new_at_most_once_expiring(path, receipt_window_ms, max_entries) {
            Ok(cache) => Self(Some(Arc::new(cache))),
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.kernel_gateway",
                    "failed to load the gateway ledger",
                    json!({"error": error.to_string()}),
                );
                Self(None)
            }
        }
    }

    pub(crate) async fn run<F>(
        &self,
        key: &str,
        fingerprint: CommandFingerprint,
        execute: F,
    ) -> Settled
    where
        F: Future<Output = Result<Value, Failure>>,
    {
        let Some(cache) = self.0.as_ref() else {
            return Settled::Unavailable("the gateway ledger could not be loaded".into());
        };
        match cache
            .reserve_at_most_once(key, &fingerprint, interrupted())
            .await
        {
            Ok(CommandReservation::Dispatch) => {
                let mut guard = Unsettled {
                    cache: Arc::clone(cache),
                    key: key.to_string(),
                    fingerprint: fingerprint.clone(),
                    armed: true,
                };
                let result = execute.await;
                // Fault injection for the crash-after-effect drill (EA2-09 e): debug builds only.
                #[cfg(debug_assertions)]
                if std::env::var_os("CHARIOX_TEST_CRASH_BEFORE_RECEIPT").is_some() {
                    std::process::abort();
                }
                guard.armed = false;
                let stored = match &result {
                    Ok(value) => json!({"ok": value}),
                    Err(failure) => json!({"err": {
                        "code": failure.code, "message": failure.message, "retryable": failure.retryable,
                    }}),
                };
                match cache
                    .complete_at_most_once(key.to_string(), fingerprint, stored, interrupted())
                    .await
                {
                    Ok(()) => Settled::Done(result),
                    // The effect ran but its receipt did not sync: never claim success.
                    Err(_) => Settled::OutcomeUnknown,
                }
            }
            Ok(CommandReservation::Wait(wait)) => match wait.await {
                Ok(cached) => decode(*cached.response_value()),
                Err(_) => Settled::OutcomeUnknown,
            },
            Ok(CommandReservation::Conflict) => Settled::Conflict,
            Err(error) if is_receipt_capacity_error(&error) => Settled::Unavailable(
                "the gateway ledger is full; its receipts expire 15 minutes after their request, so retry in a few minutes".into(),
            ),
            Err(error) => {
                Settled::Unavailable(format!("the gateway ledger refused the request: {error}"))
            }
        }
    }
}

fn decode(stored: Option<Value>) -> Settled {
    let Some(stored) = stored else {
        return Settled::OutcomeUnknown;
    };
    if let Some(value) = stored.get("ok") {
        return Settled::Done(Ok(value.clone()));
    }
    if let Some(err) = stored.get("err") {
        let text = |field| {
            err.get(field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        return Settled::Done(Err(Failure {
            code: text("code"),
            message: text("message"),
            retryable: err
                .get("retryable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }));
    }
    Settled::OutcomeUnknown
}

/// Settles a reservation as interrupted if the request future is dropped
/// mid-flight, so a retry is answered instead of waiting forever.
struct Unsettled {
    cache: Arc<CommandResultCache>,
    key: String,
    fingerprint: CommandFingerprint,
    armed: bool,
}

impl Drop for Unsettled {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (cache, key, fingerprint) = (
            Arc::clone(&self.cache),
            std::mem::take(&mut self.key),
            self.fingerprint.clone(),
        );
        tokio::spawn(async move {
            let _ = cache
                .complete_at_most_once(key, fingerprint, interrupted(), interrupted())
                .await;
        });
    }
}

/// The request's variant name, for the audit trail and the retry rule: no payload.
pub(crate) fn variant_name(request: &crate::local::LocalDaemonRequest) -> String {
    match serde_json::to_value(request) {
        Ok(Value::Object(tagged)) => tagged.keys().next().cloned().unwrap_or_default(),
        Ok(Value::String(name)) => name,
        _ => String::new(),
    }
}

/// Reads, the attachment heartbeat and a file chunk (which names its own offset)
/// may simply run again on a retry. Everything else is state-changing and goes
/// through the ledger.
pub(crate) fn is_repeatable(request: &crate::local::LocalDaemonRequest) -> bool {
    use crate::local::{LocalDaemonRequest, RoomTerminalOperation};
    // A terminal is read by listing, a snapshot, an offset or a wait; typing, opening,
    // resizing and closing change it, so a retried `input` never types twice.
    if let LocalDaemonRequest::RoomTerminal(terminal) = request {
        return matches!(
            terminal.operation,
            RoomTerminalOperation::List
                | RoomTerminalOperation::Snapshot { .. }
                | RoomTerminalOperation::ReadOutput { .. }
                | RoomTerminalOperation::Wait { .. }
                | RoomTerminalOperation::ExecList
                | RoomTerminalOperation::ExecWrite { .. }
                | RoomTerminalOperation::ExecStdinClose { .. }
                | RoomTerminalOperation::ExecRead { .. }
                | RoomTerminalOperation::ExecCancel { .. }
                | RoomTerminalOperation::ExecClose { .. }
        );
    }
    let variant = variant_name(request);
    [
        "List",
        "Get",
        "Search",
        "Stat",
        "Heartbeat",
        "ReadKernelFile",
        "WriteKernelFile",
    ]
    .iter()
    .any(|prefix| variant.starts_with(prefix))
}

/// The target's default for waiting on an access answer (`kernel_access.request_timeout_minutes`).
const SUDO_ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// How long the origin waits for the kernel's answer: a terminal wait or long read
/// is answered when it ends, so its own wait comes first.
pub(crate) fn deadline_for(request: &crate::local::LocalDaemonRequest) -> std::time::Duration {
    use crate::local::{LocalDaemonRequest, RoomTerminalOperation};
    let wait_ms = match request {
        LocalDaemonRequest::RoomTerminal(terminal) => match terminal.operation {
            RoomTerminalOperation::Wait { timeout_ms, .. } => timeout_ms,
            RoomTerminalOperation::ReadOutput { wait_ms, .. }
            | RoomTerminalOperation::ExecRead { wait_ms, .. } => wait_ms,
            _ => 0,
        },
        _ => 0,
    };
    // The terminal service caps a wait at four minutes.
    let wait = std::time::Duration::from_millis(wait_ms.min(240_000));
    match request {
        // The answer comes when the person has answered the popup there.
        LocalDaemonRequest::RequestKernelSudo(_) => super::REQUEST_DEADLINE + SUDO_ANSWER_WAIT,
        // Owner discovery, the protocol probe and the official verification turn are bounded independently.
        LocalDaemonRequest::CopyProviderAccountToKernel(_) => std::time::Duration::from_secs(150),
        LocalDaemonRequest::CopyProviderAccountFromKernel(_) => std::time::Duration::from_secs(450),
        _ => super::REQUEST_DEADLINE + wait,
    }
}

/// A request that holds the connection for a wait (a terminal wait or long read) rather than
/// answering at once.
pub(crate) fn is_long_wait(request: &crate::local::LocalDaemonRequest) -> bool {
    deadline_for(request) > super::REQUEST_DEADLINE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{
        DeleteSessionRequest, ListProjectsRequest, ListSessionsRequest, LocalDaemonRequest,
    };
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    fn fingerprint(request: &LocalDaemonRequest) -> CommandFingerprint {
        let command = crate::runtime::command::KernelCommand::from_local_request_with_caller(
            "c",
            crate::runtime::command::KernelCommandSource::RelayClient,
            crate::runtime::command::KernelCaller::for_relay_request(None),
            None,
            None,
            request,
        );
        CommandFingerprint::for_app_control(&command, request)
    }

    fn create() -> LocalDaemonRequest {
        LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: "s".into(),
            workspace_id: None,
        })
    }

    fn journal() -> PathBuf {
        let dir = PathBuf::from(std::env::var("TMPDIR").unwrap_or_else(|_| "/var/tmp".into()))
            .join(format!(
                "gateway-ledger-{}-{}",
                std::process::id(),
                crate::session::unix_epoch_ms()
            ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("ledger.jsonl")
    }

    // EA2-09(e): the side effect ran, the kernel went away before its receipt, and the
    // retry of the same command id after the restart is NOT run again.
    #[tokio::test]
    async fn an_interrupted_reservation_is_never_run_again_after_a_restart() {
        let path = journal();
        let runs = AtomicU32::new(0);
        let ledger = GatewayLedger::open(path.clone());
        let attempt = tokio::time::timeout(
            Duration::from_millis(50),
            ledger.run("k1", fingerprint(&create()), async {
                runs.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<()>().await;
                Ok(json!({}))
            }),
        )
        .await;
        assert!(attempt.is_err(), "the request never settled");
        drop(ledger);
        let ledger = GatewayLedger::open(path.clone());
        let retry = ledger
            .run("k1", fingerprint(&create()), async {
                runs.fetch_add(1, Ordering::SeqCst);
                Ok(json!({}))
            })
            .await;
        assert!(matches!(retry, Settled::OutcomeUnknown));
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "the effect ran exactly once"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    // p8 finding: 512 terminal inputs filled the ledger and every state-changing request
    // was refused for 24 hours. Settled receipts are evicted after the window; the evicted
    // id stays fenced.
    #[tokio::test]
    async fn a_full_ledger_makes_room_once_a_receipt_is_older_than_the_window() {
        let path = journal();
        let runs = AtomicU32::new(0);
        let ledger = GatewayLedger::open_expiring(path.clone(), 2_000, 3);
        let run = |id: &'static str| {
            let ledger = &ledger;
            let runs = &runs;
            async move {
                ledger
                    .run(id, fingerprint(&create()), async {
                        runs.fetch_add(1, Ordering::SeqCst);
                        Ok(json!({}))
                    })
                    .await
            }
        };
        for id in ["k1", "k2", "k3"] {
            assert!(matches!(run(id).await, Settled::Done(Ok(_))));
        }
        assert!(
            matches!(run("k4").await, Settled::Unavailable(_)),
            "inside the window nothing can be evicted"
        );
        tokio::time::sleep(Duration::from_millis(2_300)).await;
        assert!(
            matches!(run("k4").await, Settled::Done(Ok(_))),
            "an old receipt made room"
        );
        let runs_before = runs.load(Ordering::SeqCst);
        assert!(
            !matches!(run("k1").await, Settled::Done(_)),
            "an evicted id is refused, never run again"
        );
        assert_eq!(runs.load(Ordering::SeqCst), runs_before);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn a_settled_request_returns_its_original_result_and_a_new_id_runs() {
        let path = journal();
        let runs = AtomicU32::new(0);
        let ledger = GatewayLedger::open(path.clone());
        let first = ledger
            .run("k1", fingerprint(&create()), async {
                runs.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"SessionCreated": 1}))
            })
            .await;
        assert!(matches!(first, Settled::Done(Ok(ref v)) if v == &json!({"SessionCreated": 1})));
        drop(ledger);
        let ledger = GatewayLedger::open(path.clone());
        let again = ledger
            .run("k1", fingerprint(&create()), async {
                runs.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"SessionCreated": 2}))
            })
            .await;
        assert!(matches!(again, Settled::Done(Ok(ref v)) if v == &json!({"SessionCreated": 1})));
        let other = ledger
            .run("k2", fingerprint(&create()), async {
                runs.fetch_add(1, Ordering::SeqCst);
                Err(Failure {
                    code: "refused".into(),
                    message: "no".into(),
                    retryable: false,
                })
            })
            .await;
        assert!(matches!(other, Settled::Done(Err(ref f)) if f.code == "refused"));
        let different = LocalDaemonRequest::ListProjects(ListProjectsRequest {
            include_archived: true,
        });
        let conflict = ledger
            .run("k1", fingerprint(&different), async { Ok(json!({})) })
            .await;
        assert!(matches!(conflict, Settled::Conflict));
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    // EA2-09/EA2-10: an approval answered through the gateway is on the ledger
    // like any state-changing request, and the ledger still loads after a restart.
    #[tokio::test]
    async fn an_answered_approval_keeps_the_ledger_loadable_after_a_restart() {
        let path = journal();
        let answer: LocalDaemonRequest = serde_json::from_value(json!({"RespondToInteraction":
            {"session_id":"s","interaction_id":"i","choice_id":"allow"}}))
        .unwrap();
        let ledger = GatewayLedger::open(path.clone());
        let first = ledger
            .run("k1", fingerprint(&answer), async {
                Ok(json!({"InteractionResponded": 1}))
            })
            .await;
        assert!(matches!(first, Settled::Done(Ok(_))));
        drop(ledger);
        let ledger = GatewayLedger::open(path.clone());
        let next = ledger
            .run("k2", fingerprint(&create()), async {
                Ok(json!({"SessionCreated": 2}))
            })
            .await;
        assert!(
            matches!(next, Settled::Done(Ok(ref v)) if v == &json!({"SessionCreated": 2})),
            "the ledger loads after the restart"
        );
        let again = ledger
            .run("k1", fingerprint(&answer), async {
                Ok(json!({"InteractionResponded": 2}))
            })
            .await;
        assert!(
            matches!(again, Settled::Done(Ok(ref v)) if v == &json!({"InteractionResponded": 1}))
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    // EA2-09: a forwarded request that ended in a failure keeps the ledger
    // loadable; after a restart its retry gets the same failure, a new id runs.
    #[tokio::test]
    async fn a_failed_request_survives_a_restart_of_the_ledger() {
        let path = journal();
        let failed = || async {
            Err(Failure {
                code: "attachment_not_found".into(),
                message: "no attachment".into(),
                retryable: false,
            })
        };
        let ledger = GatewayLedger::open(path.clone());
        let first = ledger.run("k1", fingerprint(&create()), failed()).await;
        assert!(matches!(first, Settled::Done(Err(ref f)) if f.code == "attachment_not_found"));
        drop(ledger);
        let ledger = GatewayLedger::open(path.clone());
        let again = ledger
            .run("k1", fingerprint(&create()), async { Ok(json!({})) })
            .await;
        assert!(
            matches!(again, Settled::Done(Err(ref f)) if f.code == "attachment_not_found"),
            "the ledger reloads and answers the original failure"
        );
        let other = ledger
            .run("k2", fingerprint(&create()), async { Ok(json!({"ok": 2})) })
            .await;
        assert!(matches!(other, Settled::Done(Ok(_))));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    // Only a request that holds the connection for a wait is watched for a revoke while it waits.
    #[test]
    fn only_a_wait_counts_as_a_long_wait() {
        use crate::local::RoomTerminalOperation as Op;
        let terminal = |operation| {
            LocalDaemonRequest::RoomTerminal(crate::local::RoomTerminalRequest {
                session_id: "s".into(),
                acting_agent: None,
                host_agent: None,
                operation,
            })
        };
        assert!(!is_long_wait(&LocalDaemonRequest::ListSessions(
            ListSessionsRequest
        )));
        assert!(!is_long_wait(&terminal(Op::List)));
        assert!(is_long_wait(&terminal(Op::ReadOutput {
            terminal_id: "t".into(),
            after_offset: 0,
            wait_ms: 20_000,
        })));
        assert!(is_long_wait(&terminal(Op::Wait {
            terminal_id: "t".into(),
            watches: Vec::new(),
            after_offset: None,
            timeout_ms: 60_000,
        })));
    }

    #[test]
    fn only_reads_skip_the_ledger() {
        assert!(is_repeatable(&LocalDaemonRequest::ListSessions(
            ListSessionsRequest
        )));
        assert!(!is_repeatable(&create()));
        let terminal = |operation| {
            LocalDaemonRequest::RoomTerminal(crate::local::RoomTerminalRequest {
                session_id: "s".into(),
                acting_agent: None,
                host_agent: None,
                operation,
            })
        };
        use crate::local::RoomTerminalOperation as Op;
        for repeats in [
            Op::List,
            Op::ExecList,
            Op::Snapshot {
                terminal_id: "t".into(),
            },
            Op::ReadOutput {
                terminal_id: "t".into(),
                after_offset: 0,
                wait_ms: 0,
            },
            Op::Wait {
                terminal_id: "t".into(),
                watches: vec![],
                after_offset: None,
                timeout_ms: 1,
            },
        ] {
            assert!(is_repeatable(&terminal(repeats)));
        }
        for changes in [
            Op::Open {
                cols: 80,
                rows: 24,
                title: None,
                command: None,
            },
            Op::Input {
                terminal_id: "t".into(),
                data_base64: "bHMK".into(),
            },
            Op::Resize {
                terminal_id: "t".into(),
                cols: 80,
                rows: 24,
            },
            Op::Close {
                terminal_id: "t".into(),
            },
        ] {
            assert!(!is_repeatable(&terminal(changes)));
        }
        // A wait is given its own length on top of the usual deadline.
        assert_eq!(deadline_for(&create()), super::super::REQUEST_DEADLINE);
        assert_eq!(
            deadline_for(&terminal(Op::Wait {
                terminal_id: "t".into(),
                watches: vec![],
                after_offset: None,
                timeout_ms: 60_000
            })),
            super::super::REQUEST_DEADLINE + Duration::from_secs(60)
        );
        assert_eq!(variant_name(&create()), "DeleteSession");
        // A heartbeat and a file chunk name their own state, so a retry is safe.
        let beat =
            LocalDaemonRequest::HeartbeatAttachments(crate::local::HeartbeatAttachmentsRequest {
                attachments: Vec::new(),
            });
        assert!(is_repeatable(&beat));
        // Pairing is a person's act and never retried by the ledger's rule.
        let forget =
            LocalDaemonRequest::ForgetGatewayKernel(crate::local::ForgetGatewayKernelRequest {
                name: "b3".into(),
            });
        assert!(!is_repeatable(&forget));
    }

    // EA2-11: the answer to a forwarded sudo request is the person's, so the origin waits
    // for the popup rather than the usual 30 s, and holds the connection like a long wait.
    #[test]
    fn a_forwarded_sudo_request_waits_for_the_popup_answer() {
        let sudo = LocalDaemonRequest::RequestKernelSudo(crate::local::RequestKernelSudoRequest {
            agent_id: "a".into(),
            prompt: "p".into(),
        });
        assert!(deadline_for(&sudo) >= Duration::from_secs(10 * 60));
        assert!(is_long_wait(&sudo));
        assert!(!is_repeatable(&sudo));
    }
}
