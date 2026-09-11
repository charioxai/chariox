use super::*;
use crate::config::{
    load_managed_cloud_relay_profile, load_or_create_managed_runtime_identity,
};
use crate::managed_bootstrap::cloud::{
    DisposableWorkerBootstrapResult, DisposableWorkerEnrollmentReceipt,
    DisposableWorkerExchangeOutcome, DisposableWorkerExchangeRequest,
    DisposableWorkerRecoveryOutcome, HttpBootstrapCloudClient,
};
use crate::managed_bootstrap::state::{
    disposable_worker_binding_digest, BootstrapEnvelope, BootstrapReceiptDocument,
    DisposableWorkerBinding, DisposableWorkerBootstrapReceiptStatus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirstExchange {
    Accept,
    AcceptResultLater,
    Reject,
    FailBeforeAccept,
    LoseAcceptedResponse,
}

struct WorkerCloud {
    exchange_calls: Mutex<Vec<DisposableWorkerExchangeRequest>>,
    recovery_calls: Mutex<Vec<String>>,
    first_exchange: Mutex<FirstExchange>,
    pending_recoveries: Mutex<usize>,
    accepted: Mutex<Option<DisposableWorkerBootstrapResult>>,
}

impl WorkerCloud {
    fn new(first_exchange: FirstExchange) -> Self {
        let pending_recoveries = if first_exchange == FirstExchange::AcceptResultLater {
            1
        } else {
            0
        };
        Self {
            exchange_calls: Mutex::new(Vec::new()),
            recovery_calls: Mutex::new(Vec::new()),
            first_exchange: Mutex::new(first_exchange),
            pending_recoveries: Mutex::new(pending_recoveries),
            accepted: Mutex::new(None),
        }
    }

    fn result(request: &DisposableWorkerExchangeRequest) -> DisposableWorkerBootstrapResult {
        DisposableWorkerBootstrapResult {
            enrollment_receipt: DisposableWorkerEnrollmentReceipt {
                grant_id: "grant-1".into(),
                allocation_id: request.allocation_id.clone(),
                worker_machine_id: request.worker_machine_id.clone(),
                worker_kernel_id: request.worker_kernel_id.clone(),
                image_digest: request.image_digest.clone(),
                runtime_release_digest: request.runtime_release_digest.clone(),
                exchanged_at: "2026-08-20T14:00:00Z".into(),
            },
            cloud_relay: ManagedCloudRelayProfile {
                api_url: "https://cloud.example.test".into(),
                email: "worker@example.test".into(),
                account_id: "account-1".into(),
                user_id: "owner-1".into(),
                account_slug: "account-one".into(),
                realm_id: "realm-1".into(),
                relay_url: "wss://relay.example.test".into(),
                issuer_id: "issuer-1".into(),
                machine_id: request.worker_machine_id.clone(),
                machine_alias: "Disposable worker".into(),
                machine_credential: format!("mcred_{}", "b".repeat(43)),
            },
        }
    }
}

impl BootstrapCloudClient for WorkerCloud {
    fn exchange(
        &self,
        _api_url: &str,
        _request: &ExchangeRequest,
    ) -> Result<ExchangeResponse, DaemonError> {
        panic!("ordinary managed exchange must not be used for a disposable worker")
    }

    fn exchange_disposable_worker(
        &self,
        _api_url: &str,
        request: &DisposableWorkerExchangeRequest,
    ) -> Result<DisposableWorkerExchangeOutcome, DaemonError> {
        self.exchange_calls.lock().unwrap().push(request.clone());
        let mode = std::mem::replace(
            &mut *self.first_exchange.lock().unwrap(),
            FirstExchange::Accept,
        );
        match mode {
            FirstExchange::Reject => Ok(DisposableWorkerExchangeOutcome::Rejected),
            FirstExchange::FailBeforeAccept => Err(DaemonError::LocalTransport {
                operation: "test disposable worker exchange",
                message: "request was not accepted".into(),
            }),
            FirstExchange::LoseAcceptedResponse => {
                *self.accepted.lock().unwrap() = Some(Self::result(request));
                Err(DaemonError::LocalTransport {
                    operation: "test disposable worker exchange",
                    message: "accepted response was lost".into(),
                })
            }
            FirstExchange::Accept | FirstExchange::AcceptResultLater => {
                let result = Self::result(request);
                *self.accepted.lock().unwrap() = Some(result.clone());
                Ok(DisposableWorkerExchangeOutcome::Accepted(
                    result.enrollment_receipt,
                ))
            }
        }
    }

    fn recover_disposable_worker_exchange(
        &self,
        _api_url: &str,
        binding_digest: &str,
        _binding: &DisposableWorkerBinding,
    ) -> Result<DisposableWorkerRecoveryOutcome, DaemonError> {
        self.recovery_calls
            .lock()
            .unwrap()
            .push(binding_digest.to_string());
        let mut pending = self.pending_recoveries.lock().unwrap();
        if *pending > 0 {
            *pending -= 1;
            return Ok(DisposableWorkerRecoveryOutcome::Pending);
        }
        Ok(self
            .accepted
            .lock()
            .unwrap()
            .clone()
            .map(DisposableWorkerRecoveryOutcome::Ready)
            .unwrap_or(DisposableWorkerRecoveryOutcome::Pending))
    }

    fn confirm(
        &self,
        _api_url: &str,
        _request: &ConfirmRequest,
    ) -> Result<ConfirmResponse, DaemonError> {
        panic!("disposable workers do not confirm managed environments")
    }
}

fn worker_binding(fixture: &Fixture) -> DisposableWorkerBinding {
    let identity = load_or_create_managed_runtime_identity(
        &fixture.config.kernel_host,
        fixture.config.kernel_port,
    )
    .unwrap();
    worker_binding_for_identity(fixture, identity.machine_id, identity.kernel_id)
}

fn worker_binding_for_identity(
    fixture: &Fixture,
    worker_machine_id: impl Into<String>,
    worker_kernel_id: impl Into<String>,
) -> DisposableWorkerBinding {
    DisposableWorkerBinding {
        allocation_id: "allocation-1".into(),
        expected_home_kernel_id: "home-kernel-1".into(),
        user_id: "owner-1".into(),
        realm_id: "realm-1".into(),
        worker_machine_id: worker_machine_id.into(),
        worker_kernel_id: worker_kernel_id.into(),
        image_digest: format!("sha256:{}", "c".repeat(64)),
        runtime_release_digest: fixture.release_digest.clone(),
        manager_operation_id: "operation-1".into(),
        manager_operation_fence: 7,
        manager_request_digest: format!("sha256:{}", "d".repeat(64)),
        sender_key_thumbprint: format!("sha256:{}", "e".repeat(64)),
    }
}

#[test]
fn pristine_disposable_worker_initializes_the_envelope_bound_identity_and_local_key() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-pristine-identity");
    let previous_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding_for_identity(
        &fixture,
        "cloud-worker-machine-1",
        "cloud-worker-kernel-1",
    );
    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud::new(FirstExchange::Accept);

    prepare_managed_kernel(&fixture.config, &cloud, fixture.now)
        .expect("a pristine disposable worker should initialize its bound identity");
    let identity = load_or_create_managed_runtime_identity(
        &fixture.config.kernel_host,
        fixture.config.kernel_port,
    )
    .unwrap();
    assert_eq!(identity.machine_id, binding.worker_machine_id);
    assert_eq!(identity.kernel_id, binding.worker_kernel_id);
    assert!(!identity.relay_public_key.trim().is_empty());
    assert!(!serde_json::to_string(&binding)
        .unwrap()
        .contains(&identity.relay_public_key));

    restore_env("CHARIOX_HOME", previous_home);
    fixture.cleanup();
}

#[test]
fn disposable_worker_rejects_a_preexisting_mismatched_identity() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-mismatched-identity");
    let previous_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let existing = load_or_create_managed_runtime_identity(
        &fixture.config.kernel_host,
        fixture.config.kernel_port,
    )
    .unwrap();
    let binding = worker_binding_for_identity(
        &fixture,
        "cloud-worker-machine-2",
        "cloud-worker-kernel-2",
    );
    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud::new(FirstExchange::Accept);

    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    assert!(cloud.exchange_calls.lock().unwrap().is_empty());
    let preserved = load_or_create_managed_runtime_identity(
        &fixture.config.kernel_host,
        fixture.config.kernel_port,
    )
    .unwrap();
    assert_eq!(preserved, existing);

    restore_env("CHARIOX_HOME", previous_home);
    fixture.cleanup();
}

fn write_worker_envelope(fixture: &Fixture, binding: &DisposableWorkerBinding) {
    fs::write(
        &fixture.config.envelope_path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "cloudApiUrl": "https://cloud.example.test",
            "token": format!("dwboot_{}", "a".repeat(43)),
            "expiresAt": (fixture.now + chrono::Duration::minutes(5)).to_rfc3339(),
            "bindingDigest": disposable_worker_binding_digest(binding).unwrap(),
            "binding": binding,
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn disposable_worker_binding_digest_matches_the_cloud_canonical_order() {
    let binding = DisposableWorkerBinding {
        allocation_id: "allocation-1".into(),
        expected_home_kernel_id: "home-kernel-1".into(),
        user_id: "owner-1".into(),
        realm_id: "realm-1".into(),
        worker_machine_id: "machine-1".into(),
        worker_kernel_id: "kernel-1".into(),
        image_digest: format!("sha256:{}", "c".repeat(64)),
        runtime_release_digest: format!("sha256:{}", "a".repeat(64)),
        manager_operation_id: "operation-1".into(),
        manager_operation_fence: 7,
        manager_request_digest: format!("sha256:{}", "d".repeat(64)),
        sender_key_thumbprint: format!("sha256:{}", "e".repeat(64)),
    };
    assert_eq!(
        disposable_worker_binding_digest(&binding).unwrap(),
        "sha256:25cc3ec339e7eff69f7537d244fa097d3ebf172b5d2ea1e3bc1407a2deec13ea"
    );
}

#[test]
fn disposable_worker_cloud_shapes_match_the_approved_contract_exactly() {
    let request = DisposableWorkerExchangeRequest {
        token: format!("dwboot_{}", "a".repeat(43)),
        allocation_id: "allocation-1".into(),
        worker_machine_id: "machine-1".into(),
        worker_kernel_id: "kernel-1".into(),
        image_digest: format!("sha256:{}", "b".repeat(64)),
        runtime_release_digest: format!("sha256:{}", "c".repeat(64)),
        manager_operation_id: "operation-1".into(),
        manager_operation_fence: 7,
        manager_request_digest: format!("sha256:{}", "d".repeat(64)),
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({
            "token": request.token,
            "allocationId": "allocation-1",
            "workerMachineId": "machine-1",
            "workerKernelId": "kernel-1",
            "imageDigest": format!("sha256:{}", "b".repeat(64)),
            "runtimeReleaseDigest": format!("sha256:{}", "c".repeat(64)),
            "managerOperationId": "operation-1",
            "managerOperationFence": 7,
            "managerRequestDigest": format!("sha256:{}", "d".repeat(64)),
        })
    );
    let receipt = DisposableWorkerEnrollmentReceipt {
        grant_id: "grant-1".into(),
        allocation_id: "allocation-1".into(),
        worker_machine_id: "machine-1".into(),
        worker_kernel_id: "kernel-1".into(),
        image_digest: format!("sha256:{}", "b".repeat(64)),
        runtime_release_digest: format!("sha256:{}", "c".repeat(64)),
        exchanged_at: "2026-09-11T00:00:00Z".into(),
    };
    assert_eq!(
        serde_json::to_value(&receipt).unwrap(),
        serde_json::json!({
            "grantId": "grant-1",
            "allocationId": "allocation-1",
            "workerMachineId": "machine-1",
            "workerKernelId": "kernel-1",
            "imageDigest": format!("sha256:{}", "b".repeat(64)),
            "runtimeReleaseDigest": format!("sha256:{}", "c".repeat(64)),
            "exchangedAt": "2026-09-11T00:00:00Z",
        })
    );
}

#[test]
fn ordinary_http_client_does_not_forge_the_home_kernel_caller_boundary() {
    let request = DisposableWorkerExchangeRequest {
        token: format!("dwboot_{}", "a".repeat(43)),
        allocation_id: "allocation-1".into(),
        worker_machine_id: "machine-1".into(),
        worker_kernel_id: "kernel-1".into(),
        image_digest: format!("sha256:{}", "b".repeat(64)),
        runtime_release_digest: format!("sha256:{}", "c".repeat(64)),
        manager_operation_id: "operation-1".into(),
        manager_operation_fence: 7,
        manager_request_digest: format!("sha256:{}", "d".repeat(64)),
    };
    let error = HttpBootstrapCloudClient::default()
        .exchange_disposable_worker("https://cloud.example.test", &request)
        .unwrap_err();
    assert!(error.to_string().contains("verified home-kernel sender proof"));
}

#[test]
fn disposable_worker_envelope_is_strict_and_distinct() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-envelope");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    assert!(matches!(
        BootstrapEnvelope::read(&fixture.config.envelope_path).unwrap(),
        BootstrapEnvelope::DisposableWorker(_)
    ));

    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.config.envelope_path).unwrap()).unwrap();
    value["unexpected"] = serde_json::json!(true);
    fs::write(&fixture.config.envelope_path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(BootstrapEnvelope::read(&fixture.config.envelope_path).is_err());
    value.as_object_mut().unwrap().remove("unexpected");
    value["expiresAt"] = serde_json::json!("tomorrow");
    fs::write(&fixture.config.envelope_path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(BootstrapEnvelope::read(&fixture.config.envelope_path).is_err());
    let cloud = WorkerCloud::new(FirstExchange::Accept);
    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    assert!(!fixture.config.envelope_path.exists());
    fixture.cleanup();
}

#[test]
fn disposable_worker_exchanges_once_and_persists_the_complete_binding() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-replay");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud::new(FirstExchange::Accept);

    let prepared = prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();
    assert!(prepared.confirmation.is_none());
    assert!(!fixture.config.envelope_path.exists());
    let Some(BootstrapReceiptDocument::DisposableWorker(receipt)) =
        BootstrapReceiptDocument::read(&fixture.config.receipt_path).unwrap()
    else {
        panic!("disposable worker receipt must exist")
    };
    assert_eq!(receipt.binding, binding);
    assert_eq!(receipt.binding_digest, disposable_worker_binding_digest(&binding).unwrap());
    assert_eq!(
        receipt.enrollment_receipt.as_ref().unwrap().grant_id,
        "grant-1"
    );
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();
    assert_eq!(cloud.exchange_calls.lock().unwrap().len(), 1);
    assert_eq!(cloud.recovery_calls.lock().unwrap().len(), 1);
    fixture.cleanup();
}

#[test]
fn disposable_worker_recovers_a_lost_accepted_response_without_reexchange() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-response-loss");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud::new(FirstExchange::LoseAcceptedResponse);

    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();
    assert_eq!(cloud.exchange_calls.lock().unwrap().len(), 1);
    assert_eq!(cloud.recovery_calls.lock().unwrap().len(), 1);
    assert!(!fixture.config.envelope_path.exists());
    fixture.cleanup();
}

#[test]
fn disposable_worker_recovers_after_the_exchange_receipt_was_persisted() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-receipt-recovery");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud::new(FirstExchange::AcceptResultLater);

    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    let Some(BootstrapReceiptDocument::DisposableWorker(receipt)) =
        BootstrapReceiptDocument::read(&fixture.config.receipt_path).unwrap()
    else {
        panic!("accepted receipt must survive until result recovery")
    };
    assert_eq!(
        receipt.status,
        DisposableWorkerBootstrapReceiptStatus::ExchangeAccepted
    );
    assert!(!fixture.config.envelope_path.exists());
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();
    assert_eq!(cloud.exchange_calls.lock().unwrap().len(), 1);
    assert_eq!(cloud.recovery_calls.lock().unwrap().len(), 2);
    assert!(!fixture.config.envelope_path.exists());
    fixture.cleanup();
}

#[test]
fn disposable_worker_retries_only_after_recovery_proves_the_send_was_not_accepted() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-send-loss");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud::new(FirstExchange::FailBeforeAccept);

    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();
    assert_eq!(cloud.exchange_calls.lock().unwrap().len(), 2);
    assert_eq!(cloud.recovery_calls.lock().unwrap().len(), 2);
    assert!(!fixture.config.envelope_path.exists());
    fixture.cleanup();
}

#[test]
fn disposable_worker_recovers_missing_profile_and_rejects_any_stale_profile() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-profile-recovery");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud::new(FirstExchange::Accept);
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();

    let profile_path = fixture
        .config
        .chariox_home
        .join("daemon")
        .join("config.json");
    fs::remove_file(&profile_path).unwrap();
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();
    let restored = load_managed_cloud_relay_profile().unwrap();
    assert_eq!(restored.machine_id.as_deref(), Some(binding.worker_machine_id.as_str()));

    let mut persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&profile_path).unwrap()).unwrap();
    persisted["cloud_relay"]["machine_credential"] =
        serde_json::json!(format!("mcred_{}", "z".repeat(43)));
    fs::write(&profile_path, serde_json::to_vec(&persisted).unwrap()).unwrap();
    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    fixture.cleanup();
}

#[test]
fn disposable_worker_rejects_a_stale_enrollment_receipt() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-stale-receipt");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud::new(FirstExchange::Accept);
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();

    let mut receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.config.receipt_path).unwrap()).unwrap();
    receipt["enrollmentReceipt"]["imageDigest"] =
        serde_json::json!(format!("sha256:{}", "f".repeat(64)));
    fs::write(
        &fixture.config.receipt_path,
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    fixture.cleanup();
}

#[test]
fn disposable_worker_recreated_envelope_is_removed_only_for_the_exact_receipt_binding() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-envelope-recovery");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    let envelope_bytes = fs::read(&fixture.config.envelope_path).unwrap();
    let cloud = WorkerCloud::new(FirstExchange::Accept);
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();

    fs::write(&fixture.config.envelope_path, &envelope_bytes).unwrap();
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();
    assert!(!fixture.config.envelope_path.exists());
    assert_eq!(cloud.exchange_calls.lock().unwrap().len(), 1);

    fs::write(&fixture.config.envelope_path, &envelope_bytes).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&envelope_bytes).unwrap();
    value["binding"]["managerOperationFence"] = serde_json::json!(8);
    value["bindingDigest"] = serde_json::json!(disposable_worker_binding_digest(
        &serde_json::from_value(value["binding"].clone()).unwrap()
    )
    .unwrap());
    fs::write(&fixture.config.envelope_path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    assert!(!fixture.config.envelope_path.exists());
    fixture.cleanup();
}

#[test]
fn completed_worker_receipt_wins_release_selection_over_a_stale_envelope() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-stale-release-envelope");
    let previous_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud::new(FirstExchange::Accept);
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();

    let mut stale_binding = binding;
    stale_binding.runtime_release_digest = format!("sha256:{}", "9".repeat(64));
    write_worker_envelope(&fixture, &stale_binding);
    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    assert!(!fixture.config.envelope_path.exists());
    assert_eq!(cloud.exchange_calls.lock().unwrap().len(), 1);

    restore_env("CHARIOX_HOME", previous_home);
    fixture.cleanup();
}

#[test]
fn disposable_worker_binding_mismatch_and_terminal_rejection_remove_envelope() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-reject");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.config.envelope_path).unwrap()).unwrap();
    value["expiresAt"] =
        serde_json::json!((fixture.now + chrono::Duration::minutes(31)).to_rfc3339());
    fs::write(&fixture.config.envelope_path, serde_json::to_vec(&value).unwrap()).unwrap();
    let cloud = WorkerCloud::new(FirstExchange::Accept);
    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    assert!(!fixture.config.envelope_path.exists());

    write_worker_envelope(&fixture, &binding);
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.config.envelope_path).unwrap()).unwrap();
    value["binding"]["allocationId"] = serde_json::json!("allocation-tampered");
    fs::write(&fixture.config.envelope_path, serde_json::to_vec(&value).unwrap()).unwrap();
    let cloud = WorkerCloud::new(FirstExchange::Accept);
    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    assert!(!fixture.config.envelope_path.exists());

    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud::new(FirstExchange::Reject);
    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    assert!(!fixture.config.envelope_path.exists());
    assert!(!fixture.config.receipt_path.exists());
    fixture.cleanup();
}

#[test]
fn existing_supervisor_inherits_disposable_worker_environment() {
    let source = include_str!("../supervisor.rs");
    assert!(!source.contains("env_clear"));
    for name in [
        "CHARIOX_KERNEL_RUNTIME_ROLE",
        "CHARIOX_REMOTE_LEASE_CAPACITY",
        "CHARIOX_ACCEPT_REMOTE_LEASES",
    ] {
        assert!(!source.contains(&format!("env_remove(\"{name}\")")));
    }
    let cloud = include_str!("../cloud.rs");
    assert!(cloud.contains("/v1/disposable-workers/bootstrap/exchange"));
    assert_eq!(
        cloud.matches("/v1/managed-kernels/bootstrap/exchange").count(),
        1
    );
    assert!(cloud.contains("verified home-kernel sender proof"));
}
