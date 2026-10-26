use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use base64::Engine;
use chrono::{TimeZone, Utc};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use super::cloud::{
    BootstrapCloudClient, ConfirmRequest, ConfirmResponse, ExchangeRequest, ExchangeResponse,
    ManagedCloudRelayProfile, ReconcileManagedBootstrapGrantRequestV1,
    ReconcileManagedBootstrapGrantResponseV1, RuntimeIdentityReportResponse,
};
use super::freshness::{
    ManagedKernelFreshnessEvidence, ManagedKernelResidueChecks, ManagedKernelRuntimeIdentityReport,
};
use super::prepare_managed_kernel;
use super::release::verify_release;
use super::state::{
    BootstrapConfig, BootstrapReceipt, BootstrapReceiptStatus, ManagedBootstrapEnvelope,
    ManagedBootstrapGrantBinding,
};
use super::supervisor::run_kernel_once;
use super::{
    expected_data_volume_identity, validate_pre_reimage_observation_binding,
    validate_rebuild_volume_evidence, ConfirmedManagedKernelRegistration, ManagedKernelContextPlan,
    ManagedProviderTopology, MANAGED_PROVIDER_TOPOLOGY_ENV,
};
use crate::config::{DaemonConfig, ManagedRuntimeIdentity, PersistedCloudRelayProfile};
use crate::error::DaemonError;

mod disposable_worker;

#[test]
fn ordinary_explicit_state_ignores_host_global_managed_receipt() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("ordinary-global-receipt");
    let _restore = EnvironmentRestoreGuard::capture([
        "HOME",
        "CHARIOX_HOME",
        "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
        MANAGED_PROVIDER_TOPOLOGY_ENV,
    ]);
    std::env::set_var("HOME", &fixture.config.process_home);
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    std::env::remove_var("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT");
    std::env::remove_var(MANAGED_PROVIDER_TOPOLOGY_ENV);
    let fallback = fixture.root.join("host-global-receipt.json");
    fs::write(&fallback, "not this ordinary kernel's receipt").unwrap();
    let registration =
        super::confirmed_managed_kernel_registration_with_receipt_fallback(fallback.clone());
    assert!(
        matches!(registration, Ok(None)),
        "ordinary startup must ignore a co-resident managed receipt"
    );
    assert_eq!(
        fs::read_to_string(&fallback).unwrap(),
        "not this ordinary kernel's receipt"
    );

    std::env::set_var("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT", &fallback);
    assert!(
        super::confirmed_managed_kernel_registration_with_receipt_fallback(fallback.clone())
            .is_err(),
        "explicit managed registration must still fail closed without a topology"
    );
    std::env::remove_var("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT");
    std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "invalid");
    assert!(
        super::confirmed_managed_kernel_registration_with_receipt_fallback(fallback).is_err(),
        "a configured invalid topology must still fail closed"
    );
}

#[test]
fn path1_rebuild_freshness_must_match_the_protected_volume_identity() {
    let config = BootstrapConfig {
        process_home: PathBuf::new(),
        chariox_home: PathBuf::new(),
        envelope_path: PathBuf::from(super::state::PROTECTED_MANAGED_BOOTSTRAP_PATH),
        receipt_path: PathBuf::new(),
        manifest_path: PathBuf::new(),
        signature_path: PathBuf::new(),
        public_key_path: PathBuf::new(),
        kernel_binary: PathBuf::new(),
        kernel_host: "127.0.0.1".to_string(),
        kernel_port: 1,
    };
    let envelope = ManagedBootstrapEnvelope {
        schema_version: 3,
        cloud_api_url: "https://cloud.example.test".to_string(),
        environment_id: "environment-1".to_string(),
        token: format!("mkboot_{}", "x".repeat(40)),
        expires_at: "2026-09-27T00:00:00Z".to_string(),
        runtime_release_digest: format!("sha256:{}", "a".repeat(64)),
        managed_repository_root: None,
        provider_rebuild_action_id: Some("12345".to_string()),
        expected_data_volume_serial: Some("12345".to_string()),
        expected_data_volume_size_gb: Some(20),
    };
    let evidence = ManagedKernelFreshnessEvidence {
        schema_version: Some(3),
        linux_boot_id: "01234567-89ab-cdef-0123-456789abcdef".to_string(),
        os_machine_id: "b".repeat(32),
        runtime_release_digest: format!("sha256:{}", "a".repeat(64)),
        runtime_source_commit: "c".repeat(40),
        runtime_source_tree: "d".repeat(40),
        residue_checks: ManagedKernelResidueChecks {
            old_services_absent: true,
            old_processes_absent: true,
            old_state_absent: true,
        },
        data_volume_serial: Some("12345".to_string()),
        data_volume_size_gb: Some(20),
    };

    assert_eq!(
        expected_data_volume_identity(&envelope).unwrap(),
        Some(("12345", 20))
    );
    validate_rebuild_volume_evidence(&config, Some(&envelope), &evidence)
        .expect("protected volume identity is matched");
    let mut missing_serial = evidence.clone();
    missing_serial.data_volume_serial = None;
    assert!(validate_rebuild_volume_evidence(&config, Some(&envelope), &missing_serial).is_err());
    let mut missing_size = evidence.clone();
    missing_size.data_volume_size_gb = None;
    assert!(validate_rebuild_volume_evidence(&config, Some(&envelope), &missing_size).is_err());
    let mut wrong_serial = evidence.clone();
    wrong_serial.data_volume_serial = Some("54321".to_string());
    assert!(validate_rebuild_volume_evidence(&config, Some(&envelope), &wrong_serial).is_err());
    let mut wrong_size = evidence;
    wrong_size.data_volume_size_gb = Some(30);
    assert!(validate_rebuild_volume_evidence(&config, Some(&envelope), &wrong_size).is_err());
}

#[test]
fn pre_reimage_observation_binding_requires_exact_confirmed_generation_and_current_identity() {
    let receipt = BootstrapReceipt {
        schema_version: 3,
        status: BootstrapReceiptStatus::Confirmed,
        environment_id: "environment-1".to_string(),
        machine_id: "machine-1".to_string(),
        kernel_id: "kernel-1".to_string(),
        generation: 4,
        relay_public_key: "relay-public-key".to_string(),
        runtime_release_digest: format!("sha256:{}", "a".repeat(64)),
        managed_repository_root: Some("/srv/path1 managed workspaces".to_string()),
        confirmed_at: Some("2026-09-22T00:00:00Z".to_string()),
        context_plan: None,
        provider_rebuild_action_id: None,
        freshness_evidence: None,
    };
    let registration = ConfirmedManagedKernelRegistration {
        environment_id: receipt.environment_id.clone(),
        machine_id: receipt.machine_id.clone(),
        kernel_id: receipt.kernel_id.clone(),
        context_plan: None,
    };
    let mut config = DaemonConfig::for_tests();
    config.daemon_id = receipt.kernel_id.clone();
    config.host_machine_id = receipt.machine_id.clone();
    config.relay_public_key = receipt.relay_public_key.clone();
    config.cloud_relay = Some(PersistedCloudRelayProfile {
        kernel_id: None,
        kernel_credential: None,
        kernel_public_key_thumbprint: None,
        api_url: "https://cloud.example.test".to_string(),
        user_id: "owner-1".to_string(),
        machine_id: Some(receipt.machine_id.clone()),
        machine_credential: Some(format!("mcred_{}", "b".repeat(40))),
        relay_url: "wss://relay.example.test".to_string(),
        ..PersistedCloudRelayProfile::default()
    });

    validate_pre_reimage_observation_binding(&config, &registration, &receipt, "environment-1", 4)
        .expect("exact current managed generation");
    assert!(validate_pre_reimage_observation_binding(
        &config,
        &registration,
        &receipt,
        "environment-2",
        4,
    )
    .is_err());
    assert!(validate_pre_reimage_observation_binding(
        &config,
        &registration,
        &receipt,
        "environment-1",
        3,
    )
    .is_err());

    let mut stale_registration = registration;
    stale_registration.kernel_id = "kernel-old".to_string();
    assert!(validate_pre_reimage_observation_binding(
        &config,
        &stale_registration,
        &receipt,
        "environment-1",
        4,
    )
    .is_err());
}

#[test]
fn legacy_initial_bootstrap_receipt_defaults_to_generation_one() {
    let receipt: BootstrapReceipt = serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "status": "confirmed",
        "environmentId": "environment-1",
        "machineId": "machine-1",
        "kernelId": "kernel-1",
        "relayPublicKey": "relay-public-key",
        "runtimeReleaseDigest": format!("sha256:{}", "a".repeat(64)),
        "confirmedAt": "2026-09-22T00:00:00Z"
    }))
    .expect("legacy receipt shape");
    assert_eq!(receipt.generation, 1);
}

struct FakeCloud {
    exchange_response: ExchangeResponse,
    exchange_calls: Mutex<Vec<ExchangeRequest>>,
    confirm_calls: Mutex<Vec<ConfirmRequest>>,
    reconcile_calls: Mutex<usize>,
    reconcile_response_override: Mutex<Option<ReconcileManagedBootstrapGrantResponseV1>>,
    fail_reconciliation: Mutex<bool>,
    fail_next_confirm: Mutex<bool>,
    confirm_after_child_marker: Mutex<Option<PathBuf>>,
}

impl FakeCloud {
    fn new(response: ExchangeResponse) -> Self {
        Self {
            exchange_response: response,
            exchange_calls: Mutex::new(Vec::new()),
            confirm_calls: Mutex::new(Vec::new()),
            reconcile_calls: Mutex::new(0),
            reconcile_response_override: Mutex::new(None),
            fail_reconciliation: Mutex::new(false),
            fail_next_confirm: Mutex::new(false),
            confirm_after_child_marker: Mutex::new(None),
        }
    }
}

impl BootstrapCloudClient for FakeCloud {
    fn exchange(
        &self,
        _api_url: &str,
        request: &ExchangeRequest,
    ) -> Result<ExchangeResponse, DaemonError> {
        self.exchange_calls
            .lock()
            .expect("exchange calls")
            .push(request.clone());
        let mut response = self.exchange_response.clone();
        response.environment_id = request.environment_id.clone();
        response.kernel_id = request.kernel_id.clone();
        response.runtime_release_digest = request.runtime_release_digest.clone();
        response.cloud_relay.machine_id = request.machine_id.clone();
        Ok(response)
    }

    fn confirm(
        &self,
        _api_url: &str,
        request: &ConfirmRequest,
    ) -> Result<ConfirmResponse, DaemonError> {
        self.confirm_calls
            .lock()
            .expect("confirm calls")
            .push(request.clone());
        if let Some(marker) = self
            .confirm_after_child_marker
            .lock()
            .expect("confirmation marker")
            .as_ref()
        {
            for _ in 0..50 {
                if marker.exists() {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            if !marker.exists() {
                return Err(DaemonError::LocalTransport {
                    operation: "test managed bootstrap confirm",
                    message: "kernel child has not started".to_string(),
                });
            }
        }
        let mut fail = self.fail_next_confirm.lock().expect("fail next confirm");
        if *fail {
            *fail = false;
            return Err(DaemonError::LocalTransport {
                operation: "test managed bootstrap confirm",
                message: "transient confirmation failure".to_string(),
            });
        }
        Ok(ConfirmResponse {
            confirmed: true,
            observed_state: "awaiting_context".to_string(),
            managed_repository_root: self.exchange_response.managed_repository_root.clone(),
        })
    }

    fn reconcile_managed_bootstrap_grant(
        &self,
        _api_url: &str,
        request: &ReconcileManagedBootstrapGrantRequestV1,
    ) -> Result<ReconcileManagedBootstrapGrantResponseV1, DaemonError> {
        *self
            .reconcile_calls
            .lock()
            .expect("grant reconciliation calls") += 1;
        if *self
            .fail_reconciliation
            .lock()
            .expect("fail reconciliation")
        {
            return Err(DaemonError::LocalTransport {
                operation: "test managed bootstrap reconciliation",
                message: "the retained grant is stale or does not match the original token"
                    .to_string(),
            });
        }
        let response_override = self
            .reconcile_response_override
            .lock()
            .expect("reconciliation response override")
            .clone();
        Ok(
            response_override.unwrap_or_else(|| ReconcileManagedBootstrapGrantResponseV1 {
                protocol_version: request.protocol_version,
                reconciled: true,
                grant_id: "grant-1".to_string(),
                operation_id: "operation-1".to_string(),
                operation_kind: if request.generation == 1 {
                    "CREATE".to_string()
                } else {
                    "REIMAGE".to_string()
                },
                environment_id: request.environment_id.clone(),
                machine_id: request.machine_id.clone(),
                kernel_id: request.kernel_id.clone(),
                generation: request.generation,
                runtime_release_digest: request.runtime_release_digest.clone(),
                managed_repository_root: request.managed_repository_root.clone(),
                data_volume_serial: request.expected_data_volume_serial.clone(),
                data_volume_size_gb: request.expected_data_volume_size_gb,
            }),
        )
    }

    fn report_runtime_identity(
        &self,
        _api_url: &str,
        request: &ManagedKernelRuntimeIdentityReport,
    ) -> Result<RuntimeIdentityReportResponse, DaemonError> {
        Ok(RuntimeIdentityReportResponse {
            accepted: true,
            environment_id: request.environment_id.clone(),
            generation: request.generation,
            observed_at: request.observed_at.clone(),
        })
    }
}

#[cfg(unix)]
#[test]
fn bootstrap_verifies_release_persists_identity_and_profile_then_resumes_without_token() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("complete");
    let previous_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let cloud = FakeCloud::new(fixture.exchange_response());

    let mut prepared = prepare_managed_kernel(&fixture.config, &cloud, fixture.now)
        .expect("managed registration should exchange");
    assert_eq!(prepared.release.digest, fixture.release_digest);
    assert_eq!(prepared.release.kernel_binary, fixture.config.kernel_binary);
    assert!(fixture.config.envelope_path.exists());
    let receipt = BootstrapReceipt::read(&fixture.config.receipt_path)
        .expect("read receipt")
        .expect("receipt exists");
    assert_eq!(receipt.status, BootstrapReceiptStatus::Exchanged);
    assert_eq!(receipt.generation, 1);
    assert_eq!(
        receipt
            .context_plan
            .as_ref()
            .map(ManagedKernelContextPlan::context_id),
        Some("managed_ctx_bootstrap")
    );
    assert_eq!(
        cloud.exchange_calls.lock().expect("exchange calls").len(),
        1
    );
    assert!(cloud
        .confirm_calls
        .lock()
        .expect("confirm calls")
        .is_empty());
    *cloud
        .confirm_after_child_marker
        .lock()
        .expect("confirmation marker") = Some(fixture.kernel_started_marker.clone());
    let previous_topology = std::env::var_os(MANAGED_PROVIDER_TOPOLOGY_ENV);
    std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "shared_host");

    run_kernel_once(
        &fixture.config,
        &prepared.release,
        &mut prepared.confirmation,
        &cloud,
        ManagedProviderTopology::SharedHost,
    )
    .expect("kernel child should start before relay-ready confirmation");
    assert!(prepared.confirmation.is_none());
    assert_eq!(
        fs::read_to_string(&fixture.kernel_started_marker).expect("kernel launch record"),
        "exchanged\nconfirmed\n",
        "the relay-presence child must be replaced by a kernel that loads the confirmed receipt"
    );
    assert!(!fixture.config.envelope_path.exists());
    assert_eq!(
        BootstrapReceipt::read(&fixture.config.receipt_path)
            .expect("read confirmed receipt")
            .expect("confirmed receipt")
            .status,
        BootstrapReceiptStatus::Confirmed
    );
    assert_eq!(cloud.confirm_calls.lock().expect("confirm calls").len(), 1);

    let daemon_config = fs::read_to_string(
        fixture
            .config
            .chariox_home
            .join("daemon")
            .join("config.json"),
    )
    .expect("managed daemon config");
    assert!(daemon_config.contains("mcred_"));
    assert!(!daemon_config.contains(&fixture.token));
    let receipt_bytes = fs::read_to_string(&fixture.config.receipt_path).expect("receipt bytes");
    assert!(!receipt_bytes.contains(&fixture.token));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(
                fixture
                    .config
                    .chariox_home
                    .join("daemon")
                    .join("config.json")
            )
            .expect("daemon config metadata")
            .permissions()
            .mode()
                & 0o777,
            0o600
        );
    }

    let resumed = prepare_managed_kernel(&fixture.config, &cloud, fixture.now)
        .expect("confirmed registration should resume offline");
    assert!(resumed.confirmation.is_none());
    assert_eq!(
        cloud.exchange_calls.lock().expect("exchange calls").len(),
        1
    );
    assert_eq!(cloud.confirm_calls.lock().expect("confirm calls").len(), 1);

    restore_env(MANAGED_PROVIDER_TOPOLOGY_ENV, previous_topology);
    restore_env("CHARIOX_HOME", previous_home);
    fixture.cleanup();
}

#[test]
fn schema_two_bootstrap_persists_the_exact_managed_repository_root() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("schema-two-repository-root");
    let previous_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    fs::write(
        &fixture.config.envelope_path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "cloudApiUrl": "https://cloud.example.test",
            "environmentId": "managed-env-1",
            "token": fixture.token,
            "expiresAt": (fixture.now + chrono::Duration::minutes(1)).to_rfc3339(),
            "runtimeReleaseDigest": fixture.release_digest,
            "managedRepositoryRoot": "/srv/managed workspaces",
        }))
        .unwrap(),
    )
    .unwrap();
    let mut response = fixture.exchange_response();
    response.generation = Some(1);
    response.managed_repository_root = Some("/srv/managed workspaces".to_string());
    let cloud = FakeCloud::new(response);

    let mut prepared = prepare_managed_kernel(&fixture.config, &cloud, fixture.now)
        .expect("schema two bootstrap should exchange");
    assert!(prepared.confirmation.is_some());
    let receipt = BootstrapReceipt::read(&fixture.config.receipt_path)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.schema_version, 2);
    assert_eq!(receipt.generation, 1);
    assert_eq!(
        receipt.managed_repository_root().unwrap(),
        "/srv/managed workspaces"
    );
    prepared
        .confirmation
        .take()
        .expect("confirmation remains pending")
        .confirm(&fixture.config, &cloud, fixture.now)
        .expect("schema two confirmation preserves the repository root");
    assert_eq!(cloud.confirm_calls.lock().expect("confirm calls").len(), 1);
    assert_eq!(
        BootstrapReceipt::read(&fixture.config.receipt_path)
            .unwrap()
            .unwrap()
            .managed_repository_root()
            .unwrap(),
        "/srv/managed workspaces"
    );

    match previous_home {
        Some(value) => std::env::set_var("CHARIOX_HOME", value),
        None => std::env::remove_var("CHARIOX_HOME"),
    }
    fixture.cleanup();
}

#[test]
fn schema_three_exchange_receipt_and_confirmation_preserve_the_exact_repository_root() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("schema-three-repository-root");
    let previous_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let repository_root = "/srv/path1 managed workspaces";
    let envelope = ManagedBootstrapEnvelope {
        schema_version: 3,
        cloud_api_url: "https://cloud.example.test".to_string(),
        environment_id: "managed-env-1".to_string(),
        token: fixture.token.clone(),
        expires_at: (fixture.now + chrono::Duration::minutes(1)).to_rfc3339(),
        runtime_release_digest: fixture.release_digest.clone(),
        managed_repository_root: Some(repository_root.to_string()),
        provider_rebuild_action_id: None,
        expected_data_volume_serial: Some("12345".to_string()),
        expected_data_volume_size_gb: Some(20),
    };
    let identity = ManagedRuntimeIdentity {
        machine_id: "machine-1".to_string(),
        kernel_id: "kernel-1".to_string(),
        relay_public_key: "relay-public-key".to_string(),
    };
    let mut config = fixture.config.clone();
    config.envelope_path = PathBuf::from(super::state::PROTECTED_MANAGED_BOOTSTRAP_PATH);
    let mut response = fixture.exchange_response();
    response.generation = Some(1);
    response.managed_repository_root = Some(repository_root.to_string());
    let cloud = FakeCloud::new(response);
    let release = verify_release(
        &fixture.config.manifest_path,
        &fixture.config.signature_path,
        &fixture.config.public_key_path,
        &fixture.release_digest,
        &fixture.config.kernel_binary,
    )
    .expect("fixture release is verified");

    let pending =
        super::begin_registration(&config, &cloud, fixture.now, &envelope, &identity, &release)
            .expect("schema three exchange succeeds")
            .expect("exchange requires confirmation");
    let receipt = BootstrapReceipt::read(&fixture.config.receipt_path)
        .expect("read exchanged receipt")
        .expect("receipt is persisted");
    assert_eq!(receipt.schema_version, 3);
    assert_eq!(receipt.generation, 1);
    assert_eq!(receipt.managed_repository_root().unwrap(), repository_root);
    assert_eq!(
        cloud.exchange_calls.lock().expect("exchange calls").len(),
        1
    );

    pending
        .confirm(&config, &cloud, fixture.now)
        .expect("schema three confirmation preserves the repository root");
    let confirmed = BootstrapReceipt::read(&fixture.config.receipt_path)
        .expect("read confirmed receipt")
        .expect("confirmed receipt remains persisted");
    assert_eq!(confirmed.status, BootstrapReceiptStatus::Confirmed);
    assert_eq!(
        confirmed.managed_repository_root().unwrap(),
        repository_root
    );
    assert_eq!(cloud.confirm_calls.lock().expect("confirm calls").len(), 1);
    assert_eq!(
        cloud.confirm_calls.lock().expect("confirm calls")[0]
            .freshness_evidence
            .as_ref(),
        None,
        "generation one does not claim reimage evidence"
    );

    restore_env("CHARIOX_HOME", previous_home);
    fixture.cleanup();
}

#[test]
fn schema_three_generation_two_exchange_rejects_a_stale_repository_root() {
    let fixture = Fixture::new("schema-three-stale-repository-root");
    let root = "/srv/path1 managed workspaces";
    let envelope = path1_test_envelope(&fixture.release_digest, root);
    let identity = path1_test_identity();
    let mut response = fixture.exchange_response();
    response.environment_id = envelope.environment_id.clone();
    response.kernel_id = identity.kernel_id.clone();
    response.generation = Some(2);
    response.runtime_release_digest = envelope.runtime_release_digest.clone();
    response.managed_repository_root = Some("/srv/old managed workspaces".to_string());
    response.cloud_relay.machine_id = identity.machine_id.clone();

    assert!(super::validate_exchange_response(&envelope, &identity, &response).is_err());

    fixture.cleanup();
}

#[test]
fn path1_exchange_requires_cloud_generation_while_legacy_envelopes_keep_generation_one_fallback() {
    let fixture = Fixture::new("path1-exchange-explicit-generation");
    let identity = path1_test_identity();
    let mut envelope =
        path1_test_envelope(&fixture.release_digest, "/srv/path1 managed workspaces");
    let mut response = fixture.exchange_response();
    response.environment_id = envelope.environment_id.clone();
    response.kernel_id = identity.kernel_id.clone();
    response.runtime_release_digest = envelope.runtime_release_digest.clone();
    response.managed_repository_root = envelope.managed_repository_root.clone();
    response.cloud_relay.machine_id = identity.machine_id.clone();
    response.generation = None;

    assert!(
        super::validate_exchange_response(&envelope, &identity, &response).is_err(),
        "Path-1 must not bind a grant to an invented generation-one default"
    );

    envelope.schema_version = 2;
    envelope.expected_data_volume_serial = None;
    envelope.expected_data_volume_size_gb = None;
    assert_eq!(
        super::validate_exchange_response(&envelope, &identity, &response)
            .expect("legacy schema two exchange retains generation-one compatibility"),
        1
    );
    fixture.cleanup();
}

#[test]
fn schema_three_generation_two_receipt_roundtrips_and_rejects_incomplete_freshness() {
    let fixture = Fixture::new("schema-three-generation-two-receipt");
    let evidence = path1_test_evidence(&fixture.release_digest);
    let receipt = path1_test_receipt(
        &fixture.release_digest,
        2,
        BootstrapReceiptStatus::Exchanged,
        Some(evidence.clone()),
    );
    receipt
        .persist(&fixture.config.receipt_path)
        .expect("persist generation-two freshness evidence");
    assert_eq!(
        BootstrapReceipt::read(&fixture.config.receipt_path)
            .expect("read durable generation-two receipt")
            .expect("generation-two receipt remains present"),
        receipt
    );

    let mut incomplete = serde_json::to_value(&receipt).expect("serialize receipt fixture");
    incomplete["freshnessEvidence"]["dataVolumeSizeGb"] = serde_json::Value::Null;
    fs::write(
        &fixture.config.receipt_path,
        serde_json::to_vec(&incomplete).expect("serialize incomplete receipt"),
    )
    .expect("write incomplete receipt fixture");
    assert!(BootstrapReceipt::read(&fixture.config.receipt_path).is_err());

    let generation_one_with_rebuild_proof = path1_test_receipt(
        &fixture.release_digest,
        1,
        BootstrapReceiptStatus::Exchanged,
        Some(evidence),
    );
    fs::write(
        &fixture.config.receipt_path,
        serde_json::to_vec(&generation_one_with_rebuild_proof)
            .expect("serialize generation-one receipt with reimage proof"),
    )
    .expect("write generation-one receipt fixture");
    assert!(BootstrapReceipt::read(&fixture.config.receipt_path).is_err());

    fixture.cleanup();
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn path1_generation_two_no_action_confirmation_carries_durable_signed_volume_evidence() {
    let _env = crate::env_lock::lock();
    let fixture = AttestedReleaseFixture::new(
        "generation-two-confirm",
        &"a".repeat(40),
        &"b".repeat(40),
        "x86_64-unknown-linux-gnu",
    );
    let _restore = EnvironmentRestoreGuard::capture(
        RELEASE_EVIDENCE_ENV_NAMES
            .into_iter()
            .chain([super::state::TRUSTED_BUILDER_PUBLIC_KEY_ENV]),
    );
    configure_path1_builder_trust(&fixture);
    let mut config = fixture.config.clone();
    config.envelope_path = PathBuf::from(super::state::PROTECTED_MANAGED_BOOTSTRAP_PATH);
    let repository_root = "/srv/path1 managed workspaces";
    let envelope = path1_test_envelope(&fixture.release_digest, repository_root);
    let evidence = path1_test_evidence(&fixture.release_digest);
    let receipt = path1_test_receipt(
        &fixture.release_digest,
        2,
        BootstrapReceiptStatus::Exchanged,
        Some(evidence.clone()),
    );
    receipt
        .persist(&config.receipt_path)
        .expect("persist before simulated restart");
    let receipt_after_restart = BootstrapReceipt::read(&config.receipt_path)
        .expect("read receipt after simulated restart")
        .expect("exchanged receipt remains present");
    persist_path1_grant_binding(&config, &envelope, &receipt_after_restart);
    let cloud = FakeCloud::new(path1_test_cloud_response(
        &fixture.release_digest,
        repository_root,
    ));

    super::confirm_registration(
        &config,
        &cloud,
        Utc.with_ymd_and_hms(2026, 9, 27, 0, 0, 0).unwrap(),
        &envelope,
        receipt_after_restart,
        &path1_test_profile(),
    )
    .expect("generation-two confirmation accepts signed, volume-bound evidence");

    let confirm_calls = cloud.confirm_calls.lock().expect("confirm calls");
    assert_eq!(confirm_calls.len(), 1);
    assert_eq!(
        confirm_calls[0].freshness_evidence.as_ref(),
        Some(&evidence)
    );
    drop(confirm_calls);
    let confirmed = BootstrapReceipt::read(&config.receipt_path)
        .expect("read confirmed receipt")
        .expect("confirmed receipt remains present");
    assert_eq!(confirmed.generation, 2);
    assert_eq!(confirmed.status, BootstrapReceiptStatus::Confirmed);
    assert!(confirmed.freshness_evidence.is_none());
    assert!(confirmed.provider_rebuild_action_id.is_none());

    fixture.cleanup();
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn path1_generation_two_no_action_confirmation_rejects_missing_or_mismatched_proof() {
    let _env = crate::env_lock::lock();
    let fixture = AttestedReleaseFixture::new(
        "generation-two-confirm-rejections",
        &"a".repeat(40),
        &"b".repeat(40),
        "x86_64-unknown-linux-gnu",
    );
    let _restore = EnvironmentRestoreGuard::capture(
        RELEASE_EVIDENCE_ENV_NAMES
            .into_iter()
            .chain([super::state::TRUSTED_BUILDER_PUBLIC_KEY_ENV]),
    );
    configure_path1_builder_trust(&fixture);
    let mut config = fixture.config.clone();
    config.envelope_path = PathBuf::from(super::state::PROTECTED_MANAGED_BOOTSTRAP_PATH);
    let repository_root = "/srv/path1 managed workspaces";
    let envelope = path1_test_envelope(&fixture.release_digest, repository_root);
    let valid = path1_test_evidence(&fixture.release_digest);
    let mut missing_serial = valid.clone();
    missing_serial.data_volume_serial = None;
    let mut wrong_volume = valid.clone();
    wrong_volume.data_volume_serial = Some("54321".to_string());
    let mut wrong_source = valid.clone();
    wrong_source.runtime_source_commit = "c".repeat(40);

    for (label, evidence) in [
        ("missing evidence", None),
        ("incomplete volume identity", Some(missing_serial)),
        ("wrong volume identity", Some(wrong_volume)),
        ("wrong signed source", Some(wrong_source)),
    ] {
        let receipt = path1_test_receipt(
            &fixture.release_digest,
            2,
            BootstrapReceiptStatus::Exchanged,
            evidence,
        );
        persist_path1_grant_binding(&config, &envelope, &receipt);
        let cloud = FakeCloud::new(path1_test_cloud_response(
            &fixture.release_digest,
            repository_root,
        ));
        assert!(
            super::confirm_registration(
                &config,
                &cloud,
                Utc.with_ymd_and_hms(2026, 9, 27, 0, 0, 0).unwrap(),
                &envelope,
                receipt,
                &path1_test_profile(),
            )
            .is_err(),
            "confirmation must reject {label}"
        );
        assert!(
            cloud
                .confirm_calls
                .lock()
                .expect("confirm calls")
                .is_empty(),
            "Cloud confirmation must not receive {label}"
        );
    }

    fixture.cleanup();
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn path1_confirmed_generation_two_retry_preserves_validated_freshness_evidence() {
    let _env = crate::env_lock::lock();
    let fixture = AttestedReleaseFixture::new(
        "generation-two-confirmed-retry",
        &"a".repeat(40),
        &"b".repeat(40),
        "x86_64-unknown-linux-gnu",
    );
    let _restore = EnvironmentRestoreGuard::capture(
        RELEASE_EVIDENCE_ENV_NAMES
            .into_iter()
            .chain([super::state::TRUSTED_BUILDER_PUBLIC_KEY_ENV]),
    );
    configure_path1_builder_trust(&fixture);
    let mut config = fixture.config.clone();
    config.envelope_path = PathBuf::from(super::state::PROTECTED_MANAGED_BOOTSTRAP_PATH);
    let repository_root = "/srv/path1 managed workspaces";
    let envelope = path1_test_envelope(&fixture.release_digest, repository_root);
    let evidence = path1_test_evidence(&fixture.release_digest);
    let receipt = path1_test_receipt(
        &fixture.release_digest,
        2,
        BootstrapReceiptStatus::Confirmed,
        Some(evidence.clone()),
    );
    receipt
        .persist(&config.receipt_path)
        .expect("persist confirmed retry receipt");
    let restarted_receipt = BootstrapReceipt::read(&config.receipt_path)
        .expect("read confirmed retry receipt")
        .expect("confirmed retry receipt remains present");
    let release = verify_release(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &fixture.release_digest,
        &config.kernel_binary,
    )
    .expect("fixture release signature verifies");

    let resumed = super::ensure_rebuild_freshness_evidence(
        &config,
        Some(&envelope),
        restarted_receipt,
        &release,
    )
    .expect("confirmed retry validates its existing proof");

    assert_eq!(resumed.status, BootstrapReceiptStatus::Confirmed);
    assert_eq!(resumed.generation, 2);
    assert_eq!(resumed.freshness_evidence, Some(evidence));

    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn legacy_confirmed_path1_receipt_restores_without_consuming_its_protected_envelope() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("stale-confirmed-generation-one-receipt");
    let _restore = EnvironmentRestoreGuard::capture(["HOME", "CHARIOX_HOME"]);
    std::env::set_var("HOME", &fixture.config.process_home);
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let profile = path1_test_profile();
    crate::config::persist_managed_cloud_relay_profile(profile)
        .expect("persist Cloud profile for restore path");
    let mut config = fixture.config.clone();
    config.envelope_path = PathBuf::from(super::state::PROTECTED_MANAGED_BOOTSTRAP_PATH);
    let envelope = path1_test_envelope(&fixture.release_digest, "/srv/path1 managed workspaces");
    let receipt = path1_test_receipt(
        &fixture.release_digest,
        1,
        BootstrapReceiptStatus::Confirmed,
        None,
    );
    receipt
        .persist(&config.receipt_path)
        .expect("persist released legacy receipt");
    let identity = path1_test_identity();
    let release = verify_release(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &fixture.release_digest,
        &config.kernel_binary,
    )
    .expect("fixture release signature verifies");
    let cloud = FakeCloud::new(path1_test_cloud_response(
        &fixture.release_digest,
        "/srv/path1 managed workspaces",
    ));

    assert!(
        super::resume_registration(
            &config,
            Some(&envelope),
            receipt.clone(),
            &identity,
            &release,
        )
        .is_err(),
        "an unbound legacy receipt must fail closed without authoritative reconciliation"
    );

    super::reconcile_legacy_confirmed_grant_binding(&config, &cloud, &envelope, &receipt)
        .expect("read-only Cloud reconciliation binds the exact legacy grant");

    let resumed =
        super::resume_registration(&config, Some(&envelope), receipt, &identity, &release)
            .expect("reconciled confirmed receipt remains a valid restore authority");
    assert!(resumed.is_none());
    assert!(
        ManagedBootstrapGrantBinding::read_for_receipt(&config.receipt_path)
            .expect("inspect reconciled legacy binding")
            .is_some(),
        "the exact legacy grant binding must survive the restart"
    );
    assert_eq!(
        *cloud.reconcile_calls.lock().expect("reconciliation calls"),
        1
    );
    assert!(cloud
        .exchange_calls
        .lock()
        .expect("exchange calls")
        .is_empty());
    assert!(cloud
        .confirm_calls
        .lock()
        .expect("confirm calls")
        .is_empty());
    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn legacy_confirmed_schema_one_and_two_receipts_restore_with_protected_envelope() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("legacy-confirmed-schema-one-two-restore");
    let _restore = EnvironmentRestoreGuard::capture(["HOME", "CHARIOX_HOME"]);
    std::env::set_var("HOME", &fixture.config.process_home);
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    crate::config::persist_managed_cloud_relay_profile(path1_test_profile())
        .expect("persist Cloud profile for restore path");
    let mut config = fixture.config.clone();
    config.envelope_path = PathBuf::from(super::state::PROTECTED_MANAGED_BOOTSTRAP_PATH);
    let identity = path1_test_identity();
    let release = verify_release(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &fixture.release_digest,
        &config.kernel_binary,
    )
    .expect("fixture release signature verifies");
    let envelope = path1_test_envelope(&fixture.release_digest, "/home/chariox");
    let cloud = FakeCloud::new(path1_test_cloud_response(
        &fixture.release_digest,
        "/home/chariox",
    ));

    for (schema_version, repository_root) in [(1, None), (2, Some("/home/chariox"))] {
        let mut receipt = path1_test_receipt(
            &fixture.release_digest,
            1,
            BootstrapReceiptStatus::Confirmed,
            None,
        );
        receipt.schema_version = schema_version;
        receipt.managed_repository_root = repository_root.map(str::to_string);
        receipt
            .persist(&config.receipt_path)
            .expect("persist released legacy receipt");

        super::reconcile_legacy_confirmed_grant_binding(&config, &cloud, &envelope, &receipt)
            .expect("read-only Cloud reconciliation binds the exact legacy grant");

        let resumed =
            super::resume_registration(&config, Some(&envelope), receipt, &identity, &release)
                .expect("legacy confirmed receipt remains a restore authority");
        assert!(resumed.is_none());
        assert!(
            ManagedBootstrapGrantBinding::read_for_receipt(&config.receipt_path)
                .expect("inspect legacy binding")
                .is_some(),
            "legacy schema {schema_version} must bind only the grant proven by Cloud"
        );
        fs::remove_file(
            super::state::managed_bootstrap_grant_binding_path(&config.receipt_path)
                .expect("binding path"),
        )
        .expect("clear binding before next legacy schema fixture");
    }

    assert_eq!(
        *cloud.reconcile_calls.lock().expect("reconciliation calls"),
        2
    );
    assert!(cloud
        .exchange_calls
        .lock()
        .expect("exchange calls")
        .is_empty());
    assert!(cloud
        .confirm_calls
        .lock()
        .expect("confirm calls")
        .is_empty());

    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn exchanged_receipt_completes_pending_binding_after_restart_without_reexchange() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("path1-pending-binding-resume");
    let _restore = EnvironmentRestoreGuard::capture(["HOME", "CHARIOX_HOME"]);
    std::env::set_var("HOME", &fixture.config.process_home);
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    crate::config::persist_managed_cloud_relay_profile(path1_test_profile())
        .expect("persist Cloud profile for restart");
    let mut config = fixture.config.clone();
    config.envelope_path = PathBuf::from(super::state::PROTECTED_MANAGED_BOOTSTRAP_PATH);
    let envelope = path1_test_envelope(&fixture.release_digest, "/srv/path1 managed workspaces");
    let identity = path1_test_identity();
    super::persist_or_validate_grant_binding(&config, &envelope, &identity)
        .expect("persist pending binding before exchange");
    let receipt = path1_test_receipt(
        &fixture.release_digest,
        1,
        BootstrapReceiptStatus::Exchanged,
        None,
    );
    receipt
        .persist(&config.receipt_path)
        .expect("persist exchange receipt before completing binding");
    let release = verify_release(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &fixture.release_digest,
        &config.kernel_binary,
    )
    .expect("fixture release signature verifies");

    let resumed = super::resume_registration(
        &config,
        Some(&envelope),
        receipt.clone(),
        &identity,
        &release,
    )
    .expect("resume uses persisted exchange receipt");
    assert!(resumed.is_some());
    let binding = ManagedBootstrapGrantBinding::read_for_receipt(&config.receipt_path)
        .expect("read completed binding")
        .expect("resume completed the pending binding");
    assert_eq!(
        binding,
        ManagedBootstrapGrantBinding::for_receipt(&envelope, &receipt)
    );

    fs::remove_file(&config.receipt_path).expect("simulate missing receipt");
    assert!(super::persist_or_validate_grant_binding(&config, &envelope, &identity).is_err());

    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn cloud_reconciliation_response_must_match_every_retained_receipt_and_volume_claim() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("path1-reconciliation-response-binding");
    let receipt = path1_test_receipt(
        &fixture.release_digest,
        2,
        BootstrapReceiptStatus::Confirmed,
        None,
    );
    let expected_root = "/srv/path1 managed workspaces";
    let mut valid = ReconcileManagedBootstrapGrantResponseV1 {
        protocol_version: 1,
        reconciled: true,
        grant_id: "grant-1".to_string(),
        operation_id: "operation-1".to_string(),
        operation_kind: "REIMAGE".to_string(),
        environment_id: receipt.environment_id.clone(),
        machine_id: receipt.machine_id.clone(),
        kernel_id: receipt.kernel_id.clone(),
        generation: receipt.generation,
        runtime_release_digest: receipt.runtime_release_digest.clone(),
        managed_repository_root: expected_root.to_string(),
        data_volume_serial: "12345".to_string(),
        data_volume_size_gb: 20,
    };
    super::validate_reconciled_grant_response(&valid, &receipt, expected_root, "12345", 20)
        .expect("exact current grant proof is accepted");

    let mut changed_environment = valid.clone();
    changed_environment.environment_id = "other-environment".to_string();
    let mut changed_machine = valid.clone();
    changed_machine.machine_id = "other-machine".to_string();
    let mut changed_kernel = valid.clone();
    changed_kernel.kernel_id = "other-kernel".to_string();
    let mut changed_generation = valid.clone();
    changed_generation.generation = 1;
    let mut changed_release = valid.clone();
    changed_release.runtime_release_digest = format!("sha256:{}", "d".repeat(64));
    let mut changed_root = valid.clone();
    changed_root.managed_repository_root = "/srv/other".to_string();
    let mut changed_volume_serial = valid.clone();
    changed_volume_serial.data_volume_serial = "54321".to_string();
    let mut changed_volume_size = valid.clone();
    changed_volume_size.data_volume_size_gb = 30;
    let mut changed_operation = valid.clone();
    changed_operation.operation_kind = "CREATE".to_string();
    let mut changed_protocol = valid.clone();
    changed_protocol.protocol_version = 2;
    let mut not_reconciled = valid.clone();
    not_reconciled.reconciled = false;

    for (label, response) in [
        ("environment", changed_environment),
        ("machine", changed_machine),
        ("kernel", changed_kernel),
        ("generation", changed_generation),
        ("release", changed_release),
        ("repository root", changed_root),
        ("Volume serial", changed_volume_serial),
        ("Volume size", changed_volume_size),
        ("operation kind", changed_operation),
        ("protocol version", changed_protocol),
        ("reconciliation status", not_reconciled),
    ] {
        assert!(
            super::validate_reconciled_grant_response(
                &response,
                &receipt,
                expected_root,
                "12345",
                20,
            )
            .is_err(),
            "Cloud reconciliation must reject a different {label}"
        );
    }

    valid.grant_id.clear();
    assert!(
        super::validate_reconciled_grant_response(&valid, &receipt, expected_root, "12345", 20)
            .is_err(),
        "Cloud reconciliation must return a grant identity"
    );

    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn prepare_rejects_legacy_confirmed_grant_reconciliation_mismatches_without_mutation() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("prepare-legacy-grant-reconciliation-rejections");
    let _restore = EnvironmentRestoreGuard::capture(["HOME", "CHARIOX_HOME"]);
    std::env::set_var("HOME", &fixture.config.process_home);
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);

    let identity = crate::config::load_or_create_managed_runtime_identity(
        &fixture.config.kernel_host,
        fixture.config.kernel_port,
    )
    .expect("create managed identity before the preparation snapshot");
    let mut profile = path1_test_profile();
    profile.machine_id = Some(identity.machine_id.clone());
    crate::config::persist_managed_cloud_relay_profile(profile)
        .expect("persist Cloud profile for confirmed restore");

    let repository_root = "/srv/path1 managed workspaces";
    let envelope = path1_test_envelope(&fixture.release_digest, repository_root);
    let mut receipt = path1_test_receipt(
        &fixture.release_digest,
        2,
        BootstrapReceiptStatus::Confirmed,
        None,
    );
    receipt.machine_id = identity.machine_id;
    receipt.kernel_id = identity.kernel_id;
    receipt.relay_public_key = identity.relay_public_key;
    receipt
        .persist(&fixture.config.receipt_path)
        .expect("persist legacy confirmed receipt");

    let mut config = fixture.config.clone();
    config.envelope_path = PathBuf::from(super::state::PROTECTED_MANAGED_BOOTSTRAP_PATH);
    let receipt_path_before = fs::read(&config.receipt_path).expect("snapshot confirmed receipt");
    let profile_path = DaemonConfig::default_daemon_config_path();
    let profile_before = fs::read(&profile_path).expect("snapshot Cloud profile");
    let binding_path = super::state::managed_bootstrap_grant_binding_path(&config.receipt_path)
        .expect("grant binding path");
    assert!(
        !binding_path.exists(),
        "legacy receipt starts without a binding sidecar"
    );

    let valid_cloud = FakeCloud::new(path1_test_cloud_response(
        &fixture.release_digest,
        repository_root,
    ));
    let prepared = super::prepare_managed_kernel_with_documents_for_test(
        &config,
        &valid_cloud,
        fixture.now,
        receipt.clone(),
        envelope.clone(),
    )
    .expect("the shared prepare orchestration accepts the exact legacy grant proof");
    assert!(prepared.confirmation.is_none());
    assert_eq!(
        *valid_cloud.reconcile_calls.lock().expect("reconcile calls"),
        1
    );
    assert!(valid_cloud
        .exchange_calls
        .lock()
        .expect("exchange calls")
        .is_empty());
    assert!(valid_cloud
        .confirm_calls
        .lock()
        .expect("confirm calls")
        .is_empty());
    assert_eq!(
        fs::read(&config.receipt_path).expect("receipt after valid prepare"),
        receipt_path_before
    );
    assert_eq!(
        fs::read(&profile_path).expect("profile after valid prepare"),
        profile_before
    );
    assert!(
        binding_path.exists(),
        "the valid proof persists its binding sidecar"
    );
    fs::remove_file(&binding_path).expect("restore legacy unbound state before rejection cases");

    let assert_rejected_without_mutation =
        |cloud: &FakeCloud, candidate_envelope: &ManagedBootstrapEnvelope, label: &str| {
            assert!(
                super::prepare_managed_kernel_with_documents_for_test(
                    &config,
                    cloud,
                    fixture.now,
                    receipt.clone(),
                    candidate_envelope.clone(),
                )
                .is_err(),
                "prepare must reject {label}"
            );
            assert_eq!(
                fs::read(&config.receipt_path).expect("receipt after rejected prepare"),
                receipt_path_before,
                "rejected {label} must not rewrite the receipt"
            );
            assert!(
                !binding_path.exists(),
                "rejected {label} must not persist a grant-binding sidecar"
            );
            assert_eq!(
                fs::read(&profile_path).expect("profile after rejected prepare"),
                profile_before,
                "rejected {label} must not rewrite the Cloud profile"
            );
            assert_eq!(*cloud.reconcile_calls.lock().expect("reconcile calls"), 1);
            assert!(cloud
                .exchange_calls
                .lock()
                .expect("exchange calls")
                .is_empty());
            assert!(cloud
                .confirm_calls
                .lock()
                .expect("confirm calls")
                .is_empty());
        };

    let mut substituted_token_envelope = envelope.clone();
    substituted_token_envelope.token = format!("mkboot_{}", "z".repeat(43));
    let stale_grant_cloud = FakeCloud::new(path1_test_cloud_response(
        &fixture.release_digest,
        repository_root,
    ));
    *stale_grant_cloud
        .fail_reconciliation
        .lock()
        .expect("configure stale grant") = true;
    assert_rejected_without_mutation(
        &stale_grant_cloud,
        &substituted_token_envelope,
        "a substituted token with otherwise identical claims",
    );

    let valid_response = ReconcileManagedBootstrapGrantResponseV1 {
        protocol_version: 1,
        reconciled: true,
        grant_id: "grant-1".to_string(),
        operation_id: "operation-1".to_string(),
        operation_kind: "REIMAGE".to_string(),
        environment_id: receipt.environment_id.clone(),
        machine_id: receipt.machine_id.clone(),
        kernel_id: receipt.kernel_id.clone(),
        generation: receipt.generation,
        runtime_release_digest: receipt.runtime_release_digest.clone(),
        managed_repository_root: repository_root.to_string(),
        data_volume_serial: "12345".to_string(),
        data_volume_size_gb: 20,
    };
    let mut wrong_environment = valid_response.clone();
    wrong_environment.environment_id = "other-environment".to_string();
    let mut wrong_machine = valid_response.clone();
    wrong_machine.machine_id = "other-machine".to_string();
    let mut wrong_kernel = valid_response.clone();
    wrong_kernel.kernel_id = "other-kernel".to_string();
    let mut wrong_generation = valid_response.clone();
    wrong_generation.generation = 1;
    let mut wrong_release = valid_response.clone();
    wrong_release.runtime_release_digest = format!("sha256:{}", "d".repeat(64));
    let mut wrong_root = valid_response.clone();
    wrong_root.managed_repository_root = "/srv/other".to_string();
    let mut wrong_volume_serial = valid_response.clone();
    wrong_volume_serial.data_volume_serial = "54321".to_string();
    let mut wrong_volume_size = valid_response;
    wrong_volume_size.data_volume_size_gb = 30;

    for (label, response) in [
        ("environment identity", wrong_environment),
        ("machine identity", wrong_machine),
        ("kernel identity", wrong_kernel),
        ("generation", wrong_generation),
        ("release digest", wrong_release),
        ("repository root", wrong_root),
        ("Volume serial", wrong_volume_serial),
        ("Volume size", wrong_volume_size),
    ] {
        let cloud = FakeCloud::new(path1_test_cloud_response(
            &fixture.release_digest,
            repository_root,
        ));
        *cloud
            .reconcile_response_override
            .lock()
            .expect("reconciliation response override") = Some(response);
        assert_rejected_without_mutation(&cloud, &envelope, label);
    }

    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn path1_confirmed_generation_two_recovers_after_ack_with_cleared_freshness() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("path1-confirmed-generation-two-post-ack-recovery");
    let _restore = EnvironmentRestoreGuard::capture(["HOME", "CHARIOX_HOME"]);
    std::env::set_var("HOME", &fixture.config.process_home);
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    crate::config::persist_managed_cloud_relay_profile(path1_test_profile())
        .expect("persist Cloud profile for restore path");
    let mut config = fixture.config.clone();
    config.envelope_path = PathBuf::from(super::state::PROTECTED_MANAGED_BOOTSTRAP_PATH);
    let envelope = path1_test_envelope(&fixture.release_digest, "/srv/path1 managed workspaces");
    let mut envelope = envelope;
    envelope.provider_rebuild_action_id = Some("12345".to_string());
    let mut receipt = path1_test_receipt(
        &fixture.release_digest,
        2,
        BootstrapReceiptStatus::Confirmed,
        None,
    );
    receipt.provider_rebuild_action_id = None;
    receipt.freshness_evidence = None;
    receipt
        .persist(&config.receipt_path)
        .expect("persist Cloud-confirmed generation-two receipt");
    persist_path1_grant_binding(&config, &envelope, &receipt);
    let identity = path1_test_identity();
    let release = verify_release(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &fixture.release_digest,
        &config.kernel_binary,
    )
    .expect("fixture release signature verifies");

    let cloud = FakeCloud::new(path1_test_cloud_response(
        &fixture.release_digest,
        "/srv/path1 managed workspaces",
    ));
    super::reconcile_legacy_confirmed_grant_binding(&config, &cloud, &envelope, &receipt)
        .expect("existing binding avoids a redundant Cloud reconciliation");
    super::validate_envelope_receipt_compatibility(&envelope, &receipt)
        .expect("confirmed receipt may have cleared rebuild action after Cloud ACK");
    let resumed =
        super::resume_registration(&config, Some(&envelope), receipt, &identity, &release)
            .expect("bound confirmed generation-two receipt restores without reconfirming");
    assert!(resumed.is_none());
    let binding = ManagedBootstrapGrantBinding::read_for_receipt(&config.receipt_path)
        .expect("read durable grant binding")
        .expect("post-ACK grant binding survives reboot");
    assert_eq!(binding.generation, Some(2));
    assert!(
        !serde_json::to_string(&binding)
            .expect("serialize grant binding")
            .contains(&envelope.token),
        "the sidecar must never persist the bearer token"
    );
    assert_eq!(
        *cloud.reconcile_calls.lock().expect("reconciliation calls"),
        0
    );
    assert!(cloud
        .exchange_calls
        .lock()
        .expect("exchange calls")
        .is_empty());
    assert!(cloud
        .confirm_calls
        .lock()
        .expect("confirm calls")
        .is_empty());
    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn path1_grant_binding_survives_an_in_place_release_update() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("path1-grant-binding-release-update");
    let config = fixture.config.clone();
    let envelope = path1_test_envelope(&fixture.release_digest, "/srv/path1 managed workspaces");
    let receipt = path1_test_receipt(
        &fixture.release_digest,
        2,
        BootstrapReceiptStatus::Confirmed,
        None,
    );
    // A machine bound before in-place updates carries a schema 1 binding of its
    // provisioned release; the upgrader installs the target receipt before the
    // new supervisor first reads that binding.
    ManagedBootstrapGrantBinding::legacy_for_receipt(&envelope, &receipt)
        .persist_for_receipt(&config.receipt_path)
        .expect("persist legacy binding");
    let mut updated = receipt.clone();
    updated.runtime_release_digest = format!("sha256:{}", "e".repeat(64));
    let mut other_generation = updated.clone();
    other_generation.generation = 3;
    assert!(
        super::validate_receipt_envelope_grant_binding(&config, &envelope, &other_generation)
            .is_err(),
        "a legacy binding still binds the receipt's identity"
    );
    super::validate_envelope_receipt_compatibility(&envelope, &updated)
        .expect("a confirmed machine may run a release other than the provisioned one");
    super::validate_receipt_envelope_grant_binding(&config, &envelope, &updated)
        .expect("the legacy binding is accepted under the updated receipt");
    assert_eq!(
        ManagedBootstrapGrantBinding::read_for_receipt(&config.receipt_path)
            .expect("read binding")
            .expect("binding exists"),
        ManagedBootstrapGrantBinding::for_receipt(&envelope, &updated),
        "the legacy binding is rebound by identity"
    );
    super::validate_receipt_envelope_grant_binding(&config, &envelope, &receipt)
        .expect("a later release change keeps the grant binding");

    let mut exchanged = updated.clone();
    exchanged.status = BootstrapReceiptStatus::Exchanged;
    exchanged.confirmed_at = None;
    assert!(
        super::validate_envelope_receipt_compatibility(&envelope, &exchanged).is_err(),
        "before confirmation the receipt must still name the provisioned release"
    );
    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn path1_grant_binding_rejects_distinct_token_and_claim_substitution() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("path1-grant-binding-substitution");
    let config = fixture.config.clone();
    let original = path1_test_envelope(&fixture.release_digest, "/srv/path1 managed workspaces");
    let receipt = path1_test_receipt(
        &fixture.release_digest,
        2,
        BootstrapReceiptStatus::Confirmed,
        None,
    );
    persist_path1_grant_binding(&config, &original, &receipt);

    let mut distinct_token = original.clone();
    distinct_token.token = format!("mkboot_{}", "y".repeat(43));
    let mut changed_root = original.clone();
    changed_root.managed_repository_root = Some("/srv/other managed workspaces".to_string());
    let mut changed_volume_serial = original.clone();
    changed_volume_serial.expected_data_volume_serial = Some("54321".to_string());
    let mut changed_volume_size = original.clone();
    changed_volume_size.expected_data_volume_size_gb = Some(30);
    let mut changed_release = original.clone();
    changed_release.runtime_release_digest = format!("sha256:{}", "d".repeat(64));
    let mut changed_generation = receipt.clone();
    changed_generation.generation = 1;
    let mut changed_machine = receipt.clone();
    changed_machine.machine_id = "different-machine".to_string();
    let mut changed_kernel = receipt.clone();
    changed_kernel.kernel_id = "different-kernel".to_string();
    let mut changed_relay_key = receipt.clone();
    changed_relay_key.relay_public_key = "different-relay-public-key".to_string();

    for (label, envelope, receipt) in [
        (
            "different token with identical claims",
            distinct_token,
            receipt.clone(),
        ),
        ("repository root", changed_root, receipt.clone()),
        ("data-volume serial", changed_volume_serial, receipt.clone()),
        ("data-volume size", changed_volume_size, receipt.clone()),
        ("release digest", changed_release, receipt.clone()),
        ("generation", original.clone(), changed_generation),
        ("runtime identity", original.clone(), changed_machine),
        ("kernel identity", original.clone(), changed_kernel),
        ("relay public key", original.clone(), changed_relay_key),
    ] {
        assert!(
            super::validate_receipt_envelope_grant_binding(&config, &envelope, &receipt).is_err(),
            "grant binding must reject substituted {label}"
        );
    }

    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn confirmed_path1_receipt_does_not_delete_a_same_claims_different_token_envelope() {
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("path1-confirmed-different-token-preserves-envelope");
    let _restore = EnvironmentRestoreGuard::capture(["HOME", "CHARIOX_HOME"]);
    std::env::set_var("HOME", &fixture.config.process_home);
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    crate::config::persist_managed_cloud_relay_profile(path1_test_profile())
        .expect("persist Cloud profile for restore path");
    let mut config = fixture.config.clone();
    config.envelope_path = fixture.root.join("managed-bootstrap.json");
    let original = path1_test_envelope(&fixture.release_digest, "/srv/path1 managed workspaces");
    let original_bytes = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": original.schema_version,
        "cloudApiUrl": &original.cloud_api_url,
        "environmentId": &original.environment_id,
        "token": &original.token,
        "expiresAt": &original.expires_at,
        "runtimeReleaseDigest": &original.runtime_release_digest,
        "managedRepositoryRoot": &original.managed_repository_root,
        "providerRebuildActionId": &original.provider_rebuild_action_id,
        "expectedDataVolumeSerial": &original.expected_data_volume_serial,
        "expectedDataVolumeSizeGb": original.expected_data_volume_size_gb,
    }))
    .expect("serialize original envelope");
    fs::write(&config.envelope_path, original_bytes).expect("persist envelope fixture");
    let receipt = path1_test_receipt(
        &fixture.release_digest,
        2,
        BootstrapReceiptStatus::Confirmed,
        None,
    );
    receipt
        .persist(&config.receipt_path)
        .expect("persist confirmed receipt");
    persist_path1_grant_binding(&config, &original, &receipt);
    let mut substituted = original.clone();
    substituted.token = format!("mkboot_{}", "z".repeat(43));
    let identity = path1_test_identity();
    let release = verify_release(
        &config.manifest_path,
        &config.signature_path,
        &config.public_key_path,
        &fixture.release_digest,
        &config.kernel_binary,
    )
    .expect("fixture release signature verifies");

    assert!(
        super::resume_registration(&config, Some(&substituted), receipt, &identity, &release,)
            .is_err()
    );
    assert!(config.envelope_path.exists());
    let retained: ManagedBootstrapEnvelope =
        serde_json::from_slice(&fs::read(&config.envelope_path).expect("read retained envelope"))
            .expect("decode retained envelope");
    assert_eq!(
        retained.grant_binding_digest(),
        original.grant_binding_digest(),
        "the rejected substituted token must not replace or remove the original envelope"
    );

    fixture.cleanup();
}

#[test]
fn schema_two_bootstrap_rejects_a_cloud_repository_root_mismatch() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("schema-two-repository-root-mismatch");
    let previous_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    fs::write(
        &fixture.config.envelope_path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "cloudApiUrl": "https://cloud.example.test",
            "environmentId": "managed-env-1",
            "token": fixture.token,
            "expiresAt": (fixture.now + chrono::Duration::minutes(1)).to_rfc3339(),
            "runtimeReleaseDigest": fixture.release_digest,
            "managedRepositoryRoot": "/srv/expected",
        }))
        .unwrap(),
    )
    .unwrap();
    let mut response = fixture.exchange_response();
    response.managed_repository_root = Some("/srv/different".to_string());
    let cloud = FakeCloud::new(response);

    assert!(prepare_managed_kernel(&fixture.config, &cloud, fixture.now).is_err());

    match previous_home {
        Some(value) => std::env::set_var("CHARIOX_HOME", value),
        None => std::env::remove_var("CHARIOX_HOME"),
    }
    fixture.cleanup();
}

#[test]
fn exchanged_registration_can_confirm_after_the_one_time_token_expires() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("late-confirm");
    let previous_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let cloud = FakeCloud::new(fixture.exchange_response());
    *cloud.fail_next_confirm.lock().expect("fail confirm") = true;

    let prepared = prepare_managed_kernel(&fixture.config, &cloud, fixture.now)
        .expect("exchange should complete before confirmation");
    let pending = prepared
        .confirmation
        .expect("confirmation should remain pending");
    assert!(pending
        .confirm(&fixture.config, &cloud, fixture.now)
        .is_err());
    assert!(fixture.config.envelope_path.exists());
    assert_eq!(
        BootstrapReceipt::read(&fixture.config.receipt_path)
            .expect("read exchanged receipt")
            .expect("exchanged receipt")
            .status,
        BootstrapReceiptStatus::Exchanged
    );

    let after_expiry = fixture.now + chrono::Duration::minutes(10);
    pending
        .confirm(&fixture.config, &cloud, after_expiry)
        .expect("bound exchange should confirm after expiry");
    assert!(!fixture.config.envelope_path.exists());
    assert_eq!(
        cloud.exchange_calls.lock().expect("exchange calls").len(),
        1
    );
    assert_eq!(cloud.confirm_calls.lock().expect("confirm calls").len(), 2);

    restore_env("CHARIOX_HOME", previous_home);
    fixture.cleanup();
}

#[test]
fn bootstrap_rejects_a_tampered_kernel_before_contacting_cloud() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("tampered");
    let previous_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    fs::write(&fixture.config.kernel_binary, b"tampered kernel").expect("tamper kernel");
    let cloud = FakeCloud::new(fixture.exchange_response());

    let error = prepare_managed_kernel(&fixture.config, &cloud, fixture.now)
        .expect_err("tampered release must fail");
    assert!(error.to_string().contains("kernel artifact digest"));
    assert!(cloud
        .exchange_calls
        .lock()
        .expect("exchange calls")
        .is_empty());
    assert!(cloud
        .confirm_calls
        .lock()
        .expect("confirm calls")
        .is_empty());

    restore_env("CHARIOX_HOME", previous_home);
    fixture.cleanup();
}

#[test]
fn release_verifier_accepts_v2_identity_and_legacy_v1() {
    let fixture = Fixture::new("release-schema-compatibility");

    verify_release(
        &fixture.config.manifest_path,
        &fixture.config.signature_path,
        &fixture.config.public_key_path,
        &fixture.release_digest,
        &fixture.config.kernel_binary,
    )
    .expect("schema v2 release should verify");

    let legacy_digest = fixture.write_signed_manifest(serde_json::json!({
        "schemaVersion": 1,
        "artifacts": [fixture.kernel_artifact()],
    }));
    verify_release(
        &fixture.config.manifest_path,
        &fixture.config.signature_path,
        &fixture.config.public_key_path,
        &legacy_digest,
        &fixture.config.kernel_binary,
    )
    .expect("legacy schema v1 release should remain restart-compatible");

    fixture.cleanup();
}

#[test]
fn release_verifier_requires_signed_v3_update_evidence_capability() {
    let fixture = Fixture::new("release-v3-update-evidence");
    let manifest = serde_json::json!({
        "schemaVersion": 3,
        "managedUpdateEvidenceVersion": 1,
        "sourceCommit": "a".repeat(40),
        "sourceTree": "b".repeat(40),
        "artifacts": [fixture.kernel_artifact()],
    });
    let digest = fixture.write_signed_manifest(manifest.clone());
    verify_release(
        &fixture.config.manifest_path,
        &fixture.config.signature_path,
        &fixture.config.public_key_path,
        &digest,
        &fixture.config.kernel_binary,
    )
    .expect("schema v3 release must declare supported update evidence");

    for (schema, capability) in [(3, None), (3, Some(2)), (2, Some(1)), (1, Some(1))] {
        let mut invalid = manifest.clone();
        invalid["schemaVersion"] = serde_json::json!(schema);
        if let Some(version) = capability {
            invalid["managedUpdateEvidenceVersion"] = serde_json::json!(version);
        } else {
            invalid
                .as_object_mut()
                .expect("manifest object")
                .remove("managedUpdateEvidenceVersion");
        }
        let digest = fixture.write_signed_manifest(invalid);
        let error = verify_release(
            &fixture.config.manifest_path,
            &fixture.config.signature_path,
            &fixture.config.public_key_path,
            &digest,
            &fixture.config.kernel_binary,
        )
        .expect_err("unsigned or unsupported update evidence capability must fail");
        assert!(error.to_string().contains("schema is unsupported"));
    }
    let mut null_capability = manifest;
    null_capability["managedUpdateEvidenceVersion"] = serde_json::Value::Null;
    let digest = fixture.write_signed_manifest(null_capability);
    assert!(verify_release(
        &fixture.config.manifest_path,
        &fixture.config.signature_path,
        &fixture.config.public_key_path,
        &digest,
        &fixture.config.kernel_binary,
    )
    .is_err());
    fixture.cleanup();
}

#[cfg(unix)]
#[test]
fn release_verifier_pins_installer_owned_release_symlinks() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("release-installer-symlinks");
    let install_root = fixture.root.join("installed");
    let chariox_root = install_root.join("usr/lib/chariox");
    let kernel_facade = install_root.join("usr/local/bin/chariox-kernel");
    let manifest_facade = chariox_root.join("release-manifest.json");
    let signature_facade = chariox_root.join("release-manifest.sig");
    let public_key_facade = chariox_root.join("release-public-key");
    let kernel_bytes = fs::read(&fixture.config.kernel_binary).expect("read kernel fixture");
    let kernel_sha256 = format!("sha256:{:x}", Sha256::digest(&kernel_bytes));
    let release_digest = fixture.write_signed_manifest(serde_json::json!({
        "schemaVersion": 2,
        "sourceCommit": "a".repeat(40),
        "sourceTree": "b".repeat(40),
        "artifacts": [{
            "name": "chariox-kernel",
            "path": kernel_facade,
            "sha256": kernel_sha256,
        }],
    }));
    let release_name = release_digest
        .strip_prefix("sha256:")
        .expect("release digest prefix");
    let versioned = chariox_root.join("releases").join(release_name);
    let versioned_chariox = versioned.join("usr/lib/chariox");
    let versioned_kernel = versioned.join("usr/local/bin/chariox-kernel");
    fs::create_dir_all(&versioned_chariox).expect("create versioned release metadata");
    fs::create_dir_all(versioned_kernel.parent().expect("kernel parent"))
        .expect("create versioned release binary directory");
    fs::create_dir_all(kernel_facade.parent().expect("kernel facade parent"))
        .expect("create kernel facade directory");

    for (source, destination) in [
        (
            &fixture.config.manifest_path,
            versioned_chariox.join("release-manifest.json"),
        ),
        (
            &fixture.config.signature_path,
            versioned_chariox.join("release-manifest.sig"),
        ),
        (
            &fixture.config.public_key_path,
            versioned_chariox.join("release-public-key"),
        ),
        (&fixture.config.kernel_binary, versioned_kernel.clone()),
    ] {
        fs::rename(source, destination).expect("move file into versioned release");
    }
    symlink(
        format!("releases/{release_name}"),
        chariox_root.join("current"),
    )
    .expect("link current release");
    symlink(
        "current/usr/lib/chariox/release-manifest.json",
        &manifest_facade,
    )
    .expect("link release manifest");
    symlink(
        "current/usr/lib/chariox/release-manifest.sig",
        &signature_facade,
    )
    .expect("link release signature");
    symlink(
        "current/usr/lib/chariox/release-public-key",
        &public_key_facade,
    )
    .expect("link release public key");
    symlink(
        "../../../usr/lib/chariox/current/usr/local/bin/chariox-kernel",
        &kernel_facade,
    )
    .expect("link kernel binary");

    let verified = verify_release(
        &manifest_facade,
        &signature_facade,
        &public_key_facade,
        &release_digest,
        &kernel_facade,
    )
    .expect("installer-owned release symlinks should verify");
    assert_eq!(
        verified.kernel_binary,
        fs::canonicalize(&versioned_kernel).expect("canonical versioned kernel")
    );

    let external = fixture
        .root
        .join("external")
        .join("releases")
        .join(release_name);
    fs::create_dir_all(external.join("usr/lib/chariox"))
        .expect("create external metadata directory");
    fs::create_dir_all(external.join("usr/local/bin")).expect("create external binary directory");
    for relative in [
        "usr/lib/chariox/release-manifest.json",
        "usr/lib/chariox/release-manifest.sig",
        "usr/lib/chariox/release-public-key",
        "usr/local/bin/chariox-kernel",
    ] {
        fs::copy(versioned.join(relative), external.join(relative))
            .expect("copy external release file");
    }
    let next_current = chariox_root.join("current.external");
    symlink(&external, &next_current).expect("link external release");
    fs::rename(&next_current, chariox_root.join("current")).expect("pivot to external release");
    let external_error = verify_release(
        &manifest_facade,
        &signature_facade,
        &public_key_facade,
        &release_digest,
        &kernel_facade,
    )
    .expect_err("release paths outside the pinned versioned root must fail");
    assert!(external_error
        .to_string()
        .contains("outside the pinned release"));

    let second = chariox_root.join("releases").join("0".repeat(64));
    fs::create_dir_all(second.join("usr/local/bin")).expect("create second release");
    fs::write(
        second.join("usr/local/bin/chariox-kernel"),
        b"different kernel\n",
    )
    .expect("write second release kernel");
    let next_current = chariox_root.join("current.second");
    symlink(&second, &next_current).expect("link second release");
    fs::rename(&next_current, chariox_root.join("current")).expect("pivot current release");
    assert_eq!(
        fs::read(&verified.kernel_binary).expect("read pinned kernel"),
        kernel_bytes
    );
    assert_eq!(
        fs::read(&kernel_facade).expect("read pivoted kernel facade"),
        b"different kernel\n"
    );

    fixture.cleanup();
}

#[test]
fn release_verifier_rejects_missing_or_invalid_v2_source_identity() {
    let fixture = Fixture::new("release-v2-identity");
    let invalid_manifests = [
        serde_json::json!({
            "schemaVersion": 2,
            "sourceTree": "b".repeat(40),
            "artifacts": [fixture.kernel_artifact()],
        }),
        serde_json::json!({
            "schemaVersion": 2,
            "sourceCommit": "a".repeat(40),
            "artifacts": [fixture.kernel_artifact()],
        }),
        serde_json::json!({
            "schemaVersion": 2,
            "sourceCommit": "A".repeat(40),
            "sourceTree": "b".repeat(40),
            "artifacts": [fixture.kernel_artifact()],
        }),
        serde_json::json!({
            "schemaVersion": 2,
            "sourceCommit": "a".repeat(40),
            "sourceTree": "g".repeat(40),
            "artifacts": [fixture.kernel_artifact()],
        }),
    ];

    for manifest in invalid_manifests {
        let digest = fixture.write_signed_manifest(manifest);
        let error = verify_release(
            &fixture.config.manifest_path,
            &fixture.config.signature_path,
            &fixture.config.public_key_path,
            &digest,
            &fixture.config.kernel_binary,
        )
        .expect_err("invalid schema v2 source identity must fail");
        assert!(error.to_string().contains("source identity is invalid"));
    }

    fixture.cleanup();
}

#[test]
fn release_verifier_checks_digest_and_signature_before_manifest_identity() {
    let fixture = Fixture::new("release-signature-digest");

    let digest_error = verify_release(
        &fixture.config.manifest_path,
        &fixture.config.signature_path,
        &fixture.config.public_key_path,
        &format!("sha256:{}", "0".repeat(64)),
        &fixture.config.kernel_binary,
    )
    .expect_err("wrong managed release digest must fail");
    assert!(digest_error
        .to_string()
        .contains("digest does not match the managed environment"));

    fs::write(
        &fixture.config.signature_path,
        base64::engine::general_purpose::STANDARD.encode([0_u8; 64]),
    )
    .expect("replace release signature");
    let signature_error = verify_release(
        &fixture.config.manifest_path,
        &fixture.config.signature_path,
        &fixture.config.public_key_path,
        &fixture.release_digest,
        &fixture.config.kernel_binary,
    )
    .expect_err("invalid release signature must fail");
    assert!(signature_error
        .to_string()
        .contains("release manifest signature is invalid"));

    fixture.cleanup();
}

#[test]
fn bootstrap_rejects_a_context_source_in_another_relay_realm() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = Fixture::new("context-realm-mismatch");
    let previous_home = std::env::var_os("CHARIOX_HOME");
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    let mut response = fixture.exchange_response();
    response.context_plan = ManagedKernelContextPlan::source_project_for_tests(
        "managed_ctx_bootstrap",
        "realm-2",
        "source-kernel",
        &"a".repeat(64),
        "project-chariox",
    );
    let cloud = FakeCloud::new(response);

    let error = prepare_managed_kernel(&fixture.config, &cloud, fixture.now)
        .expect_err("a target outside the source realm must fail bootstrap");
    assert!(error
        .to_string()
        .contains("does not match the local identity"));
    assert!(!fixture.config.receipt_path.exists());

    restore_env("CHARIOX_HOME", previous_home);
    fixture.cleanup();
}

#[test]
fn legacy_confirmed_receipt_remains_readable_without_context_authorization() {
    let fixture = Fixture::new("legacy-receipt");
    fs::create_dir_all(
        fixture
            .config
            .receipt_path
            .parent()
            .expect("receipt parent"),
    )
    .expect("create receipt parent");
    fs::write(
        &fixture.config.receipt_path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "status": "confirmed",
            "environmentId": "managed-env-legacy",
            "machineId": "managed-machine-legacy",
            "kernelId": "managed-kernel-legacy",
            "relayPublicKey": "legacy-relay-public-key",
            "runtimeReleaseDigest": fixture.release_digest,
            "confirmedAt": fixture.now.to_rfc3339(),
        }))
        .expect("encode legacy receipt"),
    )
    .expect("write legacy receipt");

    let receipt = BootstrapReceipt::read(&fixture.config.receipt_path)
        .expect("read legacy receipt")
        .expect("legacy receipt exists");
    assert_eq!(receipt.status, BootstrapReceiptStatus::Confirmed);
    assert!(receipt.context_plan.is_none());

    fixture.cleanup();
}

#[test]
fn managed_systemd_unit_keeps_bootstrap_and_kernel_in_one_hardened_cgroup() {
    let unit = include_str!("../../../../deploy/managed-kernel/chariox-managed-bootstrap.service");
    for required in [
        "User=chariox",
        "Group=chariox",
        "SupplementaryGroups=chariox-slice",
        "Environment=CHARIOX_HOME=/home/chariox/.chariox",
        "Environment=HOME=/home/chariox",
        "Environment=CHARIOX_CAPABILITY_ISOLATION_ROOT=/home/chariox/.chariox/managed-context/kernel",
        "Environment=CHARIOX_MANAGED_PROVIDER_ISOLATION=1",
        "Environment=CHARIOX_MANAGED_SLICE_SERVICE_ROOT=/var/lib/chariox-slice-share",
        "Environment=CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT=/var/lib/chariox-slice-share/slices",
        "Environment=CHARIOX_SLICE_ROOT=/var/lib/chariox-slice-share/slices",
        "Environment=CHARIOX_MANAGED_VAULT_PATH=/home/chariox/.chariox/vault/vault.json",
        "Environment=CHARIOX_SLICE_DOCKER_BROKER_SOCKET=/var/lib/chariox-slice-share/.broker-private/control/control.sock",
        "Environment=PATH=/usr/local/bin:/usr/bin:/bin",
        "After=chariox-rootless-docker.service",
        "Wants=network-online.target chariox-rootless-docker.service",
        "ExecStartPre=-+/usr/bin/systemctl restart chariox-slice-broker.service",
        "ExecStart=/usr/local/bin/chariox-managed-bootstrap",
        "KillMode=control-group",
        "StartLimitIntervalSec=0",
        "After=network-online.target",
        "RestartSteps=8",
        "RestartMaxDelaySec=5min",
        "NoNewPrivileges=true",
        "RestrictSUIDSGID=true",
        "ProtectSystem=strict",
        "ProtectKernelTunables=false",
        "StateDirectory=chariox",
        "StateDirectoryMode=0700",
        "ReadWritePaths=/home/chariox /var/lib/chariox /var/lib/chariox-slice-share",
        "UMask=0007",
    ] {
        assert!(
            unit.contains(required),
            "missing systemd contract: {required}"
        );
    }
    assert_eq!(
        unit.lines()
            .find(|line| line.starts_with("RestrictAddressFamilies=")),
        Some("RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK")
    );
    assert!(!unit.contains("cloud-final.service"));
    assert!(!unit.contains("ssh"));
    assert!(!unit.contains("Requires=chariox-rootless-docker.service"));
    assert!(!unit.contains("Environment=DOCKER_HOST="));
}

#[test]
fn disposable_worker_systemd_unit_runs_path1_without_provider_isolation() {
    let unit = include_str!(
        "../../../../deploy/managed-kernel/chariox-disposable-worker-bootstrap.service"
    );
    for required in [
        "User=chariox",
        "Group=chariox",
        "Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1",
        "Environment=HOME=/home/chariox",
        "Environment=CHARIOX_HOME=/home/chariox/.chariox",
        "Environment=CHARIOX_MANAGED_VAULT_PATH=/home/chariox/.chariox/vault/vault.json",
        "Environment=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "After=network-online.target chariox-rootless-docker.service",
        "Requires=chariox-rootless-docker.service",
        "ExecStart=/usr/local/bin/chariox-managed-bootstrap --disposable-worker",
        // Managed-machine automatic shutdown triggers stay on the Path-1 unit.
        "Conflicts=chariox-managed-bootstrap.service",
        "KillMode=control-group",
    ] {
        assert!(
            unit.contains(required),
            "missing disposable worker contract: {required}"
        );
    }
    // Path 1 providers use the ordinary HOME; the worker has no separate provider home.
    assert!(!unit.contains("CHARIOX_MANAGED_PROVIDER_HOME"));
    assert!(!unit.contains("CHARIOX_DISPOSABLE_WORKER_BOOTSTRAP_PATH="));
    // Path 1 executes providers directly in the disposable VM. The VM is the
    // provider boundary, so this unit must not carry Bubblewrap/provider
    // isolation or the managed shared-host systemd hardening restrictions.
    for forbidden in [
        "CHARIOX_MANAGED_PROVIDER_ISOLATION",
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        "CHARIOX_MANAGED_PROVIDER_BWRAP",
        "bwrap",
        "NoNewPrivileges=",
        "ProtectSystem=",
        "ProtectHome=",
        "RestrictSUIDSGID=",
        "RestrictAddressFamilies=",
        "SupplementaryGroups=chariox-slice",
        "/var/lib/chariox/home",
    ] {
        assert!(
            !unit.contains(forbidden),
            "Path-1 disposable worker must not carry: {forbidden}"
        );
    }
}

#[test]
fn managed_rootless_docker_unit_never_exposes_the_rootful_socket() {
    let unit = include_str!("../../../../deploy/managed-kernel/chariox-rootless-docker.service");
    for required in [
        "User=chariox-docker",
        "Group=chariox-docker",
        "Environment=HOME=/var/lib/chariox-docker/home",
        "Environment=XDG_RUNTIME_DIR=/run/chariox-docker",
        "Environment=DOCKER_HOST=unix:///run/chariox-docker/docker.sock",
        "ExecStartPre=+/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-rootless-service.sh prepare",
        "ExecStart=/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-rootless-service.sh start",
        "ExecStartPost=/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-rootless-service.sh ready",
        "ExecStopPost=/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-rootless-service.sh stop",
        "RuntimeDirectory=chariox-docker",
        "RuntimeDirectoryMode=0700",
        "StateDirectory=chariox-docker",
        "StateDirectoryMode=0700",
        "ProtectSystem=strict",
        "ProtectKernelTunables=false",
        "RestrictSUIDSGID=true",
        "NoNewPrivileges=true",
        "ReadWritePaths=/var/lib/chariox-docker /var/lib/chariox-slice-share/.broker-private /var/lib/chariox-slice-share/slices/development /run/chariox-docker",
    ] {
        assert!(
            unit.contains(required),
            "missing rootless Docker contract: {required}"
        );
    }
    assert!(!unit.contains("/var/run/docker.sock"));
    assert!(!unit.contains("User=root"));
    assert!(!unit.contains("SupplementaryGroups=chariox-slice"));
    assert!(!unit.contains("/var/lib/chariox/home"));
    let engine = include_str!("../../slice-linux-docker/chariox-rootless-engine.service");
    assert!(engine.contains("ExecStart=/usr/share/docker.io/contrib/dockerd-rootless.sh --host=unix:///run/chariox-docker/docker.sock --data-root=/var/lib/chariox-docker/data --exec-opt native.cgroupdriver=systemd"));
    assert!(engine.contains("Delegate=cpu cpuset io memory pids"));
    assert!(engine.contains("Restart=no"));
    assert!(!engine.contains("/var/run/docker.sock"));
    assert!(!engine.lines().any(|line| line.starts_with("User=")));
}

#[test]
fn managed_slice_broker_owns_the_only_docker_socket_and_unlinks_its_endpoint() {
    let unit = include_str!("../../../../deploy/managed-kernel/chariox-slice-broker.service");
    let broker = include_str!("../../slice-linux-docker/managed-docker-broker.mjs");
    for required in [
        "User=chariox-docker",
        "Group=chariox-docker",
        "Environment=DOCKER_HOST=unix:///run/chariox-docker/docker.sock",
        "Environment=CHARIOX_SLICE_DOCKER_BROKER_SOCKET=/var/lib/chariox-slice-share/.broker-private/control/control.sock",
        "Environment=CHARIOX_MANAGED_RELEASE_MANIFEST=/usr/lib/chariox/release-manifest.json",
        "ExecStart=/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/enter-rootless-docker-namespace.sh /usr/bin/node",
        "Restart=no",
        "NoNewPrivileges=true",
        "CapabilityBoundingSet=",
        "ProtectSystem=strict",
    ] {
        assert!(
            unit.contains(required),
            "missing slice broker contract: {required}"
        );
    }
    let writable_paths = systemd_service_list_property(unit, "ReadWritePaths");
    let required_writable_paths = [
        "/run/chariox-docker",
        "/var/lib/chariox-docker",
        "/var/lib/chariox-slice-share",
    ];
    for required in required_writable_paths {
        assert!(
            writable_paths.contains(&required),
            "broker is missing writable path {required}: {writable_paths:?}"
        );
    }
    let additional_writable_paths = writable_paths
        .iter()
        .copied()
        .filter(|path| !required_writable_paths.contains(path))
        .collect::<Vec<_>>();
    assert!(
        additional_writable_paths.is_empty()
            || additional_writable_paths == vec!["/var/lib/chariox-slice-disk-quota"],
        "broker has an unexpected writable path: {additional_writable_paths:?}"
    );
    assert!(broker.contains("server.close()"));
    assert!(broker.contains("rmSync(SOCKET_PATH, { force: true })"));
    assert!(broker.contains("Docker command shape is not allowed"));
    assert!(broker.contains("must stay under the managed slice share"));
    assert!(broker.contains("chariox-slice-build-context"));
    assert!(!unit.contains("/var/lib/chariox/home"));
}

fn systemd_service_list_property<'a>(unit: &'a str, property: &str) -> Vec<&'a str> {
    let mut in_service_section = false;
    let mut values = Vec::new();
    let prefix = format!("{property}=");
    for line in unit.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_service_section = line == "[Service]";
            continue;
        }
        if !in_service_section {
            continue;
        }
        let Some(value) = line.strip_prefix(&prefix) else {
            continue;
        };
        if value.trim().is_empty() {
            values.clear();
        } else {
            values.extend(value.split_whitespace());
        }
    }
    values.sort_unstable();
    values.dedup();
    values
}

#[test]
fn managed_systemd_unit_remains_eligible_after_one_time_envelope_removal() {
    crate::test_support::isolated_env_test!();
    let unit = include_str!("../../../../deploy/managed-kernel/chariox-managed-bootstrap.service");
    assert!(unit.contains("ConditionPathExists=/usr/local/bin/chariox-managed-bootstrap"));
    assert!(!unit.contains("ConditionPathExists=/var/lib/chariox/managed-bootstrap.json"));
}

pub(super) struct Fixture {
    root: PathBuf,
    pub(super) config: BootstrapConfig,
    now: chrono::DateTime<Utc>,
    pub(super) release_digest: String,
    token: String,
    kernel_started_marker: PathBuf,
}

impl Fixture {
    pub(super) fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-bootstrap-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let process_home = root.join("home");
        let chariox_home = process_home.join(".chariox");
        fs::create_dir_all(&process_home).expect("create process HOME");
        let kernel_binary = root.join("bin").join("chariox-kernel");
        fs::create_dir_all(kernel_binary.parent().expect("kernel parent"))
            .expect("create kernel parent");
        let kernel_fixture = b"#!/bin/sh\nreceipt=\"${CHARIOX_DISPOSABLE_WORKER_RECEIPT:-$CHARIOX_HOME/managed/bootstrap-receipt.json}\"\nmarker=\"${CHARIOX_KERNEL_STARTED_MARKER:-$CHARIOX_HOME/managed/kernel-started}\"\nif grep -Eq '\"status\"[[:space:]]*:[[:space:]]*\"confirmed\"' \"$receipt\"; then\n  state=confirmed\nelif grep -Eq '\"status\"[[:space:]]*:[[:space:]]*\"exchanged\"' \"$receipt\"; then\n  state=exchanged\nelse\n  state=invalid\nfi\nprintf '%s\\n' \"$state\" >> \"$marker\"\ntest -s \"$CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE\"\nrm -f -- \"$CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE\"\nsleep 1\n";
        fs::write(&kernel_binary, kernel_fixture).expect("write kernel fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&kernel_binary, fs::Permissions::from_mode(0o755))
                .expect("make kernel fixture executable");
        }
        let kernel_digest = format!("sha256:{:x}", Sha256::digest(kernel_fixture));
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "sourceCommit": "a".repeat(40),
            "sourceTree": "b".repeat(40),
            "artifacts": [{
                "name": "chariox-kernel",
                "path": kernel_binary.display().to_string(),
                "sha256": kernel_digest,
            }],
        }))
        .expect("encode release manifest");
        let release_digest = format!("sha256:{:x}", Sha256::digest(&manifest));
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let signature = signing_key.sign(&manifest);
        let manifest_path = root.join("release-manifest.json");
        let signature_path = root.join("release-manifest.sig");
        let public_key_path = root.join("release-public-key");
        fs::write(&manifest_path, manifest).expect("write manifest");
        fs::write(
            &signature_path,
            base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
        )
        .expect("write signature");
        fs::write(
            &public_key_path,
            base64::engine::general_purpose::STANDARD
                .encode(signing_key.verifying_key().to_bytes()),
        )
        .expect("write public key");
        let now = Utc.with_ymd_and_hms(2026, 8, 20, 14, 0, 0).unwrap();
        let token = format!("mkboot_{}", "a".repeat(43));
        let envelope_path = root.join("managed-bootstrap.json");
        fs::write(
            &envelope_path,
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "cloudApiUrl": "https://cloud.example.test",
                "environmentId": "managed-env-1",
                "token": token,
                "expiresAt": (now + chrono::Duration::minutes(1)).to_rfc3339(),
                "runtimeReleaseDigest": release_digest,
            }))
            .expect("encode envelope"),
        )
        .expect("write envelope");
        Self {
            config: BootstrapConfig {
                process_home,
                chariox_home: chariox_home.clone(),
                envelope_path,
                receipt_path: chariox_home.join("managed").join("bootstrap-receipt.json"),
                manifest_path,
                signature_path,
                public_key_path,
                kernel_binary,
                kernel_host: "127.0.0.1".to_string(),
                kernel_port: 43118,
            },
            root,
            now,
            release_digest,
            token,
            kernel_started_marker: chariox_home.join("managed").join("kernel-started"),
        }
    }

    fn exchange_response(&self) -> ExchangeResponse {
        ExchangeResponse {
            environment_id: String::new(),
            kernel_id: String::new(),
            generation: Some(1),
            runtime_release_digest: String::new(),
            managed_repository_root: None,
            context_plan: ManagedKernelContextPlan::empty_for_tests("managed_ctx_bootstrap"),
            cloud_relay: ManagedCloudRelayProfile {
                api_url: "https://cloud.example.test".to_string(),
                email: "owner@example.test".to_string(),
                account_id: "account-1".to_string(),
                user_id: "owner-1".to_string(),
                account_slug: "account-one".to_string(),
                realm_id: "realm-1".to_string(),
                relay_url: "wss://relay.example.test".to_string(),
                issuer_id: "issuer-1".to_string(),
                machine_id: String::new(),
                machine_alias: "Managed agents".to_string(),
                machine_credential: format!("mcred_{}", "b".repeat(43)),
            },
        }
    }

    fn kernel_artifact(&self) -> serde_json::Value {
        serde_json::json!({
            "name": "chariox-kernel",
            "path": self.config.kernel_binary.display().to_string(),
            "sha256": format!(
                "sha256:{:x}",
                Sha256::digest(fs::read(&self.config.kernel_binary).expect("read kernel fixture"))
            ),
        })
    }

    fn write_signed_manifest(&self, manifest: serde_json::Value) -> String {
        let manifest = serde_json::to_vec(&manifest).expect("encode release manifest");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let signature = signing_key.sign(&manifest);
        fs::write(&self.config.manifest_path, &manifest).expect("write release manifest");
        fs::write(
            &self.config.signature_path,
            base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
        )
        .expect("write release signature");
        format!("sha256:{:x}", Sha256::digest(&manifest))
    }

    pub(super) fn cleanup(self) {
        let _ = fs::remove_dir_all(self.root);
    }
}

fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => std::env::set_var(name, value),
        None => std::env::remove_var(name),
    }
}

/// Restores the captured environment variables when dropped. A failed assertion
/// unwinds through the drop too, so a failing test cannot leave a managed HOME,
/// CHARIOX_HOME or bootstrap receipt behind for every later test in the process.
#[cfg(unix)]
struct EnvironmentRestoreGuard {
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

#[cfg(unix)]
impl EnvironmentRestoreGuard {
    fn capture(names: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            previous: names
                .into_iter()
                .map(|name| (name, std::env::var_os(name)))
                .collect(),
        }
    }
}

#[cfg(unix)]
impl Drop for EnvironmentRestoreGuard {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..) {
            restore_env(name, value);
        }
    }
}

#[cfg(unix)]
struct AttestedReleaseFixture {
    root: PathBuf,
    config: BootstrapConfig,
    release_digest: String,
    release_root: PathBuf,
    attestation_path: PathBuf,
    attestation_signature_path: PathBuf,
    receipt_path: PathBuf,
}

#[cfg(unix)]
impl AttestedReleaseFixture {
    fn new(
        label: &str,
        attestation_source_commit: &str,
        attestation_source_tree: &str,
        target: &str,
    ) -> Self {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "chariox-release-evidence-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let install_root = root.join("installed");
        let chariox_root = install_root.join("usr/lib/chariox");
        let kernel_facade = install_root.join("usr/local/bin/chariox-kernel");
        let kernel_bytes = b"#!/bin/sh\nverified-kernel\n";
        let kernel_digest = format!("sha256:{:x}", Sha256::digest(kernel_bytes));
        let supervisor_bytes = b"#!/bin/sh\nverified-bootstrap\n";
        let supervisor_digest = format!("sha256:{:x}", Sha256::digest(supervisor_bytes));
        let source_commit = "a".repeat(40);
        let source_tree = "b".repeat(40);
        let attestation = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "sourceCommit": attestation_source_commit,
            "sourceTree": attestation_source_tree,
            "target": target,
            "artifacts": [
                { "name": "chariox-kernel", "sha256": kernel_digest },
                { "name": "chariox-managed-bootstrap", "sha256": supervisor_digest },
                { "name": "chariox-relay", "sha256": format!("sha256:{}", "d".repeat(64)) },
            ],
        }))
        .expect("encode build attestation");
        let builder_key = SigningKey::from_bytes(&[8_u8; 32]);
        let attestation_signature = builder_key.sign(&attestation);
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "sourceCommit": source_commit,
            "sourceTree": source_tree,
            "artifacts": [
                {
                    "name": "chariox-kernel",
                    "path": kernel_facade.display().to_string(),
                    "sha256": kernel_digest,
                },
                {
                    "name": "chariox-managed-bootstrap",
                    "path": "/usr/local/bin/chariox-managed-bootstrap",
                    "sha256": supervisor_digest,
                },
                {
                    "name": "chariox-build-attestation",
                    "path": "/usr/lib/chariox/build-attestation.json",
                    "sha256": format!("sha256:{:x}", Sha256::digest(&attestation)),
                },
                {
                    "name": "chariox-build-attestation-signature",
                    "path": "/usr/lib/chariox/build-attestation.sig",
                    "sha256": format!(
                        "sha256:{:x}",
                        Sha256::digest(
                            base64::engine::general_purpose::STANDARD
                                .encode(attestation_signature.to_bytes())
                                .as_bytes()
                        )
                    ),
                },
                {
                    "name": "chariox-builder-public-key",
                    "path": "/usr/lib/chariox/builder-public-key",
                    "sha256": format!(
                        "sha256:{:x}",
                        Sha256::digest(
                            base64::engine::general_purpose::STANDARD
                                .encode(builder_key.verifying_key().to_bytes())
                                .as_bytes()
                        )
                    ),
                },
            ],
        }))
        .expect("encode release manifest");
        let release_digest = format!("sha256:{:x}", Sha256::digest(&manifest));
        let release_name = release_digest
            .strip_prefix("sha256:")
            .expect("release digest prefix");
        let release_root = chariox_root.join("releases").join(release_name);
        let release_chariox_root = release_root.join("usr/lib/chariox");
        let release_kernel = release_root.join("usr/local/bin/chariox-kernel");
        let release_supervisor = release_root.join("usr/local/bin/chariox-managed-bootstrap");
        fs::create_dir_all(&release_chariox_root).expect("create release metadata directory");
        fs::create_dir_all(release_kernel.parent().expect("release kernel parent"))
            .expect("create release kernel directory");
        fs::create_dir_all(kernel_facade.parent().expect("kernel facade parent"))
            .expect("create kernel facade directory");
        fs::write(&release_kernel, kernel_bytes).expect("write kernel artifact");
        fs::write(&release_supervisor, supervisor_bytes).expect("write supervisor artifact");
        fs::write(
            release_chariox_root.join("build-attestation.json"),
            &attestation,
        )
        .expect("write build attestation");
        fs::write(
            release_chariox_root.join("build-attestation.sig"),
            base64::engine::general_purpose::STANDARD.encode(attestation_signature.to_bytes()),
        )
        .expect("write build attestation signature");
        fs::write(
            release_chariox_root.join("builder-public-key"),
            base64::engine::general_purpose::STANDARD
                .encode(builder_key.verifying_key().to_bytes()),
        )
        .expect("write builder public key");
        let release_signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let release_signature = release_signing_key.sign(&manifest);
        fs::write(
            release_chariox_root.join("release-manifest.json"),
            &manifest,
        )
        .expect("write release manifest");
        fs::write(
            release_chariox_root.join("release-manifest.sig"),
            base64::engine::general_purpose::STANDARD.encode(release_signature.to_bytes()),
        )
        .expect("write release signature");
        fs::write(
            release_chariox_root.join("release-public-key"),
            base64::engine::general_purpose::STANDARD
                .encode(release_signing_key.verifying_key().to_bytes()),
        )
        .expect("write release public key");
        symlink(
            format!("releases/{release_name}"),
            chariox_root.join("current"),
        )
        .expect("link current release");
        for (target, facade) in [
            (
                "current/usr/lib/chariox/release-manifest.json",
                chariox_root.join("release-manifest.json"),
            ),
            (
                "current/usr/lib/chariox/release-manifest.sig",
                chariox_root.join("release-manifest.sig"),
            ),
            (
                "current/usr/lib/chariox/release-public-key",
                chariox_root.join("release-public-key"),
            ),
        ] {
            symlink(target, facade).expect("link release facade");
        }
        symlink(
            "../../../usr/lib/chariox/current/usr/local/bin/chariox-kernel",
            &kernel_facade,
        )
        .expect("link kernel facade");

        let process_home = root.join("home");
        let chariox_home = process_home.join(".chariox");
        let receipt_path = chariox_home.join("managed/bootstrap-receipt.json");
        fs::create_dir_all(receipt_path.parent().expect("receipt parent"))
            .expect("create receipt parent");
        fs::write(
            &receipt_path,
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "status": "confirmed",
                "environmentId": "managed-env-1",
                "machineId": "managed-machine-1",
                "kernelId": "managed-kernel-1",
                "relayPublicKey": "relay-public-key",
                "runtimeReleaseDigest": release_digest,
                "confirmedAt": "2026-09-20T00:00:00Z",
            }))
            .expect("encode bootstrap receipt"),
        )
        .expect("write bootstrap receipt");

        Self {
            config: BootstrapConfig {
                process_home,
                chariox_home,
                envelope_path: root.join("managed-bootstrap.json"),
                receipt_path: receipt_path.clone(),
                manifest_path: chariox_root.join("release-manifest.json"),
                signature_path: chariox_root.join("release-manifest.sig"),
                public_key_path: chariox_root.join("release-public-key"),
                kernel_binary: kernel_facade,
                kernel_host: "127.0.0.1".to_string(),
                kernel_port: 43118,
            },
            root,
            release_digest,
            release_root,
            attestation_path: release_chariox_root.join("build-attestation.json"),
            attestation_signature_path: release_chariox_root.join("build-attestation.sig"),
            receipt_path,
        }
    }

    fn cleanup(self) {
        let _ = fs::remove_dir_all(self.root);
    }
}

fn path1_test_envelope(
    runtime_release_digest: &str,
    managed_repository_root: &str,
) -> ManagedBootstrapEnvelope {
    ManagedBootstrapEnvelope {
        schema_version: 3,
        cloud_api_url: "https://cloud.example.test".to_string(),
        environment_id: "managed-env-1".to_string(),
        token: format!("mkboot_{}", "x".repeat(43)),
        expires_at: "2026-09-27T00:00:00Z".to_string(),
        runtime_release_digest: runtime_release_digest.to_string(),
        managed_repository_root: Some(managed_repository_root.to_string()),
        provider_rebuild_action_id: None,
        expected_data_volume_serial: Some("12345".to_string()),
        expected_data_volume_size_gb: Some(20),
    }
}

fn persist_path1_grant_binding(
    config: &BootstrapConfig,
    envelope: &ManagedBootstrapEnvelope,
    receipt: &BootstrapReceipt,
) {
    ManagedBootstrapGrantBinding::for_receipt(envelope, receipt)
        .persist_for_receipt(&config.receipt_path)
        .expect("persist exact Path-1 grant binding");
}

fn path1_test_evidence(runtime_release_digest: &str) -> ManagedKernelFreshnessEvidence {
    ManagedKernelFreshnessEvidence {
        schema_version: Some(3),
        linux_boot_id: "01234567-89ab-cdef-0123-456789abcdef".to_string(),
        os_machine_id: "b".repeat(32),
        runtime_release_digest: runtime_release_digest.to_string(),
        runtime_source_commit: "a".repeat(40),
        runtime_source_tree: "b".repeat(40),
        residue_checks: ManagedKernelResidueChecks {
            old_services_absent: true,
            old_processes_absent: true,
            old_state_absent: true,
        },
        data_volume_serial: Some("12345".to_string()),
        data_volume_size_gb: Some(20),
    }
}

fn path1_test_receipt(
    runtime_release_digest: &str,
    generation: u64,
    status: BootstrapReceiptStatus,
    freshness_evidence: Option<ManagedKernelFreshnessEvidence>,
) -> BootstrapReceipt {
    BootstrapReceipt {
        schema_version: 3,
        status,
        environment_id: "managed-env-1".to_string(),
        machine_id: "managed-machine-1".to_string(),
        kernel_id: "managed-kernel-1".to_string(),
        generation,
        relay_public_key: "relay-public-key".to_string(),
        runtime_release_digest: runtime_release_digest.to_string(),
        managed_repository_root: Some("/srv/path1 managed workspaces".to_string()),
        confirmed_at: (status == BootstrapReceiptStatus::Confirmed)
            .then_some("2026-09-27T00:00:00Z".to_string()),
        context_plan: None,
        provider_rebuild_action_id: None,
        freshness_evidence,
    }
}

fn path1_test_identity() -> ManagedRuntimeIdentity {
    ManagedRuntimeIdentity {
        machine_id: "managed-machine-1".to_string(),
        kernel_id: "managed-kernel-1".to_string(),
        relay_public_key: "relay-public-key".to_string(),
    }
}

fn path1_test_profile() -> PersistedCloudRelayProfile {
    PersistedCloudRelayProfile {
        kernel_id: None,
        kernel_credential: None,
        kernel_public_key_thumbprint: None,
        api_url: "https://cloud.example.test".to_string(),
        user_id: "owner-1".to_string(),
        machine_id: Some("managed-machine-1".to_string()),
        machine_credential: Some(format!("mcred_{}", "b".repeat(43))),
        relay_url: "wss://relay.example.test".to_string(),
        ..PersistedCloudRelayProfile::default()
    }
}

fn path1_test_cloud_response(
    runtime_release_digest: &str,
    managed_repository_root: &str,
) -> ExchangeResponse {
    ExchangeResponse {
        environment_id: "managed-env-1".to_string(),
        kernel_id: "managed-kernel-1".to_string(),
        generation: Some(2),
        runtime_release_digest: runtime_release_digest.to_string(),
        managed_repository_root: Some(managed_repository_root.to_string()),
        context_plan: ManagedKernelContextPlan::empty_for_tests("managed_ctx_bootstrap"),
        cloud_relay: ManagedCloudRelayProfile {
            api_url: "https://cloud.example.test".to_string(),
            email: "owner@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "owner-1".to_string(),
            account_slug: "account-one".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "wss://relay.example.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            machine_id: "managed-machine-1".to_string(),
            machine_alias: "Managed agents".to_string(),
            machine_credential: format!("mcred_{}", "b".repeat(43)),
        },
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn configure_path1_builder_trust(fixture: &AttestedReleaseFixture) {
    set_release_evidence_env(fixture);
    std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "path1");
    let packaged_key = fixture
        .release_root
        .join("usr/lib/chariox/builder-public-key");
    let external_key = fixture.root.join("trusted-builder-public-key");
    fs::copy(&packaged_key, &external_key).expect("copy fixture builder key outside release");
    std::env::set_var(super::state::TRUSTED_BUILDER_PUBLIC_KEY_ENV, external_key);
}

/// Point the release-evidence lookup at `fixture`. Release evidence requires an
/// explicit provider topology; shared_host verifies against the release's own
/// signing key, so tests that need Path 1 builder trust override it afterwards.
#[cfg(unix)]
fn set_release_evidence_env(fixture: &AttestedReleaseFixture) {
    std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "shared_host");
    std::env::set_var("HOME", &fixture.config.process_home);
    std::env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    std::env::set_var(
        "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
        &fixture.config.receipt_path,
    );
    std::env::set_var(
        "CHARIOX_MANAGED_RELEASE_MANIFEST",
        &fixture.config.manifest_path,
    );
    std::env::set_var(
        "CHARIOX_MANAGED_RELEASE_SIGNATURE",
        &fixture.config.signature_path,
    );
    std::env::set_var(
        "CHARIOX_MANAGED_RELEASE_PUBLIC_KEY",
        &fixture.config.public_key_path,
    );
    std::env::set_var(
        "CHARIOX_MANAGED_KERNEL_BINARY",
        &fixture.config.kernel_binary,
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const RELEASE_EVIDENCE_ENV_NAMES: [&str; 8] = [
    "HOME",
    "CHARIOX_HOME",
    "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
    "CHARIOX_MANAGED_RELEASE_MANIFEST",
    "CHARIOX_MANAGED_RELEASE_SIGNATURE",
    "CHARIOX_MANAGED_RELEASE_PUBLIC_KEY",
    "CHARIOX_MANAGED_KERNEL_BINARY",
    MANAGED_PROVIDER_TOPOLOGY_ENV,
];

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn managed_release_evidence_is_kernel_verified_from_active_release_and_receipt() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let fixture = AttestedReleaseFixture::new(
        "valid",
        &"a".repeat(40),
        &"b".repeat(40),
        "x86_64-unknown-linux-gnu",
    );
    let _restore = EnvironmentRestoreGuard::capture(RELEASE_EVIDENCE_ENV_NAMES);
    set_release_evidence_env(&fixture);

    std::env::remove_var(MANAGED_PROVIDER_TOPOLOGY_ENV);
    let missing_topology = super::authoritative_managed_release_evidence_from_env()
        .expect_err("release evidence must reject an unspecified provider topology");
    assert!(missing_topology
        .to_string()
        .contains("must be explicitly set to path1 or shared_host"));
    std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "shared_host");

    let evidence = super::authoritative_managed_release_evidence_from_env()
        .expect("valid managed release evidence should verify")
        .expect("confirmed managed receipt should produce release evidence");
    assert_eq!(evidence.runtime_release_digest, fixture.release_digest);
    assert_eq!(evidence.source_commit, "a".repeat(40));
    assert_eq!(evidence.source_tree, "b".repeat(40));
    assert_eq!(evidence.target, "x86_64-unknown-linux-gnu");
    assert_eq!(
        evidence.active_release_path,
        fixture.release_root.display().to_string()
    );
    assert!(evidence.manifest_signature_verified);
    assert!(evidence.manifest_digest_verified);
    assert!(evidence.kernel_artifact_verified);
    assert!(evidence.bootstrap_receipt_verified);

    fixture.cleanup();
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn path1_release_evidence_requires_the_external_builder_key() {
    crate::test_support::isolated_env_test!();
    use std::os::unix::fs::symlink;

    let _env = crate::env_lock::lock();
    let fixture = AttestedReleaseFixture::new(
        "external-builder-key",
        &"a".repeat(40),
        &"b".repeat(40),
        "x86_64-unknown-linux-gnu",
    );
    let _restore = EnvironmentRestoreGuard::capture(
        RELEASE_EVIDENCE_ENV_NAMES
            .into_iter()
            .chain([super::state::TRUSTED_BUILDER_PUBLIC_KEY_ENV]),
    );
    set_release_evidence_env(&fixture);
    std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "path1");
    std::env::remove_var(super::state::TRUSTED_BUILDER_PUBLIC_KEY_ENV);
    assert!(super::authoritative_managed_release_evidence_from_env().is_err());

    let packaged_key = fixture
        .release_root
        .join("usr/lib/chariox/builder-public-key");
    let external_key = fixture.root.join("etc/chariox/trusted-builder-public-key");
    fs::create_dir_all(external_key.parent().expect("external key parent"))
        .expect("create external key directory");
    fs::copy(&packaged_key, &external_key).expect("persist external key");
    std::env::set_var(super::state::TRUSTED_BUILDER_PUBLIC_KEY_ENV, &external_key);
    super::authoritative_managed_release_evidence_from_env()
        .expect("matching external builder key")
        .expect("confirmed release evidence");

    let other_key = SigningKey::from_bytes(&[9_u8; 32]);
    fs::write(
        &external_key,
        base64::engine::general_purpose::STANDARD.encode(other_key.verifying_key().to_bytes()),
    )
    .expect("replace external key");
    let mismatch = super::authoritative_managed_release_evidence_from_env()
        .expect_err("mismatched builder trust must fail");
    assert!(mismatch.to_string().contains("does not match"));

    fs::remove_file(&external_key).expect("remove test key");
    symlink(&packaged_key, &external_key).expect("link external key to package");
    assert!(super::authoritative_managed_release_evidence_from_env().is_err());
    std::env::set_var(super::state::TRUSTED_BUILDER_PUBLIC_KEY_ENV, &packaged_key);
    let image_key = super::authoritative_managed_release_evidence_from_env()
        .expect_err("key inside release image must fail");
    assert!(image_key
        .to_string()
        .contains("outside the managed release image"));

    fixture.cleanup();
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn managed_release_evidence_fails_closed_for_attestation_target_source_signature_artifact_receipt_and_layout_mismatch(
) {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    for (label, source_commit, source_tree, target, expected) in [
        (
            "source-mismatch",
            "c",
            "b",
            "x86_64-unknown-linux-gnu",
            "source identity or target",
        ),
        (
            "tree-mismatch",
            "a",
            "c",
            "x86_64-unknown-linux-gnu",
            "source identity or target",
        ),
        (
            "target-mismatch",
            "a",
            "b",
            "aarch64-unknown-linux-gnu",
            "source identity or target",
        ),
    ] {
        let source = format!("{source_commit}{source_commit}").repeat(20);
        let tree = format!("{source_tree}{source_tree}").repeat(20);
        let fixture = AttestedReleaseFixture::new(label, &source, &tree, target);
        let _restore = EnvironmentRestoreGuard::capture(RELEASE_EVIDENCE_ENV_NAMES);
        set_release_evidence_env(&fixture);
        let error = super::authoritative_managed_release_evidence_from_env()
            .expect_err("attestation identity mismatch must fail closed");
        assert!(error.to_string().contains(expected));
        fixture.cleanup();
    }

    let receipt_fixture = AttestedReleaseFixture::new(
        "receipt-mismatch",
        &"a".repeat(40),
        &"b".repeat(40),
        "x86_64-unknown-linux-gnu",
    );
    let _restore = EnvironmentRestoreGuard::capture(RELEASE_EVIDENCE_ENV_NAMES);
    set_release_evidence_env(&receipt_fixture);
    fs::write(
        &receipt_fixture.receipt_path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "status": "confirmed",
            "environmentId": "managed-env-1",
            "machineId": "managed-machine-1",
            "kernelId": "managed-kernel-1",
            "relayPublicKey": "relay-public-key",
            "runtimeReleaseDigest": format!("sha256:{}", "0".repeat(64)),
            "confirmedAt": "2026-09-20T00:00:00Z",
        }))
        .expect("encode stale receipt"),
    )
    .expect("write stale receipt");
    let receipt_error = super::authoritative_managed_release_evidence_from_env()
        .expect_err("receipt digest mismatch must fail closed");
    assert!(
        receipt_error.to_string().contains("release path")
            || receipt_error.to_string().contains("digest")
    );
    receipt_fixture.cleanup();

    let fixture = AttestedReleaseFixture::new(
        "tamper",
        &"a".repeat(40),
        &"b".repeat(40),
        "x86_64-unknown-linux-gnu",
    );
    let _restore = EnvironmentRestoreGuard::capture(RELEASE_EVIDENCE_ENV_NAMES);
    set_release_evidence_env(&fixture);
    fs::write(&fixture.attestation_signature_path, "invalid")
        .expect("tamper attestation signature");
    let signature_error = super::authoritative_managed_release_evidence_from_env()
        .expect_err("attestation signature mismatch must fail closed");
    assert!(signature_error.to_string().contains("artifact digest"));
    fs::write(&fixture.attestation_path, "tampered").expect("tamper attestation");
    let artifact_error = super::authoritative_managed_release_evidence_from_env()
        .expect_err("pinned attestation mismatch must fail closed");
    assert!(artifact_error.to_string().contains("artifact digest"));
    fixture.cleanup();

    let legacy = Fixture::new("unversioned-evidence");
    let layout_error = super::release::verify_release_evidence(
        &legacy.config.manifest_path,
        &legacy.config.signature_path,
        &legacy.config.public_key_path,
        &legacy.release_digest,
        &legacy.config.kernel_binary,
        None,
    )
    .expect_err("unversioned release layout must not produce authoritative evidence");
    assert!(layout_error.to_string().contains("active versioned layout"));
    legacy.cleanup();
}
