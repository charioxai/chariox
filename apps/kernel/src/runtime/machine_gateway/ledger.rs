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

pub(crate) struct GatewayLedger(Option<Arc<CommandResultCache>>);

fn interrupted() -> Value {
    json!({"gateway": "outcome_unknown"})
}

impl GatewayLedger {
    pub(crate) fn open(path: PathBuf) -> Self {
        match CommandResultCache::new_at_most_once(path) {
            Ok(cache) => Self(Some(Arc::new(cache))),
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.machine_gateway",
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
                "the gateway ledger is full; wait for old entries to expire".into(),
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

/// Reads never change state, so a retry may simply run them again. Everything
/// else is state-changing and goes through the ledger.
pub(crate) fn is_read_only(request: &crate::local::LocalDaemonRequest) -> bool {
    let Ok(Value::Object(tagged)) = serde_json::to_value(request) else {
        return false;
    };
    tagged.keys().next().is_some_and(|variant| {
        ["List", "Get", "Search", "Stat"]
            .iter()
            .any(|p| variant.starts_with(p))
    })
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

    #[test]
    fn only_reads_skip_the_ledger() {
        assert!(is_read_only(&LocalDaemonRequest::ListSessions(
            ListSessionsRequest
        )));
        assert!(!is_read_only(&create()));
    }
}
