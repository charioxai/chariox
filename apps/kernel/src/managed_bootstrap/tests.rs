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
use super::release::verify_release;
use super::state::{BootstrapConfig, BootstrapReceipt, BootstrapReceiptStatus};
use super::supervisor::run_kernel_once;
use super::ManagedKernelContextPlan;
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

    run_kernel_once(
        &fixture.config,
        &prepared.release,
        &mut prepared.confirmation,
        &cloud,
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
        "Environment=CHARIOX_HOME=/var/lib/chariox/home",
        "Environment=HOME=/var/lib/chariox/home",
        "Environment=CHARIOX_CAPABILITY_ISOLATION_ROOT=/var/lib/chariox/home/managed-context/kernel",
        "Environment=CHARIOX_SLICE_ROOT=/var/lib/chariox-slice-share/slices",
        "Environment=CHARIOX_MANAGED_VAULT_PATH=/var/lib/chariox/home/.chariox/vault/vault.json",
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
        "ProtectSystem=strict",
        "ProtectKernelTunables=false",
        "StateDirectory=chariox",
        "StateDirectoryMode=0700",
        "ReadWritePaths=/var/lib/chariox /var/lib/chariox-slice-share",
        "UMask=0007",
    ] {
        assert!(
            unit.contains(required),
            "missing systemd contract: {required}"
        );
    }
    assert!(!unit.contains("cloud-final.service"));
    assert!(!unit.contains("ssh"));
    assert!(!unit.contains("Requires=chariox-rootless-docker.service"));
    assert!(!unit.contains("Environment=DOCKER_HOST="));
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
        "ExecStart=/usr/share/docker.io/contrib/dockerd-rootless.sh",
        "RuntimeDirectory=chariox-docker",
        "RuntimeDirectoryMode=0700",
        "StateDirectory=chariox-docker",
        "StateDirectoryMode=0700",
        "Delegate=yes",
        "ProtectSystem=strict",
        "ExecStart=/usr/share/docker.io/contrib/dockerd-rootless.sh --host=unix:///run/chariox-docker/docker.sock --data-root=/var/lib/chariox-docker/data --exec-opt native.cgroupdriver=cgroupfs",
        "ProtectKernelTunables=false",
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
        "ReadWritePaths=/var/lib/chariox-docker /var/lib/chariox-slice-share /run/chariox-docker",
    ] {
        assert!(
            unit.contains(required),
            "missing slice broker contract: {required}"
        );
    }
    assert!(broker.contains("server.close()"));
    assert!(broker.contains("rmSync(SOCKET_PATH, { force: true })"));
    assert!(broker.contains("Docker command shape is not allowed"));
    assert!(broker.contains("must stay under the managed slice share"));
    assert!(broker.contains("chariox-slice-build-context"));
    assert!(!unit.contains("/var/lib/chariox/home"));
}

#[test]
fn managed_systemd_unit_remains_eligible_after_one_time_envelope_removal() {
    let unit = include_str!("../../../../deploy/managed-kernel/chariox-managed-bootstrap.service");
    assert!(unit.contains("ConditionPathExists=/usr/local/bin/chariox-managed-bootstrap"));
    assert!(!unit.contains("ConditionPathExists=/var/lib/chariox/managed-bootstrap.json"));
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
        let kernel_fixture = b"#!/bin/sh\nreceipt=\"$CHARIOX_HOME/managed/bootstrap-receipt.json\"\nif grep -Eq '\"status\"[[:space:]]*:[[:space:]]*\"confirmed\"' \"$receipt\"; then\n  state=confirmed\nelif grep -Eq '\"status\"[[:space:]]*:[[:space:]]*\"exchanged\"' \"$receipt\"; then\n  state=exchanged\nelse\n  state=invalid\nfi\nprintf '%s\\n' \"$state\" >> \"$CHARIOX_HOME/managed/kernel-started\"\ntest -s \"$CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE\"\nrm -f -- \"$CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE\"\nsleep 1\n";
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
