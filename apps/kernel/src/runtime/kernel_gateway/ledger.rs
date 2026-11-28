//! Target side of at-most-once: a state-changing request forwarded by a gateway
//! is reserved durably BEFORE it runs and settled after. A retry of an
//! interrupted reservation is answered "outcome unknown" and never run again.
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime_transport::command_cache::{
    is_receipt_capacity_error, CommandFingerprint, CommandReservation, CommandResultCache,
    ReceiptStatus,
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

/// Keep the gateway's existing command namespace so an earlier reservation still fences a retry.
pub(crate) fn key(gateway_public_key: &str, command_id: &str) -> String {
    json!([gateway_public_key, command_id]).to_string()
}

impl GatewayLedger {
    pub(crate) async fn status(
        &self,
        key: &str,
        grant_id: &str,
    ) -> Result<crate::local::GatewayCommandStatus, Failure> {
        use crate::local::{GatewayCommandResult, GatewayCommandStatus};
        let cache = self.0.as_ref().ok_or_else(|| Failure {
            code: "ledger_unavailable".into(),
            message:
                "The kernel could not open the command receipts. Try reading the status again."
                    .into(),
            retryable: true,
        })?;
        Ok(match cache.receipt_status(key, grant_id).await {
            ReceiptStatus::Missing => GatewayCommandStatus::NotRun,
            ReceiptStatus::Pending => GatewayCommandStatus::Running,
            ReceiptStatus::Inaccessible => {
                return Err(Failure {
                    code: "command_receipt_unavailable".into(),
                    message: "The original receipt is unavailable to this gateway grant. Check the target Room before submitting a new command.".into(),
                    retryable: false,
                });
            }
            ReceiptStatus::Expired => GatewayCommandStatus::Expired {
                next_step: "Check the target Room before submitting a new command.".into(),
            },
            ReceiptStatus::Recorded(stored) => match decode(stored) {
                Settled::Done(Ok(value)) => GatewayCommandStatus::Ran {
                    result: GatewayCommandResult::Response(value),
                },
                Settled::Done(Err(error)) => GatewayCommandStatus::Ran {
                    result: GatewayCommandResult::Error {
                        code: error.code,
                        message: error.message,
                        retryable: error.retryable,
                    },
                },
                _ => GatewayCommandStatus::Interrupted {
                    next_step: "The effect is unknown. Check the target Room before submitting a new command.".into(),
                },
            },
        })
    }

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
                let mut result = execute.await;
                // Fault injection for the crash-after-effect drill (EA2-09 e): debug builds only.
                #[cfg(debug_assertions)]
                if std::env::var_os("CHARIOX_TEST_CRASH_BEFORE_RECEIPT").is_some() {
                    std::process::abort();
                }
                guard.armed = false;
                if let Ok(response) = &mut result {
                    super::write_receipt::compact(response);
                }
                let mut stored = stored_of(&result);
                // The result data may exceed the journal's bound even after removing Room
                // state. Keep the outcome in the receipt, so losing details never turns a
                // successful write into an apparent failure.
                if let Err(limit) = CommandResultCache::receipt_room(key, &fingerprint, &stored) {
                    result = match result {
                        Ok(response) => Ok(completed_without_details(&response)),
                        Err(mut failure) => {
                            failure.message =
                                format!("The request failed and its error details exceed the {limit}-byte receipt limit.");
                            Err(failure)
                        }
                    };
                    stored = stored_of(&result);
                }
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

fn stored_of(result: &Result<Value, Failure>) -> Value {
    match result {
        Ok(value) => json!({"ok": value}),
        Err(failure) => json!({"err": {
            "code": failure.code, "message": failure.message, "retryable": failure.retryable,
        }}),
    }
}

fn completed_without_details(response: &Value) -> Value {
    let (response_kind, payload) = response
        .as_object()
        .and_then(|tagged| tagged.iter().next())
        .expect("an oversized typed response has a payload");
    let session_id = payload
        .get("session_id")
        .or_else(|| payload.pointer("/result/session_id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let exit_code = payload
        .pointer("/result/exit_code")
        .and_then(Value::as_i64)
        .and_then(|code| i32::try_from(code).ok());
    json!(crate::local::LocalDaemonResponse::GatewayWriteCompleted {
        response_kind: response_kind.clone(),
        session_id,
        exit_code,
    })
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

/// The terminal operation a request carries, in a Room or on the kernel itself.
fn terminal_operation(
    request: &crate::local::LocalDaemonRequest,
) -> Option<&crate::local::RoomTerminalOperation> {
    use crate::local::LocalDaemonRequest;
    match request {
        LocalDaemonRequest::RoomTerminal(terminal) => Some(&terminal.operation),
        LocalDaemonRequest::KernelTerminal(terminal) => Some(&terminal.operation),
        _ => None,
    }
}

/// Reads, the attachment heartbeat and a file chunk (which names its own offset)
/// may simply run again on a retry. Everything else is state-changing and goes
/// through the ledger.
pub(crate) fn is_repeatable(request: &crate::local::LocalDaemonRequest) -> bool {
    use crate::local::RoomTerminalOperation;
    if matches!(request, crate::local::LocalDaemonRequest::ResolveSession(_)) {
        return true;
    }
    // A terminal is read by listing, a snapshot, an offset or a wait; typing, opening,
    // resizing and closing change it, so a retried `input` never types twice.
    if let Some(operation) = terminal_operation(request) {
        return matches!(
            operation,
            RoomTerminalOperation::List
                | RoomTerminalOperation::Snapshot { .. }
                | RoomTerminalOperation::ReadOutput { .. }
                | RoomTerminalOperation::Wait { .. }
        );
    }
    if let crate::local::LocalDaemonRequest::RunCommand(request) = request {
        use crate::local::RunCommandOperation as Op;
        return matches!(
            request.operation,
            Op::ExecList
                | Op::ExecRead { .. }
                | Op::ExecWrite { .. }
                | Op::ExecStdinClose { .. }
                | Op::ExecCancel { .. }
                | Op::ExecClose { .. }
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

/// Whether the target records a gateway request in its durable audit: every request
/// that changes state, and content reads of the machine's files. Status, list and
/// state polls are not recorded.
pub(crate) fn is_audited(request: &crate::local::LocalDaemonRequest) -> bool {
    use crate::local::LocalDaemonRequest;
    match request {
        LocalDaemonRequest::ReadKernelFile(_) | LocalDaemonRequest::WriteKernelFile(_) => true,
        LocalDaemonRequest::RelayStatus(_) | LocalDaemonRequest::CloudRelayStatus(_) => false,
        request => !is_repeatable(request),
    }
}

/// The target's default for waiting on an access answer (`kernel_access.request_timeout_minutes`).
const SUDO_ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// How long the origin waits for the kernel's answer: a terminal wait or long read
/// is answered when it ends, so its own wait comes first.
pub(crate) fn deadline_for(request: &crate::local::LocalDaemonRequest) -> std::time::Duration {
    use crate::local::{LocalDaemonRequest, RoomTerminalOperation};
    let wait_ms = match terminal_operation(request) {
        Some(RoomTerminalOperation::Wait { timeout_ms, .. }) => *timeout_ms,
        Some(RoomTerminalOperation::ReadOutput { wait_ms, .. }) => *wait_ms,
        _ => match request {
            LocalDaemonRequest::RunCommand(request) => match request.operation {
                crate::local::RunCommandOperation::ExecRead { wait_ms, .. } => wait_ms,
                _ => 0,
            },
            _ => 0,
        },
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

    #[tokio::test]
    async fn reconnected_steer_replays_the_first_result_and_still_conflicts_on_changed_text() {
        let dir = crate::test_support::TestTempDir::new("reconnected-steer-receipt");
        let path = dir.join("ledger.jsonl");
        let steer = |attachment: &str, text: &str| {
            LocalDaemonRequest::SteerActivePrompt(crate::local::SteerActivePromptRequest {
                session_id: "room".into(),
                attachment_id: attachment.into(),
                target_agent_id: "agent".into(),
                expected_active_prompt_id: "turn".into(),
                require_exact_turn: false,
                prompt: text.into(),
                attachments: vec![],
            })
        };
        let first = steer("before-reconnect", "change direction");
        let retry = steer("after-reconnect", "change direction");
        let ledger = GatewayLedger::open(path.clone());
        let result = json!({"PromptSteered": {"id": "first-delivery"}});
        assert!(matches!(
            ledger
                .run("same-id", fingerprint(&first), async { Ok(result.clone()) })
                .await,
            Settled::Done(Ok(_))
        ));
        for _ in 0..2 {
            assert!(
                matches!(ledger.run("same-id", fingerprint(&retry), async { panic!("retry must not execute") }).await,
                Settled::Done(Ok(value)) if value == result)
            );
        }
        drop(ledger);
        let ledger = GatewayLedger::open(path.clone());
        assert!(
            matches!(ledger.run("same-id", fingerprint(&retry), async { panic!("cold retry must not execute") }).await,
            Settled::Done(Ok(value)) if value == result)
        );
        assert!(matches!(
            ledger
                .run(
                    "same-id",
                    fingerprint(&steer("after-reconnect", "different text")),
                    async { panic!("conflict must not execute") }
                )
                .await,
            Settled::Conflict
        ));
        drop(ledger);
    }

    #[tokio::test]
    async fn a_command_status_reads_pending_and_recovers_its_original_receipt() {
        use crate::local::{GatewayCommandResult, GatewayCommandStatus};
        let path = journal();
        let ledger = Arc::new(GatewayLedger::open(path.clone()));
        let id = key("paired-key", "spawn");
        assert_eq!(
            ledger.status(&id, "grant-a").await.unwrap(),
            GatewayCommandStatus::NotRun
        );
        let (entered, running) = tokio::sync::oneshot::channel();
        let (release, waiting) = tokio::sync::oneshot::channel();
        let work = tokio::spawn({
            let ledger = Arc::clone(&ledger);
            let id = id.clone();
            async move {
                ledger
                    .run(
                        &id,
                        fingerprint(&create()).with_gateway_grant("grant-a"),
                        async {
                            entered.send(()).unwrap();
                            waiting.await.unwrap();
                            Ok(json!({"AgentSpawned": {"id": "original"}}))
                        },
                    )
                    .await
            }
        });
        running.await.unwrap();
        assert_eq!(
            ledger.status(&id, "grant-a").await.unwrap(),
            GatewayCommandStatus::Running
        );
        assert_eq!(
            ledger.status(&id, "grant-b").await.unwrap_err().code,
            "command_receipt_unavailable"
        );
        release.send(()).unwrap();
        assert!(matches!(work.await.unwrap(), Settled::Done(Ok(_))));
        drop(ledger);

        let ledger = GatewayLedger::open(path.clone());
        let expected = json!({"AgentSpawned": {"id": "original"}});
        assert_eq!(
            ledger.status(&id, "grant-a").await.unwrap(),
            GatewayCommandStatus::Ran {
                result: GatewayCommandResult::Response(expected.clone()),
            }
        );
        let replay = ledger
            .run(
                &id,
                fingerprint(&create()).with_gateway_grant("grant-a"),
                async { panic!("reading a receipt must not redispatch the command") },
            )
            .await;
        assert!(matches!(replay, Settled::Done(Ok(value)) if value == expected));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
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
            ledger.run(
                "k1",
                fingerprint(&create()).with_gateway_grant("grant-a"),
                async {
                    runs.fetch_add(1, Ordering::SeqCst);
                    std::future::pending::<()>().await;
                    Ok(json!({}))
                },
            ),
        )
        .await;
        assert!(attempt.is_err(), "the request never settled");
        drop(ledger);
        let ledger = GatewayLedger::open(path.clone());
        assert!(matches!(
            ledger.status("k1", "grant-a").await.unwrap(),
            crate::local::GatewayCommandStatus::Interrupted { .. }
        ));
        let retry = ledger
            .run(
                "k1",
                fingerprint(&create()).with_gateway_grant("grant-a"),
                async {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Ok(json!({}))
                },
            )
            .await;
        assert!(matches!(retry, Settled::OutcomeUnknown));
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "the effect ran exactly once"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn a_reservation_keeps_its_duplicate_fence_when_its_receipt_has_no_grant() {
        let path = journal();
        let ledger = GatewayLedger::open(path.clone());
        let id = key("paired-key", "older-command");
        let request = create();
        assert!(matches!(
            ledger
                .run(&id, fingerprint(&request), async { Ok(json!({})) })
                .await,
            Settled::Done(Ok(_))
        ));
        drop(ledger);
        let ledger = GatewayLedger::open(path.clone());
        assert_eq!(
            ledger.status(&id, "grant-a").await.unwrap_err().code,
            "command_receipt_unavailable"
        );
        let retry = ledger
            .run(
                &id,
                fingerprint(&request).with_gateway_grant("grant-a"),
                async { panic!("an older reservation must fence this retry") },
            )
            .await;
        assert!(matches!(retry, Settled::Conflict));
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

    // A 300 KB answer does not fit its receipt record. Preserve the write's successful
    // outcome across a restart without keeping its large response.
    #[tokio::test]
    async fn a_large_result_keeps_its_successful_receipt_without_the_details() {
        let path = journal();
        let runs = AtomicU32::new(0);
        let answer = |size: usize| {
            json!({"ShellCommandCompleted": {"result": {
                "session_id": "room", "exit_code": 17, "stdout": "x".repeat(size)
            }}})
        };
        let run = |size| {
            let runs = &runs;
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                Ok(answer(size))
            }
        };
        let expected = json!({"GatewayWriteCompleted": {
            "response_kind": "ShellCommandCompleted", "session_id": "room", "exit_code": 17,
        }});
        let ledger = GatewayLedger::open(path.clone());
        let small = ledger
            .run("small", fingerprint(&create()), run(120_000))
            .await;
        assert!(matches!(small, Settled::Done(Ok(value)) if value == answer(120_000)));
        let large = ledger
            .run("big", fingerprint(&create()), run(300_000))
            .await;
        assert!(matches!(large, Settled::Done(Ok(value)) if value == expected));
        drop(ledger);
        let ledger = GatewayLedger::open(path.clone());
        let replay = ledger
            .run("big", fingerprint(&create()), run(300_000))
            .await;
        assert!(matches!(replay, Settled::Done(Ok(value)) if value == expected));
        assert_eq!(runs.load(Ordering::SeqCst), 2, "each id ran exactly once");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn a_large_error_keeps_its_failure_code_on_replay() {
        let path = journal();
        let ledger = GatewayLedger::open(path.clone());
        let failed = async {
            Err(Failure {
                code: "fixture_failure".into(),
                message: "x".repeat(300_000),
                retryable: false,
            })
        };
        let result = ledger.run("failed", fingerprint(&create()), failed).await;
        let Settled::Done(Err(failure)) = result else {
            panic!("an oversized error must retain its failure outcome");
        };
        assert_eq!(failure.code, "fixture_failure");
        assert!(!failure.retryable);
        assert!(failure.message.contains("receipt limit"));
        let replay = ledger
            .run("failed", fingerprint(&create()), async {
                panic!("a completed failed write must not run again");
            })
            .await;
        assert!(matches!(replay, Settled::Done(Err(cached)) if cached == failure));
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
                stdin: None,
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

    #[test]
    fn command_start_returns_without_waiting_for_its_deadline() {
        use crate::local::{CommandTarget, RunCommandOperation as Op, RunCommandRequest};
        let command = |operation| {
            LocalDaemonRequest::RunCommand(RunCommandRequest {
                target: CommandTarget::Kernel,
                acting_agent: None,
                operation,
            })
        };
        let opening = command(Op::ExecOpen {
            command: vec!["sleep".into(), "400".into()],
            directory: None,
            timeout_ms: Some(400_000),
            stdin_file: None,
        });
        assert_eq!(deadline_for(&opening), super::super::REQUEST_DEADLINE);
        assert!(!is_long_wait(&opening));
        assert!(!is_repeatable(&opening));
        for operation in [
            Op::ExecList,
            Op::ExecRead {
                exec_id: "e".into(),
                after_cursor: 0,
                wait_ms: 0,
            },
            Op::ExecWrite {
                exec_id: "e".into(),
                offset: 0,
                data_base64: "YQ==".into(),
            },
            Op::ExecStdinClose {
                exec_id: "e".into(),
                offset: 0,
            },
            Op::ExecCancel {
                exec_id: "e".into(),
            },
            Op::ExecClose {
                exec_id: "e".into(),
            },
        ] {
            assert!(is_repeatable(&command(operation)));
        }
        let read = command(Op::ExecRead {
            exec_id: "e".into(),
            after_cursor: 0,
            wait_ms: 400_000,
        });
        assert_eq!(
            deadline_for(&read),
            super::super::REQUEST_DEADLINE + Duration::from_secs(240)
        );
        assert!(is_long_wait(&read));
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
