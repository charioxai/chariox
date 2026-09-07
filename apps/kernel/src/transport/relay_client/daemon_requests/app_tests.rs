use super::*;
use crate::durable_state::apps::AppRegistryMutation;
use crate::local::{AppInstallationRequest, ListAppInstallationsRequest, LocalDaemonResponse};
use chariox_app_runtime::installation::ReleaseMetadata;

struct TestRoot(std::path::PathBuf);

impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-app-relay-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
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
    let aborted = LocalDaemonResponse::AppPackageUploadStatus { upload: complete };
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), abort, "abort").await,
        aborted
    );
    assert_eq!(
        dispatch(&router, &cache, Some("alice"), begin, "begin").await,
        aborted
    );
}
