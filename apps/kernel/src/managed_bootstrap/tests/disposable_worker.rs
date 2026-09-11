use super::*;
use crate::config::load_or_create_managed_runtime_identity;
use crate::managed_bootstrap::cloud::{
    DisposableWorkerExchangeOutcome, DisposableWorkerExchangeRequest,
    DisposableWorkerExchangeResponse,
};
use crate::managed_bootstrap::state::{
    disposable_worker_binding_digest, BootstrapEnvelope, BootstrapReceiptDocument,
    DisposableWorkerBinding,
};

struct WorkerCloud {
    calls: Mutex<Vec<DisposableWorkerExchangeRequest>>,
    reject: bool,
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
        self.calls.lock().unwrap().push(request.clone());
        if self.reject {
            return Ok(DisposableWorkerExchangeOutcome::Rejected);
        }
        Ok(DisposableWorkerExchangeOutcome::Accepted(
            DisposableWorkerExchangeResponse {
                allocation_id: request.allocation_id.clone(),
                worker_machine_id: request.worker_machine_id.clone(),
                worker_kernel_id: request.worker_kernel_id.clone(),
                image_digest: request.image_digest.clone(),
                runtime_release_digest: request.runtime_release_digest.clone(),
                manager_operation_id: request.manager_operation_id.clone(),
                manager_operation_fence: request.manager_operation_fence,
                manager_request_digest: request.manager_request_digest.clone(),
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
            },
        ))
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
    DisposableWorkerBinding {
        allocation_id: "allocation-1".into(),
        expected_home_kernel_id: "home-kernel-1".into(),
        user_id: "owner-1".into(),
        realm_id: "realm-1".into(),
        worker_machine_id: identity.machine_id,
        worker_kernel_id: identity.kernel_id,
        image_digest: format!("sha256:{}", "c".repeat(64)),
        runtime_release_digest: fixture.release_digest.clone(),
        manager_operation_id: "operation-1".into(),
        manager_operation_fence: 7,
        manager_request_digest: format!("sha256:{}", "d".repeat(64)),
        sender_key_thumbprint: format!("sha256:{}", "e".repeat(64)),
    }
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
    fixture.cleanup();
}

#[test]
fn disposable_worker_exchanges_once_and_resumes_from_receipt() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("worker-replay");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let binding = worker_binding(&fixture);
    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud { calls: Mutex::new(Vec::new()), reject: false };

    let prepared = prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();
    assert!(prepared.confirmation.is_none());
    assert!(!fixture.config.envelope_path.exists());
    assert!(matches!(
        BootstrapReceiptDocument::read(&fixture.config.receipt_path).unwrap(),
        Some(BootstrapReceiptDocument::DisposableWorker(_))
    ));
    prepare_managed_kernel(&fixture.config, &cloud, fixture.now).unwrap();
    assert_eq!(cloud.calls.lock().unwrap().len(), 1);
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
    value["binding"]["allocationId"] = serde_json::json!("allocation-tampered");
    fs::write(&fixture.config.envelope_path, serde_json::to_vec(&value).unwrap()).unwrap();
    let cloud = WorkerCloud { calls: Mutex::new(Vec::new()), reject: false };
    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    assert!(!fixture.config.envelope_path.exists());

    write_worker_envelope(&fixture, &binding);
    let cloud = WorkerCloud { calls: Mutex::new(Vec::new()), reject: true };
    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());
    assert!(!fixture.config.envelope_path.exists());
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
}
