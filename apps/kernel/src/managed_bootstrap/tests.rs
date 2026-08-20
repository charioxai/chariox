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
    ManagedCloudRelayProfile,
};
use super::prepare_managed_kernel;
use super::state::{BootstrapConfig, BootstrapReceipt, BootstrapReceiptStatus};
use super::supervisor::run_kernel_once;
use crate::error::DaemonError;

struct FakeCloud {
    exchange_response: ExchangeResponse,
    exchange_calls: Mutex<Vec<ExchangeRequest>>,
    confirm_calls: Mutex<Vec<ConfirmRequest>>,
    fail_next_confirm: Mutex<bool>,
    confirm_after_child_marker: Mutex<Option<PathBuf>>,
}

impl FakeCloud {
    fn new(response: ExchangeResponse) -> Self {
        Self {
            exchange_response: response,
            exchange_calls: Mutex::new(Vec::new()),
            confirm_calls: Mutex::new(Vec::new()),
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
        })
    }
}

#[cfg(unix)]
#[test]
fn bootstrap_verifies_release_persists_identity_and_profile_then_resumes_without_token() {
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

    run_kernel_once(
        &fixture.config,
        &prepared.release,
        &mut prepared.confirmation,
        &cloud,
    )
    .expect("kernel child should start before relay-ready confirmation");
    assert!(prepared.confirmation.is_none());
    assert!(fixture.kernel_started_marker.exists());
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

    restore_env("CHARIOX_HOME", previous_home);
    fixture.cleanup();
}

#[test]
fn exchanged_registration_can_confirm_after_the_one_time_token_expires() {
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
fn managed_systemd_unit_keeps_bootstrap_and_kernel_in_one_hardened_cgroup() {
    let unit = include_str!("../../../../deploy/managed-kernel/chariox-managed-bootstrap.service");
    for required in [
        "User=chariox",
        "Group=chariox",
        "Environment=CHARIOX_HOME=/var/lib/chariox/home",
        "Environment=HOME=/var/lib/chariox/home",
        "Environment=CHARIOX_CAPABILITY_ISOLATION_ROOT=/var/lib/chariox/home/managed-context/kernel",
        "Environment=CHARIOX_MANAGED_VAULT_PATH=/var/lib/chariox/home/.chariox/vault/vault.json",
        "ExecStart=/usr/local/bin/chariox-managed-bootstrap",
        "KillMode=control-group",
        "StartLimitIntervalSec=0",
        "RestartSteps=8",
        "RestartMaxDelaySec=5min",
        "NoNewPrivileges=true",
        "ProtectSystem=strict",
        "StateDirectory=chariox",
        "StateDirectoryMode=0700",
        "ReadWritePaths=/var/lib/chariox",
        "UMask=0077",
    ] {
        assert!(
            unit.contains(required),
            "missing systemd contract: {required}"
        );
    }
    assert!(!unit.contains("ssh"));
}

struct Fixture {
    root: PathBuf,
    config: BootstrapConfig,
    now: chrono::DateTime<Utc>,
    release_digest: String,
    token: String,
    kernel_started_marker: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-bootstrap-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let home = root.join("home");
        let kernel_binary = root.join("bin").join("chariox-kernel");
        fs::create_dir_all(kernel_binary.parent().expect("kernel parent"))
            .expect("create kernel parent");
        let kernel_fixture =
            b"#!/bin/sh\ntouch \"$CHARIOX_HOME/managed/kernel-started\"\nsleep 1\n";
        fs::write(&kernel_binary, kernel_fixture).expect("write kernel fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&kernel_binary, fs::Permissions::from_mode(0o755))
                .expect("make kernel fixture executable");
        }
        let kernel_digest = format!("sha256:{:x}", Sha256::digest(kernel_fixture));
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
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
                chariox_home: home.clone(),
                envelope_path,
                receipt_path: home.join("managed").join("bootstrap-receipt.json"),
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
            kernel_started_marker: home.join("managed").join("kernel-started"),
        }
    }

    fn exchange_response(&self) -> ExchangeResponse {
        ExchangeResponse {
            environment_id: String::new(),
            kernel_id: String::new(),
            runtime_release_digest: String::new(),
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

    fn cleanup(self) {
        let _ = fs::remove_dir_all(self.root);
    }
}

fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => std::env::set_var(name, value),
        None => std::env::remove_var(name),
    }
}
