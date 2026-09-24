//! Real upload/verification/writer/lifecycle integration. The executable is the
//! fixed libc fixture, so this proves sequencing and ownership, not Node support.
use super::*;
use crate::{
    durable_state::{app_installation_operations::InstallOperation, apps::AppRegistryMutation},
    local::*,
    runtime::{app_control::FirstInstallControlError, command::KernelCommand},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chariox_app_runtime::installation::{CapabilityApproval, CapabilityDecision};
use sha2::{Digest, Sha256};

fn enrolled(store: &DurableKernelStateStore) -> Vec<u8> {
    let (bytes, publisher) = fixture_event_package();
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Enroll {
                publisher,
                expected_revision: 0,
                decision: TrustDecision {
                    decision_id: "first_enrollment".into(),
                    authority_ref: "kernel_test".into(),
                },
                now_ms: 1,
            },
        )
        .unwrap();
    bytes
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
async fn request(control: &AppControlService, request: LocalDaemonRequest) -> LocalDaemonResponse {
    let mut command = KernelCommand::from_local_request("first_upload", None, None, &request);
    command.caller.user_id = Some("alice".into());
    control.execute(&command, &request).await.unwrap()
}
async fn upload(control: &AppControlService, bytes: &[u8]) -> String {
    let response = request(
        control,
        LocalDaemonRequest::BeginAppPackageUpload(BeginAppPackageUploadRequest {
            request_id: "first_upload".into(),
            expected_size: bytes.len() as u64,
            sha256: digest(bytes),
        }),
    )
    .await;
    let LocalDaemonResponse::AppPackageUploadStatus { upload } = response else {
        panic!("upload begin failed: {response:?}")
    };
    let response = request(
        control,
        LocalDaemonRequest::PutAppPackageUploadChunk(PutAppPackageUploadChunkRequest {
            handle: upload.handle.clone(),
            offset: 0,
            data_base64: STANDARD.encode(bytes),
            chunk_sha256: digest(bytes),
        }),
    )
    .await;
    assert!(matches!(
        response,
        LocalDaemonResponse::AppPackageUploadStatus { .. }
    ));
    upload.handle
}
fn prepare(
    control: &AppControlService,
    runtime: &Runtime,
    bytes: &[u8],
) -> (String, InstallOperation) {
    runtime.block_on(async {
        let handle = upload(control, bytes).await;
        let operation = control
            .prepare_first_install(
                "alice".into(),
                "first_request".into(),
                handle.clone(),
                digest(bytes),
            )
            .await
            .unwrap();
        (handle, operation)
    })
}
fn approve(store: &DurableKernelStateStore, operation: &InstallOperation) {
    store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::Decide {
                token: operation.token.clone(),
                decision: CapabilityDecision::Approved {
                    approval: CapabilityApproval {
                        decision_id: "first_approval".into(),
                        authority_ref: "kernel_test".into(),
                    },
                },
                now_ms: 2,
            },
        )
        .unwrap();
}

#[test]
fn canonical_control_preparation_replays_after_upload_abort_and_reopen() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let bytes = enrolled(&store);
    let control = AppControlService::new(store.clone());
    let (handle, operation) = prepare(&control, &runtime, &bytes);
    assert_eq!(operation.package_digest, digest(&bytes));
    assert_eq!(operation.phase, InstallPhase::AwaitingApproval);
    assert!(store
        .get_app_installation("alice", &operation.token.installation_id)
        .unwrap()
        .active
        .is_none());
    runtime.block_on(async {
        assert!(matches!(
            request(
                &control,
                LocalDaemonRequest::AbortAppPackageUpload(AppPackageUploadRequest {
                    handle: handle.clone()
                })
            )
            .await,
            LocalDaemonResponse::AppPackageUploadStatus { .. }
        ));
        assert_eq!(
            control
                .prepare_first_install(
                    "alice".into(),
                    "first_request".into(),
                    handle.clone(),
                    digest(&bytes)
                )
                .await
                .unwrap(),
            operation
        );
        assert!(matches!(
            control
                .prepare_first_install(
                    "alice".into(),
                    "first_request".into(),
                    handle.clone(),
                    format!("sha256:{}", "0".repeat(64))
                )
                .await,
            Err(FirstInstallControlError::Operation(
                InstallOperationError::Conflict
            ))
        ));
        assert!(matches!(
            control
                .prepare_first_install(
                    "alice".into(),
                    "first_request".into(),
                    handle.clone(),
                    digest(&bytes)[7..].into()
                )
                .await,
            Err(FirstInstallControlError::Invalid)
        ));
        assert!(control
            .prepare_first_install(
                "bob".into(),
                "first_request".into(),
                handle.clone(),
                digest(&bytes)
            )
            .await
            .is_err());
    });
    let cancelled = store
        .cancel_first_app_install(
            "alice",
            "first_request",
            AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
    drop(control);
    drop(store);
    let store = scratch.store();
    let control = AppControlService::new(store.clone());
    assert_eq!(
        runtime
            .block_on(control.prepare_first_install(
                "alice".into(),
                "first_request".into(),
                handle,
                digest(&bytes)
            ))
            .unwrap(),
        cancelled
    );
    assert_eq!(
        store
            .list_app_installations("alice", None, 10)
            .unwrap()
            .installations
            .len(),
        1
    );
}

#[test]
fn first_install_health_precedes_commit_and_startup_sdk_then_publishes_without_view() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let bytes = enrolled(&store);
    let (control, observations) = make_control(&store, Arc::new(NativeFixture::compile().unwrap()));
    let (_, operation) = prepare(&control, &runtime, &bytes);
    approve(&store, &operation);
    control
        .lifecycle()
        .start_first_blocking("alice", "first_request", runtime.handle().clone())
        .unwrap();
    let id = &operation.token.installation_id;
    wait(|| control.active_app_lease("alice", id).is_some());
    assert_eq!(
        store
            .first_app_install_status("alice", "first_request")
            .unwrap()
            .phase,
        InstallPhase::Committed
    );
    assert_eq!(
        store.get_app_installation("alice", id).unwrap().generation,
        1
    );
    assert_eq!(
        store.app_worker_status("alice", id).unwrap().unwrap().phase,
        WorkerPhase::Running
    );
    {
        let observations = observations.lock().unwrap();
        assert_eq!(observations.len(), 1);
        assert!(observations[0].ready_was_acknowledged());
        assert_eq!(
            observations[0].private_file().unwrap().as_deref(),
            Some(b"startup".as_slice())
        );
    }
    control.lifecycle().stop_blocking("alice", id).unwrap();
    assert!(all_reaped(&observations));
    control.lifecycle().shutdown_blocking().unwrap();
    drop(control);
    drop(store);
    let reopened = scratch.store();
    assert_eq!(
        reopened
            .first_app_install_status("alice", "first_request")
            .unwrap()
            .phase,
        InstallPhase::Committed
    );
    assert!(reopened
        .app_worker_recovery_candidates(None)
        .unwrap()
        .is_empty());
}

#[test]
fn failed_actual_health_never_acknowledges_ready_or_runs_postcommit_startup() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let bytes = enrolled(&store);
    let (control, observations) = make_control(&store, Arc::new(NativeFixture::compile().unwrap()));
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .fail_health = true;
    let (_, operation) = prepare(&control, &runtime, &bytes);
    approve(&store, &operation);
    control
        .lifecycle()
        .start_first_blocking("alice", "first_request", runtime.handle().clone())
        .unwrap();
    wait(|| {
        store
            .first_app_install_status("alice", "first_request")
            .unwrap()
            .phase
            == InstallPhase::Failed
    });
    assert!(all_reaped(&observations));
    let observations = observations.lock().unwrap();
    assert_eq!(observations.len(), 1);
    assert!(!observations[0].ready_was_acknowledged());
    assert_eq!(observations[0].private_file().unwrap(), None);
    assert!(store
        .get_app_installation("alice", &operation.token.installation_id)
        .unwrap()
        .active
        .is_none());
    assert!(store
        .first_app_install_recovery_candidates(None)
        .unwrap()
        .is_empty());
    drop(observations);
    control.lifecycle().shutdown_blocking().unwrap();
}

#[test]
fn one_saturated_stop_before_first_claim_cancels_both_recovery_paths() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let bytes = enrolled(&store);
    let control = AppControlService::new(store.clone());
    let (_, operation) = prepare(&control, &runtime, &bytes);
    approve(&store, &operation);
    let service = control.lifecycle();
    let (entered, received) = std::sync::mpsc::channel();
    let entered = Mutex::new(Some(entered));
    *service.0.claim_checkpoint.lock().unwrap() = Some(Arc::new(move || {
        if let Some(sender) = entered.lock().unwrap().take() {
            sender.send(()).unwrap();
        }
    }));
    let permits = service
        .0
        .admission
        .clone()
        .try_acquire_many_owned(7)
        .unwrap();
    let mut blocked = rusqlite::Connection::open(store.path()).unwrap();
    let transaction = blocked
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    service
        .start_first_blocking("alice", "first_request", runtime.handle().clone())
        .unwrap();
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        service.stop_blocking("alice", &operation.token.installation_id),
        Err(LifecycleError::Busy)
    ));
    drop(transaction);
    drop(permits);
    wait(|| {
        store
            .first_app_install_status("alice", "first_request")
            .unwrap()
            .phase
            == InstallPhase::Cancelled
    });
    service.shutdown_blocking().unwrap();
    drop(control);
    drop(store);
    let reopened = scratch.store();
    assert!(reopened
        .first_app_install_recovery_candidates(None)
        .unwrap()
        .is_empty());
    assert!(reopened
        .app_worker_recovery_candidates(None)
        .unwrap()
        .is_empty());
    assert_eq!(
        reopened
            .first_app_install_status("alice", "first_request")
            .unwrap()
            .phase,
        InstallPhase::Cancelled
    );
}

#[test]
fn revoked_signer_before_first_claim_withdraws_pending_install_without_spawning() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let bytes = enrolled(&store);
    let control = AppControlService::new(store.clone());
    let (_, operation) = prepare(&control, &runtime, &bytes);
    approve(&store, &operation);
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: TrustDecision {
                    decision_id: "revoke_before_claim".into(),
                    authority_ref: "kernel_test".into(),
                },
                now_ms: 3,
            },
        )
        .unwrap();
    control
        .lifecycle()
        .start_first_blocking("alice", "first_request", runtime.handle().clone())
        .unwrap();
    wait(|| {
        store
            .first_app_install_status("alice", "first_request")
            .unwrap()
            .phase
            == InstallPhase::Cancelled
    });
    assert!(store
        .first_app_install_recovery_candidates(None)
        .unwrap()
        .is_empty());
    assert!(store
        .get_app_installation("alice", &operation.token.installation_id)
        .unwrap()
        .active
        .is_none());
    control.lifecycle().shutdown_blocking().unwrap();
}
