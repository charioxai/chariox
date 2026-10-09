use super::*;
use crate::durable_state::apps::AppRegistryMutation;
use crate::local::{AppInstallationRequest, ListAppInstallationsRequest, LocalDaemonResponse};
use chariox_app_runtime::installation::ReleaseMetadata;

#[tokio::test]
async fn request_decode_refusal_uses_existing_relay_message_field() {
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let receiver =
        relay_crypto::public_key_from_private_key_base64(&router.relay_private_key()).unwrap();
    let sender = relay_crypto::generate_private_key_base64();
    let cache = Arc::new(CommandResultCache::default());
    for payload in [
        br#"{"FutureRequest":{}}"#.as_slice(),
        br#"{"AcceptAppHostAction":{"session_id":"s","operation_id":17}}"#.as_slice(),
    ] {
        let encrypted =
            relay_crypto::encrypt_payload_for_peer(&sender, &receiver, payload).unwrap();
        let outcome = handle_daemon_request(
            &router,
            &AtomicU64::new(1),
            Some(caller("alice")),
            encrypted,
            &cache,
            &Default::default(),
        )
        .await;
        assert!(outcome.encrypted_response.is_none());
        let error = outcome.error.unwrap();
        assert_eq!(error.code, "invalid_request");
        assert!(!error.retryable);
        assert!(error.message.contains(&format!("This kernel (protocol {}) does not support this request; update the Chariox client or kernel so both match.", crate::local::LOCAL_DAEMON_PROTOCOL_VERSION)));
    }
}

struct TestRoot(std::path::PathBuf);

impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-app-relay-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(std::fs::canonicalize(path).unwrap())
    }

    fn config(&self) -> crate::DaemonConfig {
        let mut config = crate::DaemonConfig::for_tests();
        config.user_config.state.path = Some(self.0.join("state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(self.0.join("history.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(self.0.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(self.0.join("artifacts.db").display().to_string());
        config.with_session_history_root(self.0.join("sessions"))
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn caller(owner: &str) -> RelayCallerIdentity {
    RelayCallerIdentity {
        realm_id: "test".into(),
        subject: format!("client-{owner}"),
        subject_kind: chariox_relay::auth::RelaySubjectKind::Client,
        expires_at_ms: u64::MAX,
        token_id: None,
        user_id: Some(owner.into()),
        public_key_thumbprint: None,
    }
}

async fn dispatch(
    router: &CommandRouter,
    cache: &CommandResultCache,
    owner: Option<&str>,
    request: LocalDaemonRequest,
    command_id: &str,
) -> LocalDaemonResponse {
    let result = dispatch_relay_client_request(
        router,
        &AtomicU64::new(1),
        owner.map(caller),
        request,
        Some(command_id.into()),
        cache,
    )
    .await;
    match result {
        RelayDispatchOutcome::Response(value) => serde_json::from_value(value).unwrap(),
        RelayDispatchOutcome::RelayError(_) => panic!("unexpected relay transport error"),
    }
}

#[tokio::test]
async fn app_relay_replays_reauthorize_the_owner_and_read_current_installations() {
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    let store = app.durable_state_store();
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    let list = LocalDaemonRequest::ListAppInstallations(ListAppInstallationsRequest {
        after: None,
        limit: None,
    });
    let empty = LocalDaemonResponse::AppInstallationsListed {
        installations: vec![],
        next_cursor: None,
    };
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), list.clone(), "list").await,
        empty
    );
    store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::CreateAndStage {
                installation_id: "todo-alice".into(),
                release: ReleaseMetadata {
                    app_id: "com.chariox.todo".into(),
                    version: "1.0.0".into(),
                    publisher_id: "publisher".into(),
                    package_digest: format!("sha256:{:064x}", 1),
                    schema_version: 1,
                    capabilities_digest: format!("sha256:{:064x}", 2),
                    catalog_digest: format!("sha256:{:064x}", 3),
                    view_digest: format!("sha256:{:064x}", 4),
                },
                now_ms: 1,
            },
        )
        .unwrap();
    let current = dispatch(&router, &cache, Some("alice"), list.clone(), "list").await;
    assert!(
        matches!(current, LocalDaemonResponse::AppInstallationsListed { installations, .. } if installations.len() == 1)
    );
    // Same command ID/body reaches authorization, even after Alice's success.
    assert_eq!(
        dispatch(&router, &cache, Some("bob"), list.clone(), "list").await,
        empty
    );
    assert!(matches!(
        dispatch(&router, &cache, None, list, "list").await,
        LocalDaemonResponse::AppRequestFailed {
            code: crate::local::AppRequestErrorCode::Unauthorized
        }
    ));
    for request in [
        LocalDaemonRequest::GetAppInstallation(AppInstallationRequest {
            installation_id: "todo-alice".into(),
        }),
        LocalDaemonRequest::GetAppInstallationJournal(AppInstallationRequest {
            installation_id: "todo-alice".into(),
        }),
    ] {
        let own = dispatch(&router, &cache, Some("alice"), request.clone(), "same-id").await;
        assert!(!matches!(own, LocalDaemonResponse::AppRequestFailed { .. }));
        assert!(matches!(
            dispatch(&router, &cache, Some("bob"), request, "same-id").await,
            LocalDaemonResponse::AppRequestFailed {
                code: crate::local::AppRequestErrorCode::NotFound
            }
        ));
    }
}

#[tokio::test]
async fn app_relay_upload_replays_preserve_owner_offset_and_abort_receipt() {
    use crate::local::{
        AppPackageUploadPhase, AppPackageUploadRequest, BeginAppPackageUploadRequest,
        PutAppPackageUploadChunkRequest,
    };
    use sha2::{Digest, Sha256};
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    let digest = format!("sha256:{:x}", Sha256::digest(b"test"));
    let begin = LocalDaemonRequest::BeginAppPackageUpload(BeginAppPackageUploadRequest {
        request_id: "same-retry-id".into(),
        expected_size: 4,
        sha256: digest.clone(),
    });
    let LocalDaemonResponse::AppPackageUploadStatus { upload: alice } =
        dispatch(&router, &cache, Some("alice"), begin.clone(), "begin").await
    else {
        panic!("upload should begin")
    };
    let LocalDaemonResponse::AppPackageUploadStatus { upload: bob } =
        dispatch(&router, &cache, Some("bob"), begin.clone(), "begin").await
    else {
        panic!("Bob has a separate upload")
    };
    assert_ne!(alice.handle, bob.handle);
    let status = LocalDaemonRequest::GetAppPackageUpload(AppPackageUploadRequest {
        handle: alice.handle.clone(),
    });
    let abort = LocalDaemonRequest::AbortAppPackageUpload(AppPackageUploadRequest {
        handle: alice.handle.clone(),
    });
    let chunk = LocalDaemonRequest::PutAppPackageUploadChunk(PutAppPackageUploadChunkRequest {
        handle: alice.handle.clone(),
        offset: 0,
        data_base64: "dGVzdA==".into(),
        chunk_sha256: digest,
    });
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), status.clone(), "status").await,
        LocalDaemonResponse::AppPackageUploadStatus {
            upload: alice.clone()
        }
    );
    for request in [status.clone(), chunk.clone(), abort.clone()] {
        assert_eq!(
            dispatch(&router, &cache, Some("bob"), request, "foreign").await,
            LocalDaemonResponse::AppRequestFailed {
                code: crate::local::AppRequestErrorCode::NotFound
            }
        );
    }
    assert_eq!(
        dispatch(&router, &cache, None, begin.clone(), "begin").await,
        LocalDaemonResponse::AppRequestFailed {
            code: crate::local::AppRequestErrorCode::Unauthorized
        }
    );
    let mut complete = alice.clone();
    complete.accepted_bytes = 4;
    let expected = LocalDaemonResponse::AppPackageUploadStatus {
        upload: complete.clone(),
    };
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), chunk.clone(), "chunk").await,
        expected
    );
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), chunk, "chunk").await,
        expected
    );
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), status.clone(), "status").await,
        expected
    );
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), begin.clone(), "begin").await,
        expected
    );
    // The same local command path observes the bytes accepted through RelayClient.
    let mut local_caller = KernelCaller::for_source(&KernelCommandSource::LocalIpc);
    local_caller.user_id = Some("alice".into());
    let command = KernelCommand::from_local_request_with_caller(
        "local-status",
        KernelCommandSource::LocalIpc,
        local_caller,
        None,
        None,
        &status,
    );
    assert_eq!(router.dispatch(command, status).await.unwrap(), expected);
    complete.phase = AppPackageUploadPhase::Aborted;
    let aborted = dispatch(&router, &cache, Some("alice"), abort, "abort").await;
    let LocalDaemonResponse::AppPackageUploadStatus { upload } = &aborted else {
        panic!("abort should answer the upload status: {aborted:?}")
    };
    // The aborted receipt answers retries for a minute, not the upload's TTL.
    assert!(upload.expires_at_ms < complete.expires_at_ms);
    complete.expires_at_ms = upload.expires_at_ms;
    assert_eq!(
        aborted,
        LocalDaemonResponse::AppPackageUploadStatus { upload: complete }
    );
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), begin, "begin").await,
        aborted
    );
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
#[tokio::test]
async fn app_worker_and_automation_requests_are_owner_scoped_and_need_no_worker() {
    use crate::local::{
        AppRequestErrorCode, AppWorkerPhase, AppWorkerRequest, ConfigureAppAutomationRequest,
        DisableAppAutomationRequest,
    };
    use chariox_app_package::{verify, VerificationPolicy};
    use chariox_app_runtime::release_store::{ReleaseStore, StageBudget};
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    let store = app.durable_state_store();
    // An active, trusted, never-started installation with its stored release.
    crate::durable_state::app_state::fixture_event_catalog(&store);
    let (bytes, publisher) = crate::durable_state::app_state::fixture_event_package();
    let verified = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let budget = StageBudget {
        max_stage_bytes: 1024 * 1024,
        reserved_bytes: 1024 * 1024,
        host_reserve_bytes: 1024 * 1024,
    };
    ReleaseStore::open_or_create(store.path())
        .unwrap()
        .stage(&verified, &bytes, budget)
        .unwrap();
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    let failed = |code| LocalDaemonResponse::AppRequestFailed { code };
    let worker = LocalDaemonRequest::GetAppWorker(AppWorkerRequest {
        installation_id: "installed".into(),
    });
    let LocalDaemonResponse::AppWorker { worker: status } =
        dispatch(&router, &cache, Some("alice"), worker.clone(), "worker").await
    else {
        panic!("Alice reads her worker status")
    };
    assert_eq!(status.phase, AppWorkerPhase::NotStarted);
    assert!(status.enabled);
    assert_eq!(
        dispatch(&router, &cache, Some("bob"), worker.clone(), "worker").await,
        failed(AppRequestErrorCode::NotFound)
    );
    assert_eq!(
        dispatch(&router, &cache, None, worker, "worker").await,
        failed(AppRequestErrorCode::Unauthorized)
    );
    // A user stop is recorded without a running worker and disables on-demand use.
    let stop = LocalDaemonRequest::ControlAppWorker(crate::local::ControlAppWorkerRequest {
        installation_id: "installed".into(),
        action: crate::local::AppWorkerAction::Stop,
    });
    let LocalDaemonResponse::AppWorker { worker: stopped } =
        dispatch(&router, &cache, Some("alice"), stop.clone(), "stop").await
    else {
        panic!("Alice stops her App")
    };
    assert_eq!(stopped.phase, AppWorkerPhase::Stopped);
    assert!(!stopped.enabled);
    assert_eq!(
        dispatch(&router, &cache, Some("bob"), stop, "stop").await,
        failed(AppRequestErrorCode::NotFound)
    );
    // Automations are durable configuration: no running worker is needed.
    let list = LocalDaemonRequest::ListAppAutomations(AppWorkerRequest {
        installation_id: "installed".into(),
    });
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), list.clone(), "list").await,
        LocalDaemonResponse::AppAutomations {
            installation_id: "installed".into(),
            automations: vec![],
        }
    );
    assert_eq!(
        dispatch(&router, &cache, Some("bob"), list, "list").await,
        failed(AppRequestErrorCode::NotFound)
    );
    // Errors keep their meaning instead of collapsing into Conflict.
    let disable = |expected_revision| {
        LocalDaemonRequest::DisableAppAutomation(DisableAppAutomationRequest {
            installation_id: "installed".into(),
            automation_id: "missing".into(),
            expected_revision,
        })
    };
    // A stale or unknown revision fails the compare-and-set: retryable Conflict.
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), disable(1), "disable-1").await,
        failed(AppRequestErrorCode::Conflict)
    );
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), disable(0), "disable-0").await,
        failed(AppRequestErrorCode::InvalidRequest)
    );
    let configure = LocalDaemonRequest::ConfigureAppAutomation(ConfigureAppAutomationRequest {
        delivery_mode: crate::local::NotificationDeliveryMode::Queue,
        installation_id: "installed".into(),
        automation_id: "reminders".into(),
        expected_revision: 0,
        event_name: "missing_event".into(),
        session_id: "missing-session".into(),
        publication_ref: "missing".into(),
        queue_ref: None,
        scheduled: true,
    });
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), configure, "configure").await,
        failed(AppRequestErrorCode::NotFound)
    );
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
#[tokio::test]
async fn uninstall_is_owner_scoped_generation_checked_and_deactivates() {
    use crate::local::{AppRequestErrorCode, UninstallAppRequest};
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    let store = app.durable_state_store();
    crate::durable_state::app_state::fixture_event_catalog(&store);
    let generation = store
        .get_app_installation("alice", "installed")
        .unwrap()
        .generation
        .to_string();
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    let uninstall_with = |expected_generation: &str, delete_data| {
        LocalDaemonRequest::UninstallApp(UninstallAppRequest {
            installation_id: "installed".into(),
            expected_generation: expected_generation.into(),
            delete_data,
        })
    };
    let uninstall = |expected_generation: &str| uninstall_with(expected_generation, false);
    let failed = |code| LocalDaemonResponse::AppRequestFailed { code };
    assert_eq!(
        dispatch(
            &router,
            &cache,
            Some("bob"),
            uninstall(&generation),
            "u-bob"
        )
        .await,
        failed(AppRequestErrorCode::NotFound)
    );
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), uninstall("999"), "u-stale").await,
        failed(AppRequestErrorCode::Conflict)
    );
    // A stale uninstall has no side effect: the App is not user-stopped.
    let worker = LocalDaemonRequest::GetAppWorker(crate::local::AppWorkerRequest {
        installation_id: "installed".into(),
    });
    let LocalDaemonResponse::AppWorker { worker } =
        dispatch(&router, &cache, Some("alice"), worker, "u-worker").await
    else {
        panic!("worker status")
    };
    assert_eq!(worker.phase, crate::local::AppWorkerPhase::NotStarted);
    assert!(worker.enabled);
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), uninstall("x"), "u-bad").await,
        failed(AppRequestErrorCode::InvalidRequest)
    );
    let LocalDaemonResponse::AppInstallation { installation } = dispatch(
        &router,
        &cache,
        Some("alice"),
        uninstall(&generation),
        "u-ok",
    )
    .await
    else {
        panic!("Alice uninstalls her App")
    };
    assert!(installation.active_release.is_none());
    assert!(installation.data_kept);
    assert_ne!(installation.generation, generation);
    // Uninstalling again with delete_data deletes the kept data (protocol 363).
    store
        .append_app_log("alice", "installed", "info", "kept", &serde_json::json!({}))
        .unwrap();
    let mut delete = dispatch(
        &router,
        &cache,
        Some("alice"),
        uninstall_with(&installation.generation, true),
        "u-delete",
    )
    .await;
    if cfg!(target_os = "linux") {
        // Linux deletes storage through the root storage helper, absent here:
        // the App stays uninstalled with its data.
        assert_eq!(delete, failed(AppRequestErrorCode::StorageUnavailable));
        assert_eq!(
            store.app_logs("alice", "installed", 0, 10).unwrap().len(),
            1
        );
        // Deleting again finishes it once the storage can be deleted (here,
        // fixture storage in place of the helper's).
        let storage = router.runtime_state().app_control().fixture_app_storage();
        let generation = store
            .get_app_installation("alice", "installed")
            .unwrap()
            .generation
            .to_string();
        delete = dispatch(
            &router,
            &cache,
            Some("alice"),
            uninstall_with(&generation, true),
            "u-delete-again",
        )
        .await;
        assert_eq!(
            storage.deleted(),
            [("alice".to_owned(), "installed".to_owned())]
        );
    }
    let LocalDaemonResponse::AppInstallation { installation } = delete else {
        panic!("Alice deletes the kept data")
    };
    assert!(!installation.data_kept);
    assert!(store
        .app_logs("alice", "installed", 0, 10)
        .unwrap()
        .is_empty());
    // The successful uninstall stopped the App first.
    let worker = LocalDaemonRequest::GetAppWorker(crate::local::AppWorkerRequest {
        installation_id: "installed".into(),
    });
    let LocalDaemonResponse::AppWorker { worker } =
        dispatch(&router, &cache, Some("alice"), worker, "u-worker-after").await
    else {
        panic!("worker status")
    };
    assert!(!worker.enabled);
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
#[tokio::test]
async fn app_logs_are_owner_scoped_and_page_by_sequence() {
    use crate::local::{AppRequestErrorCode, GetAppLogsRequest};
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    let store = app.durable_state_store();
    crate::durable_state::app_state::fixture_event_catalog(&store);
    for message in ["one", "two", "three"] {
        store
            .append_app_log(
                "alice",
                "installed",
                "info",
                message,
                &serde_json::json!({}),
            )
            .unwrap();
    }
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    let logs = |after: Option<&str>| {
        LocalDaemonRequest::GetAppLogs(GetAppLogsRequest {
            installation_id: "installed".into(),
            after_sequence: after.map(str::to_owned),
            limit: Some(2),
        })
    };
    let LocalDaemonResponse::AppLogs { entries, .. } =
        dispatch(&router, &cache, Some("alice"), logs(None), "logs-1").await
    else {
        panic!("Alice reads her App's log")
    };
    assert_eq!(
        entries
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
    let LocalDaemonResponse::AppLogs { entries: rest, .. } = dispatch(
        &router,
        &cache,
        Some("alice"),
        logs(Some(&entries[1].sequence)),
        "logs-2",
    )
    .await
    else {
        panic!("next page")
    };
    assert_eq!(
        rest.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(),
        ["three"]
    );
    assert_eq!(
        dispatch(&router, &cache, Some("bob"), logs(None), "logs-bob").await,
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::NotFound
        }
    );
}

#[test]
fn app_uninstall_replay_returns_receipt_without_second_generation_change() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(uninstall_replay());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn uninstall_replay() {
    use crate::local::{AppRequestErrorCode, UninstallAppRequest};
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    let store = app.durable_state_store();
    store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::CreateAndStage {
                installation_id: "todo-alice".into(),
                release: ReleaseMetadata {
                    app_id: "com.chariox.todo".into(),
                    version: "1.0.0".into(),
                    publisher_id: "publisher".into(),
                    package_digest: format!("sha256:{:064x}", 1),
                    schema_version: 1,
                    capabilities_digest: format!("sha256:{:064x}", 2),
                    catalog_digest: format!("sha256:{:064x}", 3),
                    view_digest: format!("sha256:{:064x}", 4),
                },
                now_ms: 1,
            },
        )
        .unwrap();
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    let request = LocalDaemonRequest::UninstallApp(UninstallAppRequest {
        installation_id: "todo-alice".into(),
        expected_generation: "0".into(),
        delete_data: false,
    });
    let first = dispatch(
        &router,
        &cache,
        Some("alice"),
        request.clone(),
        "uninstall-once",
    )
    .await;
    assert!(
        matches!(&first, LocalDaemonResponse::AppInstallation { .. }),
        "{first:?}"
    );
    let generation = store
        .get_app_installation("alice", "todo-alice")
        .unwrap()
        .generation;
    assert_eq!(
        dispatch(
            &router,
            &cache,
            Some("alice"),
            request.clone(),
            "uninstall-once"
        )
        .await,
        first
    );
    assert_eq!(
        store
            .get_app_installation("alice", "todo-alice")
            .unwrap()
            .generation,
        generation
    );
    // Owner authorization still runs before receipt lookup.
    assert_eq!(
        dispatch(&router, &cache, None, request.clone(), "uninstall-once").await,
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::Unauthorized
        }
    );
    assert_eq!(
        dispatch(&router, &cache, Some("bob"), request, "uninstall-once").await,
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::NotFound
        }
    );
    let new = LocalDaemonRequest::UninstallApp(UninstallAppRequest {
        installation_id: "todo-alice".into(),
        expected_generation: generation.to_string(),
        delete_data: false,
    });
    assert!(matches!(
        dispatch(&router, &cache, Some("alice"), new, "uninstall-again").await,
        LocalDaemonResponse::AppInstallation { .. }
    ));
    assert!(
        store
            .get_app_installation("alice", "todo-alice")
            .unwrap()
            .generation
            > generation
    );
}

#[test]
fn rejected_app_requests_cannot_exhaust_another_owners_receipts() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(rejected_app_requests());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn rejected_app_requests() {
    use crate::local::{
        AppRequestErrorCode, AppWorkerAction, ControlAppWorkerRequest, UninstallAppRequest,
    };
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    crate::durable_state::app_state::fixture_event_catalog(&app.durable_state_store());
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    for n in 0..crate::runtime_transport::COMMAND_RESULT_CACHE_LIMIT {
        let request = if n % 2 == 0 {
            LocalDaemonRequest::ControlAppWorker(ControlAppWorkerRequest {
                installation_id: "nonexistent".into(),
                action: AppWorkerAction::Restart,
            })
        } else {
            LocalDaemonRequest::UninstallApp(UninstallAppRequest {
                installation_id: "installed".into(),
                expected_generation: "1".into(),
                delete_data: false,
            })
        };
        assert_eq!(
            dispatch(
                &router,
                &cache,
                Some("bob"),
                request,
                &format!("rejected-{n}")
            )
            .await,
            LocalDaemonResponse::AppRequestFailed {
                code: AppRequestErrorCode::NotFound
            }
        );
    }
    let stop = LocalDaemonRequest::ControlAppWorker(ControlAppWorkerRequest {
        installation_id: "installed".into(),
        action: AppWorkerAction::Stop,
    });
    let stopped = dispatch(
        &router,
        &cache,
        Some("alice"),
        stop.clone(),
        "authorized-stop",
    )
    .await;
    assert!(matches!(&stopped, LocalDaemonResponse::AppWorker { .. }));
    // A previously authorized receipt remains replayable while fresh App I/O
    // admission is exhausted. No ownership/generation operation runs again.
    let state = router.runtime_state();
    let mut permits = Vec::new();
    while let Ok(permit) = state.app_control().try_admit() {
        permits.push(permit);
    }
    assert!(!permits.is_empty());
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), stop, "authorized-stop").await,
        stopped
    );
}

/// V-RUN-03: exercise durable restart admission through the shared relay status
/// response without spawning a provider, App worker, or live kernel service.
#[tokio::test]
async fn app_quarantine_status_survives_the_relay_boundary_and_explicit_start_clears_it() {
    use crate::{
        durable_state::app_worker_lifecycle::{LifecycleStoreError, WorkerPhase},
        local::{AppWorkerPhase, AppWorkerRequest},
        runtime::app_operation_budget::AppOperationBudget,
    };
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    let store = app.durable_state_store();
    crate::durable_state::app_state::fixture_event_catalog(&store);
    crate::durable_state::app_state::fixture_copy_installation(
        &store,
        "alice",
        "neighbour",
        "fixture-deployment",
        crate::durable_state::app_state::fixture_event_package(),
    );
    let budget = || AppOperationBudget::from_supervisor(|| false);
    let neighbour = store
        .claim_active_app_start("alice", "neighbour", "neighbour", false, budget())
        .unwrap();
    store
        .record_app_worker(&neighbour, WorkerPhase::Running, true, None, budget())
        .unwrap();
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    let worker_request = |id: &str| {
        LocalDaemonRequest::GetAppWorker(AppWorkerRequest {
            installation_id: id.into(),
        })
    };
    for failures in 1..=4 {
        let attempt = format!("failure-{failures}");
        let admission = store
            .claim_active_app_start("alice", "installed", &attempt, failures > 1, budget())
            .unwrap();
        store
            .record_app_worker(
                &admission,
                WorkerPhase::Failed,
                true,
                Some("app_worker_exited"),
                budget(),
            )
            .unwrap();
        let LocalDaemonResponse::AppWorker { worker } = dispatch(
            &router,
            &cache,
            Some("alice"),
            worker_request("installed"),
            &attempt,
        )
        .await
        else {
            panic!("worker status must cross the relay boundary")
        };
        assert_eq!(
            worker.phase,
            if failures == 4 {
                AppWorkerPhase::Quarantined
            } else {
                AppWorkerPhase::Failed
            }
        );
        assert_eq!(worker.failure.as_deref(), Some("app_worker_exited"));
        assert!(worker.enabled);
        assert_eq!(
            store
                .app_worker_status("alice", "installed")
                .unwrap()
                .unwrap()
                .failures,
            failures
        );
        // Backdate the completed failure to avoid sleeping through the backoff.
        // Failure four must still refuse recovery even after that delay expires.
        rusqlite::Connection::open(store.path())
            .unwrap()
            .execute(
                "UPDATE app_worker_lifecycle SET updated_ms=0 WHERE installation_id='installed'",
                [],
            )
            .unwrap();
    }
    assert!(matches!(
        store.claim_active_app_start("alice", "installed", "automatic", true, budget()),
        Err(LifecycleStoreError::Stopped)
    ));
    let LocalDaemonResponse::AppWorker { worker } = dispatch(
        &router,
        &cache,
        Some("alice"),
        worker_request("neighbour"),
        "neighbour",
    )
    .await
    else {
        panic!("neighbour status must remain available")
    };
    assert_eq!(worker.phase, AppWorkerPhase::Running);
    assert!(worker.enabled);
    assert!(worker.failure.is_none());
    // The existing explicit start admission, unlike recovery, clears failures.
    let recovered = store
        .claim_active_app_start("alice", "installed", "explicit-start", false, budget())
        .unwrap();
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .failures,
        0
    );
    let LocalDaemonResponse::AppWorker { worker } = dispatch(
        &router,
        &cache,
        Some("alice"),
        worker_request("installed"),
        "starting",
    )
    .await
    else {
        panic!("explicit start status must be visible")
    };
    assert_eq!(worker.phase, AppWorkerPhase::Starting);
    assert!(worker.failure.is_none());
    store
        .record_app_worker(&recovered, WorkerPhase::Running, true, None, budget())
        .unwrap();
    let LocalDaemonResponse::AppWorker { worker } = dispatch(
        &router,
        &cache,
        Some("alice"),
        worker_request("installed"),
        "recovered",
    )
    .await
    else {
        panic!("recovered status must be visible")
    };
    assert_eq!(worker.phase, AppWorkerPhase::Running);
    assert!(worker.failure.is_none());
}

#[test]
fn full_receipt_journal_uninstalls_and_fences_replayed_generation() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    use crate::local::{
                        AppRequestErrorCode, AppWorkerAction, ControlAppWorkerRequest,
                        UninstallAppRequest,
                    };
                    use crate::runtime::command::KernelCommand;
                    let root = TestRoot::new();
                    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
                    let store = app.durable_state_store();
                    crate::durable_state::app_state::fixture_event_catalog(&store);
                    let router = CommandRouter::with_interactive_capacity(
                        Arc::new(tokio::sync::Mutex::new(app)),
                        8,
                    );
                    let state = router.runtime_state();
                    let cache = CommandResultCache::default();
                    let restart = LocalDaemonRequest::ControlAppWorker(ControlAppWorkerRequest {
                        installation_id: "installed".into(),
                        action: AppWorkerAction::Restart,
                    });
                    for n in 0..crate::runtime_transport::COMMAND_RESULT_CACHE_LIMIT {
                        let input = KernelCommand::from_local_request(
                            format!("fill-{n}"),
                            None,
                            None,
                            &restart,
                        );
                        state
                            .app_control()
                            .execute_once("alice", &input, &restart, || async {
                                LocalDaemonResponse::AppRequestFailed {
                                    code: AppRequestErrorCode::Busy,
                                }
                            })
                            .await;
                    }
                    let generation = store
                        .get_app_installation("alice", "installed")
                        .unwrap()
                        .generation;
                    let uninstall = LocalDaemonRequest::UninstallApp(UninstallAppRequest {
                        installation_id: "installed".into(),
                        expected_generation: generation.to_string(),
                        delete_data: false,
                    });
                    assert_eq!(
                        dispatch(
                            &router,
                            &cache,
                            Some("bob"),
                            uninstall.clone(),
                            "uninstall-full"
                        )
                        .await,
                        LocalDaemonResponse::AppRequestFailed {
                            code: AppRequestErrorCode::NotFound
                        }
                    );
                    let result = dispatch(
                        &router,
                        &cache,
                        Some("alice"),
                        uninstall.clone(),
                        "uninstall-full",
                    )
                    .await;
                    assert!(
                        matches!(result, LocalDaemonResponse::AppInstallation { .. }),
                        "{result:?}"
                    );
                    let removed = store.get_app_installation("alice", "installed").unwrap();
                    assert!(removed.active.is_none());
                    assert!(removed.generation > generation);
                    let stopped = store
                        .app_worker_status("alice", "installed")
                        .unwrap()
                        .unwrap();
                    assert!(!stopped.desired_running);
                    assert_eq!(
                        dispatch(&router, &cache, Some("alice"), uninstall, "uninstall-full").await,
                        LocalDaemonResponse::AppRequestFailed {
                            code: AppRequestErrorCode::Conflict
                        }
                    );
                    assert_eq!(
                        store
                            .get_app_installation("alice", "installed")
                            .unwrap()
                            .generation,
                        removed.generation
                    );
                });
        })
        .unwrap()
        .join()
        .unwrap();
}

#[tokio::test]
async fn md3_browser_relay_replay_binds_user_and_rechecks_admission() {
    let root = TestRoot::new();
    let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    let request = LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
        command: crate::local::KernelBrowserCommand::Open {
            url: "about:blank".into(),
        },
    });
    let command = KernelCommand::from_local_request_with_caller(
        "browser-retry",
        KernelCommandSource::RelayClient,
        KernelCaller::from_relay_identity(caller("alice")),
        None,
        None,
        &request,
    );
    let revision = router
        .runtime_state()
        .kernel_browser_receipt_revision(&command, &request)
        .unwrap();
    let fingerprint = CommandFingerprint::from_command_and_request(&command, &request)
        .with_browser_protection_revision(revision);
    assert!(matches!(
        cache.reserve("browser-retry", &fingerprint).await,
        CommandReservation::Dispatch
    ));
    let response = serde_json::json!({"KernelBrowser":{"result":{"tab_id":"alice-private-tab"}}});
    cache
        .complete(
            "browser-retry".into(),
            fingerprint,
            &KernelOutgoingFrame::Response {
                request_id: "browser-retry".into(),
                response: Box::new(Some(response.clone())),
                error: None,
            },
        )
        .await;
    let sequence = AtomicU64::new(1);
    let replay = |identity| {
        dispatch_relay_client_request(
            &router,
            &sequence,
            identity,
            request.clone(),
            Some("browser-retry".into()),
            &cache,
        )
    };
    // Same admitted caller gets the original receipt, so an open/input is not repeated.
    match replay(Some(caller("alice"))).await {
        RelayDispatchOutcome::Response(value) => assert_eq!(value, response),
        _ => panic!("same caller lost its browser retry receipt"),
    }
    match replay(Some(caller("bob"))).await {
        RelayDispatchOutcome::RelayError(error) => {
            assert_eq!(error.code, "duplicate_command_conflict")
        }
        _ => panic!("another user received Alice's browser receipt"),
    }
    for identity in [
        None,
        Some(RelayCallerIdentity {
            user_id: None,
            ..caller("alice")
        }),
        Some(RelayCallerIdentity {
            subject_kind: chariox_relay::auth::RelaySubjectKind::Service,
            ..caller("alice")
        }),
    ] {
        match replay(identity).await {
            RelayDispatchOutcome::RelayError(error) => assert_eq!(error.code, "unauthorized"),
            _ => panic!("browser cache bypassed terminal admission"),
        }
    }
}

#[tokio::test]
async fn md5_relay_cached_browser_observations_obey_current_vault_policy() {
    for pixels in [false, true] {
        let root = TestRoot::new();
        let app = crate::DaemonApp::bootstrap(root.config()).unwrap();
        let router =
            CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 2);
        let state = router.runtime_state();
        state.install_kernel_browser_fixture("alice", &root.0);
        let cache = CommandResultCache::default();
        let sequence = AtomicU64::new(1);
        let request = LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
            command: if pixels {
                crate::local::KernelBrowserCommand::Screenshot {
                    tab_id: "host-tab-fixture".into(),
                    generation: 1,
                }
            } else {
                crate::local::KernelBrowserCommand::Snapshot {
                    tab_id: "host-tab-fixture".into(),
                    generation: 1,
                }
            },
        });
        assert!(matches!(
            dispatch_relay_client_request(
                &router,
                &sequence,
                Some(caller("alice")),
                request.clone(),
                Some("MD5-receipt".into()),
                &cache
            )
            .await,
            RelayDispatchOutcome::Response(_)
        ));
        state.register_kernel_browser_fixture_value("alice", "MD5-sensitive-fixture");
        assert!(
            matches!(
                dispatch_relay_client_request(
                    &router,
                    &sequence,
                    Some(caller("alice")),
                    request.clone(),
                    Some("MD5-receipt".into()),
                    &cache
                )
                .await,
                RelayDispatchOutcome::RelayError(_)
            ),
            "MD-5: changed Vault protection must reject the old text/pixel receipt"
        );
        state.fence_kernel_browser_fixture("alice");
        assert!(
            matches!(
                dispatch_relay_client_request(
                    &router,
                    &sequence,
                    Some(caller("alice")),
                    request,
                    Some("MD5-receipt".into()),
                    &cache
                )
                .await,
                RelayDispatchOutcome::RelayError(_)
            ),
            "MD-5: a fenced registry cannot replay an observation"
        );
        state.shutdown_cleanup().await.unwrap();
    }
}
