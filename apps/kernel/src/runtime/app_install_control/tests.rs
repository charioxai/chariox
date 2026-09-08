use super::*;
use crate::durable_state::{
    app_publishers::AppPublisherMutation, app_state::fixture_event_package,
};
use crate::local::*;
use crate::runtime::command::KernelCommand;
use crate::{
    app::DaemonApp,
    config::DaemonConfig,
    session::{CollaborationLevel, RuntimeSession},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chariox_app_runtime::publisher_trust::TrustDecision;
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    state: KernelRuntimeState,
    session: String,
    path: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let path = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "chariox-install-control-{:016x}",
                rand::random::<u64>()
            ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(path.join("kernel.sqlite").display().to_string());
        config.session_history_root_default = path.join("history");
        config.user_config.history.operational.path =
            Some(path.join("operational.sqlite").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(path.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(path.join("artifact-index.sqlite").display().to_string());
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let mut session = RuntimeSession::new(
            "install-session",
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        session.add_member("alice", None, CollaborationLevel::Full);
        let session_id = session.id().to_owned();
        app.sessions_mut().restore_session(session);
        let config = app.config_projection_store();
        let sessions = app.session_state_store();
        let agents = app.agents().clone();
        let attachments = app.attachments().clone();
        let providers = app.providers().clone();
        let tracking = app.provider_process_tracking_store();
        let slices = app.slices();
        let projection = app.session_state_projection_store();
        let runs = app.provider_run_projection_store();
        let history = app.operational_history_store();
        let durable = app.durable_state_store();
        let prompts = app.prompt_state_owner();
        let turns = app.active_turn_store();
        let activity = app.prompt_activity_store();
        let claims = app.prompt_workspace_claim_store();
        let outputs = app.structured_output_record_store();
        let terminal = app.terminal_stream_store();
        let workflow = app.workflow_design_event_store();
        let meta = app.metaagent_event_store();
        let workspace = app.workspace_coordinator();
        let state = KernelRuntimeState::new_with_owned_state(
            Arc::new(tokio::sync::Mutex::new(app)),
            config,
            sessions,
            agents,
            attachments,
            providers,
            tracking,
            slices,
            projection,
            runs,
            history,
            durable,
            prompts,
            turns,
            activity,
            claims,
            outputs,
            terminal,
            workflow,
            meta,
            workspace,
        );
        Self {
            state,
            session: session_id,
            path,
        }
    }
    fn control(&self) -> &AppInstallControl {
        self.state.app_control().installs()
    }
    async fn request(&self, owner: &str, request: LocalDaemonRequest) -> LocalDaemonResponse {
        let mut command = KernelCommand::from_local_request("install-test", None, None, &request);
        command.caller.user_id = Some(owner.into());
        self.control()
            .execute(&self.state, &command, &request)
            .await
            .unwrap()
    }
    async fn uploaded(&self) -> (String, String) {
        let (bytes, publisher) = fixture_event_package();
        self.control()
            .0
            .shared
            .store
            .mutate_app_publisher(
                "alice",
                AppPublisherMutation::Enroll {
                    publisher,
                    expected_revision: 0,
                    decision: TrustDecision {
                        decision_id: "enroll".into(),
                        authority_ref: "kernel_test".into(),
                    },
                    now_ms: 1,
                },
            )
            .unwrap();
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        let begin = LocalDaemonRequest::BeginAppPackageUpload(BeginAppPackageUploadRequest {
            request_id: "upload".into(),
            expected_size: bytes.len() as u64,
            sha256: digest.clone(),
        });
        let mut command = KernelCommand::from_local_request("upload", None, None, &begin);
        command.caller.user_id = Some("alice".into());
        let LocalDaemonResponse::AppPackageUploadStatus { upload } = self
            .state
            .app_control()
            .execute(&command, &begin)
            .await
            .unwrap()
        else {
            panic!("upload begin")
        };
        let chunk = LocalDaemonRequest::PutAppPackageUploadChunk(PutAppPackageUploadChunkRequest {
            handle: upload.handle.clone(),
            offset: 0,
            data_base64: STANDARD.encode(bytes),
            chunk_sha256: digest.clone(),
        });
        assert!(matches!(
            self.state.app_control().execute(&command, &chunk).await,
            Some(LocalDaemonResponse::AppPackageUploadStatus { .. })
        ));
        (upload.handle, digest)
    }
    fn begin(&self, handle: String, digest: String) -> LocalDaemonRequest {
        LocalDaemonRequest::BeginAppInstall(BeginAppInstallRequest {
            session_id: self.session.clone(),
            request_id: "install".into(),
            upload_handle: handle,
            expected_package_digest: digest,
        })
    }
    async fn until(&self, mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(8), async {
            while !condition() {
                self.control().pump(&self.state).await;
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("bounded install pump");
    }
    async fn shutdown(&self) {
        let control = self.control().clone();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || control.shutdown_blocking(handle))
            .await
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fn writable(path: &std::path::Path) {
            if std::fs::symlink_metadata(path).is_ok_and(|v| v.is_dir()) {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
                if let Ok(entries) = std::fs::read_dir(path) {
                    for e in entries.flatten() {
                        writable(&e.path());
                    }
                }
            }
        }
        let _ = self.control().0.shared.store.fence_writer();
        writable(&self.path);
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
fn status(response: LocalDaemonResponse) -> AppInstallOperationSummary {
    match response {
        LocalDaemonResponse::AppInstallOperationStatus { operation } => operation,
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn immediate_receipt_then_real_pump_owner_decision_declines_without_activation() {
    let f = Fixture::new();
    let (handle, digest) = f.uploaded().await;
    let begin = f.begin(handle, digest);
    assert!(matches!(
        f.request("bob", begin.clone()).await,
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::Unauthorized
        }
    ));
    let ack = status(f.request("alice", begin.clone()).await);
    assert_eq!(ack.phase, AppInstallOperationPhase::Preparing);
    assert!(ack.installation_id.is_none());
    assert_eq!(ack, status(f.request("alice", begin).await));
    f.until(|| {
        let state = f.control().0.state.lock().unwrap();
        state
            .entries
            .values()
            .any(|e| matches!(e.step, Step::Waiting { .. }))
    })
    .await;
    let interaction = {
        let state = f.control().0.state.lock().unwrap();
        let Step::Waiting { challenge, .. } = &state.entries.values().next().unwrap().step else {
            panic!("waiting")
        };
        assert_eq!(challenge.review()["informationSetConsent"], "not_granted");
        challenge.interaction_id().to_owned()
    };
    assert!(f
        .state
        .resolve_terminal_runtime_interaction(
            &f.session,
            &interaction,
            "approve",
            None,
            Some("bob")
        )
        .await
        .is_err());
    f.state
        .resolve_terminal_runtime_interaction(
            &f.session,
            &interaction,
            "decline",
            None,
            Some("alice"),
        )
        .await
        .unwrap();
    f.until(|| {
        f.control()
            .0
            .shared
            .store
            .first_app_install_status("alice", "install")
            .is_ok_and(|v| v.phase == InstallPhase::Cancelled)
    })
    .await;
    let cancelled = status(
        f.request(
            "alice",
            LocalDaemonRequest::GetAppInstallOperation(AppInstallOperationRequest {
                request_id: "install".into(),
            }),
        )
        .await,
    );
    assert_eq!(cancelled.phase, AppInstallOperationPhase::Cancelled);
    assert!(cancelled.interaction_id.is_none());
    assert!(f
        .control()
        .0
        .shared
        .store
        .get_app_installation("alice", cancelled.installation_id.as_ref().unwrap())
        .unwrap()
        .active
        .is_none());
    f.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_cancel_under_saturated_admission_is_retained_before_stage_and_shutdown_drains() {
    let f = Fixture::new();
    let (handle, digest) = f.uploaded().await;
    let begin = f.begin(handle, digest);
    status(f.request("alice", begin.clone()).await);
    let permits = f
        .control()
        .0
        .shared
        .admission
        .clone()
        .acquire_many_owned(8)
        .await
        .unwrap();
    assert!(matches!(
        f.request(
            "alice",
            LocalDaemonRequest::CancelAppInstallOperation(AppInstallOperationRequest {
                request_id: "install".into()
            })
        )
        .await,
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::Busy
        }
    ));
    drop(permits);
    f.until(|| {
        f.control()
            .0
            .shared
            .store
            .first_app_install_status("alice", "install")
            .is_ok_and(|v| v.phase == InstallPhase::Cancelled)
    })
    .await;
    let operation = f
        .control()
        .0
        .shared
        .store
        .first_app_install_status("alice", "install")
        .unwrap();
    assert!(operation.review.is_none());
    assert!(f
        .control()
        .0
        .shared
        .store
        .get_app_installation("alice", &operation.token.installation_id)
        .is_err());
    assert_eq!(
        status(f.request("alice", begin).await).phase,
        AppInstallOperationPhase::Cancelled
    );
    f.shutdown().await;
    assert!(f.control().0.state.lock().unwrap().tasks.is_empty());
    f.control().notify(("alice".into(), "late".into()));
    assert!(f.control().0.state.lock().unwrap().entries.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_request_stays_owned_until_shutdown_joins_its_sqlite_wait() {
    disconnected_request_shutdown(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_cancel_survives_disconnection_and_shutdown_before_sqlite_commit() {
    disconnected_request_shutdown(true).await;
}

async fn disconnected_request_shutdown(cancelling: bool) {
    let f = Fixture::new();
    let begin = f.begin(
        format!("upload_{}", "a".repeat(64)),
        format!("sha256:{}", "b".repeat(64)),
    );
    let request = if cancelling {
        assert_eq!(
            status(f.request("alice", begin).await).phase,
            AppInstallOperationPhase::Preparing
        );
        LocalDaemonRequest::CancelAppInstallOperation(AppInstallOperationRequest {
            request_id: "install".into(),
        })
    } else {
        begin
    };
    let mut blocked = rusqlite::Connection::open(f.control().0.shared.store.path()).unwrap();
    let transaction = blocked
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let (entered, receiving) = std::sync::mpsc::channel();
    let entered = Mutex::new(Some(entered));
    *f.control().0.request_checkpoint.lock().unwrap() = Some(Arc::new(move || {
        if let Some(sender) = entered.lock().unwrap().take() {
            sender.send(()).unwrap();
        }
    }));
    let state = f.state.clone();
    let waiting = tokio::spawn(async move {
        let mut command = KernelCommand::from_local_request("disconnected", None, None, &request);
        command.caller.user_id = Some("alice".into());
        state
            .app_control()
            .installs()
            .execute(&state, &command, &request)
            .await
    });
    tokio::task::spawn_blocking(move || receiving.recv_timeout(Duration::from_secs(3)))
        .await
        .unwrap()
        .unwrap();
    // The budget's observed result was captured false before this signal; the
    // writer must now pass BEGIN and reach its post-wait/commit cancellation gate.
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    assert_eq!(f.control().0.state.lock().unwrap().requests.len(), 1);
    let control = f.control().clone();
    let handle = tokio::runtime::Handle::current();
    let mut shutdown = tokio::task::spawn_blocking(move || control.shutdown_blocking(handle));
    tokio::time::timeout(Duration::from_secs(2), async {
        while !f.control().0.stopped.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
            .await
            .is_err(),
        "shutdown must wait for the actual request job"
    );
    transaction.rollback().unwrap();
    tokio::time::timeout(Duration::from_secs(3), shutdown)
        .await
        .unwrap()
        .unwrap();
    assert!(f.control().0.state.lock().unwrap().requests.is_empty());
    assert_eq!(f.control().0.shared.admission.available_permits(), 8);
    let result = f
        .control()
        .0
        .shared
        .store
        .first_app_install_status("alice", "install");
    if cancelling {
        assert_eq!(result.unwrap().phase, InstallPhase::Cancelled);
    } else {
        assert!(matches!(result, Err(InstallOperationError::NotFound)));
    }
}
