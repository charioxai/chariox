use super::*;
use crate::local::{AppWorkerAction, ControlAppWorkerRequest, UninstallAppRequest};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Journal(PathBuf);
impl Journal {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "chariox-app-receipts-{:016x}.jsonl",
            rand::random::<u64>()
        )))
    }
}
impl Drop for Journal {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("expired"));
    }
}
fn restart() -> LocalDaemonRequest {
    LocalDaemonRequest::ControlAppWorker(ControlAppWorkerRequest {
        installation_id: "app".into(),
        action: AppWorkerAction::Restart,
    })
}
fn uninstall() -> LocalDaemonRequest {
    LocalDaemonRequest::UninstallApp(UninstallAppRequest {
        installation_id: "app".into(),
        expected_generation: "1".into(),
        delete_data: false,
    })
}
fn command(id: &str, request: &LocalDaemonRequest) -> KernelCommand {
    let mut command = KernelCommand::from_local_request(id, None, None, request);
    command.caller.user_id = Some("alice".into());
    command
}
fn answer() -> LocalDaemonResponse {
    LocalDaemonResponse::AppWorker {
        worker: crate::local::AppWorkerSummary {
            installation_id: "app".into(),
            phase: crate::local::AppWorkerPhase::Starting,
            enabled: true,
            failure: None,
            updated_at_ms: Some(42),
        },
    }
}

async fn duplicate_once(request: LocalDaemonRequest) {
    let journal = Journal::new();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let executions = Arc::new(AtomicUsize::new(0));
    let input = command("one", &request);
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
    let count = executions.clone();
    let first = receipts.execute("alice", &input, &request, move || async move {
        count.fetch_add(1, Ordering::SeqCst);
        accepted_tx.send(()).unwrap();
        finish_rx.await.unwrap();
        answer()
    });
    let duplicate = async {
        accepted_rx.await.unwrap();
        let replay = receipts.execute("alice", &input, &request, || async {
            panic!("duplicate executed")
        });
        // Poll the duplicate while the first execution is held open.
        let finish = async {
            tokio::task::yield_now().await;
            finish_tx.send(()).unwrap();
        };
        tokio::join!(replay, finish).0
    };
    let (first, second) = tokio::join!(first, duplicate);
    assert_eq!(first, second);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert_eq!(
        receipts
            .execute("alice", &input, &request, || async {
                panic!("completed duplicate executed")
            })
            .await,
        first
    );
    let restored = AppRequestReceipts::new(journal.0.clone());
    assert_eq!(
        restored
            .execute("alice", &input, &request, || async {
                panic!("restored duplicate executed")
            })
            .await,
        first
    );
    let count = executions.clone();
    let fresh = command("two", &request);
    assert_eq!(
        restored
            .execute("alice", &fresh, &request, move || async move {
                count.fetch_add(1, Ordering::SeqCst);
                answer()
            })
            .await,
        answer()
    );
    assert_eq!(executions.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn duplicate_restart_executes_once_and_new_identity_executes() {
    duplicate_once(restart()).await;
}
#[tokio::test]
async fn duplicate_uninstall_returns_same_result_and_new_identity_executes() {
    duplicate_once(uninstall()).await;
}

#[tokio::test]
async fn crash_after_accept_before_execute_never_redispatches() {
    let journal = Journal::new();
    let request = restart();
    let accepted_command = command("accepted", &request);
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let key = serde_json::to_string(&("alice", &accepted_command.command_id)).unwrap();
    let interrupted = LocalDaemonResponse::AppRequestFailed {
        code: AppRequestErrorCode::StorageUnavailable,
    };
    assert!(matches!(
        receipts
            .0
            .as_ref()
            .unwrap()
            .reserve_at_most_once(
                &key,
                &CommandFingerprint::for_app_control(&accepted_command, &request),
                serde_json::to_value(&interrupted).unwrap()
            )
            .await
            .unwrap(),
        CommandReservation::Dispatch
    ));
    drop(receipts); // no execution and no completion record
    let restored = AppRequestReceipts::new(journal.0.clone());
    assert_eq!(
        restored
            .execute("alice", &accepted_command, &request, || async {
                panic!("crashed acceptance executed again")
            })
            .await,
        interrupted
    );
    let new = command("new", &request);
    assert_eq!(
        restored
            .execute("alice", &new, &request, || async { answer() })
            .await,
        answer()
    );
}

#[tokio::test]
async fn caller_or_input_reuse_conflicts_and_owners_are_isolated() {
    let journal = Journal::new();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let request = restart();
    let first = command("same", &request);
    assert_eq!(
        receipts
            .execute("alice", &first, &request, || async { answer() })
            .await,
        answer()
    );
    let changed = uninstall();
    let changed_command = command("same", &changed);
    let conflict = LocalDaemonResponse::AppRequestFailed {
        code: AppRequestErrorCode::Conflict,
    };
    assert_eq!(
        receipts
            .execute("alice", &changed_command, &changed, || async {
                panic!("different input executed")
            })
            .await,
        conflict
    );
    let mut changed_caller = first.clone();
    changed_caller.caller.client_id = Some("other-client".into());
    assert_eq!(
        receipts
            .execute("alice", &changed_caller, &request, || async {
                panic!("different caller got receipt")
            })
            .await,
        conflict
    );
    changed_caller.caller.user_id = Some("bob".into());
    assert_eq!(
        receipts
            .execute("bob", &changed_caller, &request, || async { answer() })
            .await,
        answer()
    );
}

#[tokio::test]
async fn cancelled_caller_does_not_cancel_execution_or_settlement() {
    let journal = Journal::new();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let request = restart();
    let input = command("cancelled", &request);
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
    let task_receipts = receipts.clone();
    let task_request = request.clone();
    let task_input = input.clone();
    let task = tokio::spawn(async move {
        task_receipts
            .execute("alice", &task_input, &task_request, move || async move {
                accepted_tx.send(()).unwrap();
                finish_rx.await.unwrap();
                answer()
            })
            .await
    });
    accepted_rx.await.unwrap();
    task.abort();
    finish_tx.send(()).unwrap();
    assert_eq!(
        receipts
            .execute("alice", &input, &request, || async {
                panic!("cancelled request executed twice")
            })
            .await,
        answer()
    );
}

#[tokio::test]
async fn corrupt_or_unwritable_journal_fails_closed() {
    let journal = Journal::new();
    std::fs::write(&journal.0, "not json\n").unwrap();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let request = restart();
    let input = command("one", &request);
    let unavailable = LocalDaemonResponse::AppRequestFailed {
        code: AppRequestErrorCode::StorageUnavailable,
    };
    assert_eq!(
        receipts
            .execute("alice", &input, &request, || async {
                panic!("corrupt receipts dispatched")
            })
            .await,
        unavailable
    );
    let blocked = Journal::new();
    std::fs::write(&blocked.0, "blocks parent directory").unwrap();
    let receipts = AppRequestReceipts::new(blocked.0.join("receipts"));
    assert_eq!(
        receipts
            .execute("alice", &input, &request, || async {
                panic!("unwritable receipts dispatched")
            })
            .await,
        unavailable
    );
}

#[tokio::test]
async fn panicked_execution_settles_duplicates_without_redispatch() {
    let journal = Journal::new();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let request = restart();
    let input = command("panic", &request);
    let unavailable = LocalDaemonResponse::AppRequestFailed {
        code: AppRequestErrorCode::StorageUnavailable,
    };
    assert_eq!(
        receipts
            .execute("alice", &input, &request, || async {
                panic!("simulated execution failure")
            })
            .await,
        unavailable
    );
    assert_eq!(
        receipts
            .execute("alice", &input, &request, || async {
                panic!("failed execution was dispatched again")
            })
            .await,
        unavailable
    );
}

#[tokio::test]
async fn successful_effect_with_failed_append_reports_unknown_and_never_reexecutes() {
    let journal = Journal::new();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let request = restart();
    let input = command("failed-settlement", &request);
    let accepted_path = journal.0.with_extension("accepted.jsonl");
    let journal_path = journal.0.clone();
    let task_backup = accepted_path.clone();
    let executions = Arc::new(AtomicUsize::new(0));
    let count = executions.clone();
    let unavailable = LocalDaemonResponse::AppRequestFailed {
        code: AppRequestErrorCode::StorageUnavailable,
    };
    let result = receipts
        .execute("alice", &input, &request, move || async move {
            count.fetch_add(1, Ordering::SeqCst);
            // Acceptance has succeeded. Make final append fail after the effect.
            std::fs::rename(&journal_path, &task_backup).unwrap();
            std::fs::create_dir(&journal_path).unwrap();
            answer()
        })
        .await;
    // Restore the durable acceptance journal before assertions and recovery.
    std::fs::remove_dir(&journal.0).unwrap();
    std::fs::rename(&accepted_path, &journal.0).unwrap();
    assert_eq!(result, unavailable);
    assert_eq!(
        receipts
            .execute("alice", &input, &request, || async {
                panic!("failed settlement executed twice")
            })
            .await,
        unavailable
    );
    assert_eq!(
        AppRequestReceipts::new(journal.0.clone())
            .execute("alice", &input, &request, || async {
                panic!("failed settlement executed after recovery")
            })
            .await,
        unavailable
    );
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn full_receipt_journal_keeps_effects_at_most_once_after_recovery() {
    let journal = Journal::new();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let request = restart();
    for n in 0..crate::runtime_transport::COMMAND_RESULT_CACHE_LIMIT {
        let input = command(&format!("fill-{n}"), &request);
        assert_eq!(
            receipts
                .execute("alice", &input, &request, || async { answer() })
                .await,
            answer()
        );
    }
    use sha2::{Digest, Sha256};
    for recovered in [false, true] {
        let receipts = if recovered {
            AppRequestReceipts::new(journal.0.clone())
        } else {
            receipts.clone()
        };
        // Recovery compacts reservation/result pairs. Safety controls must
        // leave the loaded journal unchanged, including after that compaction.
        let old = command("fill-0", &request);
        assert_eq!(
            receipts
                .execute("alice", &old, &request, || async {
                    panic!("full journal re-executed a completed effect")
                })
                .await,
            answer()
        );
        let before = Sha256::digest(std::fs::read(&journal.0).unwrap());
        let new = command("new", &request);
        assert_eq!(
            receipts
                .execute("alice", &new, &request, || async {
                    panic!("full journal admitted a new restart")
                })
                .await,
            LocalDaemonResponse::AppRequestFailed {
                code: AppRequestErrorCode::LimitExceeded
            }
        );
        let mut start = request.clone();
        if let LocalDaemonRequest::ControlAppWorker(input) = &mut start {
            input.action = AppWorkerAction::Start;
        }
        let new = command("new-start", &start);
        assert_eq!(
            receipts
                .execute("alice", &new, &start, || async {
                    panic!("full journal admitted a new start")
                })
                .await,
            LocalDaemonResponse::AppRequestFailed {
                code: AppRequestErrorCode::LimitExceeded
            }
        );
        let stop = LocalDaemonRequest::ControlAppWorker(ControlAppWorkerRequest {
            installation_id: "app".into(),
            action: AppWorkerAction::Stop,
        });
        let conflict = command("fill-0", &stop);
        assert_eq!(
            receipts
                .execute("alice", &conflict, &stop, || async {
                    panic!("safety control bypassed an existing receipt conflict")
                })
                .await,
            LocalDaemonResponse::AppRequestFailed {
                code: AppRequestErrorCode::Conflict
            }
        );
        // More safety operations than a second finite lane could hold.
        for n in 0..=crate::runtime_transport::COMMAND_RESULT_CACHE_LIMIT {
            let input = command(&format!("stop-{n}"), &stop);
            assert_eq!(
                receipts
                    .execute("alice", &input, &stop, || async { answer() })
                    .await,
                answer()
            );
        }
        assert_eq!(Sha256::digest(std::fs::read(&journal.0).unwrap()), before);
    }
}

#[tokio::test]
async fn identity_513_evicts_old_lru_and_refuses_every_control_replay_after_restart() {
    let journal = Journal::new();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let request = restart();
    for n in 0..crate::runtime_transport::COMMAND_RESULT_CACHE_LIMIT {
        let input = command(&format!("retained-{n}"), &request);
        assert_eq!(
            receipts
                .execute("alice", &input, &request, || async { answer() })
                .await,
            answer()
        );
    }
    drop(receipts);
    // Both acceptance and settlement are outside the existing 24-hour window.
    let age = crate::session::unix_epoch_ms()
        - crate::runtime_transport::command_cache::APP_RECEIPT_RETENTION_MS
        - 60_000;
    let records = std::fs::read_to_string(&journal.0)
        .unwrap()
        .lines()
        .map(|line| {
            let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
            value["completed_at_ms"] = age.into();
            value["result"]["completed_at_ms"] = age.into();
            serde_json::to_string(&value).unwrap() + "\n"
        })
        .collect::<String>();
    std::fs::write(&journal.0, records).unwrap();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let touch = command("retained-0", &request);
    assert_eq!(
        receipts
            .execute("alice", &touch, &request, || async {
                panic!("touch re-executed")
            })
            .await,
        answer()
    );
    drop(receipts); // LRU access order itself must survive restart.
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let executions = Arc::new(AtomicUsize::new(0));
    let counter = executions.clone();
    let new = command("identity-513", &request);
    assert_eq!(
        receipts
            .execute("alice", &new, &request, move || async move {
                counter.fetch_add(1, Ordering::SeqCst);
                answer()
            })
            .await,
        answer()
    );
    drop(receipts);
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let expected = LocalDaemonResponse::AppRequestFailed {
        code: AppRequestErrorCode::ReceiptExpired,
    };
    for source in [
        crate::runtime::command::KernelCommandSource::LocalCli,
        crate::runtime::command::KernelCommandSource::LocalIpc,
        crate::runtime::command::KernelCommandSource::RelayClient,
    ] {
        for request in [
            restart(),
            uninstall(),
            LocalDaemonRequest::ControlAppWorker(ControlAppWorkerRequest {
                installation_id: "app".into(),
                action: AppWorkerAction::Stop,
            }),
        ] {
            let mut input = command("retained-1", &request);
            input.source = source.clone();
            assert_eq!(
                receipts
                    .execute("alice", &input, &request, || async {
                        panic!("expired identity re-executed, including a safety control")
                    })
                    .await,
                expected
            );
        }
    }
    assert!(receipts.has_reserved("alice", "retained-1").await);
    assert_eq!(
        serde_json::to_value(&expected).unwrap(),
        serde_json::json!({"AppRequestFailed": {"code": "receipt_expired"}})
    );
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert_eq!(
        receipts
            .execute("alice", &touch, &restart(), || async {
                panic!("retained identity re-executed")
            })
            .await,
        answer()
    );
}

#[tokio::test]
async fn marker_capacity_preserves_fresh_stop_and_uninstall_controls() {
    let journal = Journal::new();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let request = restart();
    for n in 0..crate::runtime_transport::COMMAND_RESULT_CACHE_LIMIT {
        let input = command(&format!("bounded-{n}"), &request);
        assert_eq!(
            receipts
                .execute("alice", &input, &request, || async { answer() })
                .await,
            answer()
        );
    }
    drop(receipts);
    let age = crate::session::unix_epoch_ms()
        - crate::runtime_transport::command_cache::APP_RECEIPT_RETENTION_MS
        - 60_000;
    let records = std::fs::read_to_string(&journal.0)
        .unwrap()
        .lines()
        .map(|line| {
            let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
            value["completed_at_ms"] = age.into();
            value["result"]["completed_at_ms"] = age.into();
            serde_json::to_string(&value).unwrap() + "\n"
        })
        .collect::<String>();
    std::fs::write(&journal.0, records).unwrap();
    let receipts = AppRequestReceipts::new(journal.0.clone());
    let path = journal.0.with_extension("expired");
    // Inject capacity after loading so this sparse fixture needs no 800k hashes.
    std::fs::File::create(&path)
        .unwrap()
        .set_len(50 * 1024 * 1024)
        .unwrap();
    let before = std::fs::read(&journal.0).unwrap();
    let input = command("new-restart", &request);
    assert_eq!(
        receipts
            .execute("alice", &input, &request, || async {
                panic!("capacity dispatched restart")
            })
            .await,
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::LimitExceeded
        }
    );
    let stop = LocalDaemonRequest::ControlAppWorker(ControlAppWorkerRequest {
        installation_id: "app".into(),
        action: AppWorkerAction::Stop,
    });
    for control in [stop, uninstall()] {
        let input = command("fresh-safety", &control);
        assert_eq!(
            receipts
                .execute("alice", &input, &control, || async { answer() })
                .await,
            answer()
        );
    }
    assert_eq!(std::fs::read(&journal.0).unwrap(), before);
    assert_eq!(std::fs::metadata(path).unwrap().len(), 50 * 1024 * 1024);
}
