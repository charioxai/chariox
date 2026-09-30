//! Cloud-coordinated in-place release update of a Path-1 managed kernel (plan,
//! locked decisions for 2026-09-30).
//!
//! The kernel polls Cloud with its running release. For an authorized update it
//! downloads the signed release archive and starts a detached root unit that runs
//! the signed upgrade transaction from the currently installed release. The
//! kernel stays silent while that unit runs: the upgraded kernel starts before
//! the transaction commits. Once the unit settles, a kernel running the target
//! completes the update and one still running the previous release reports it
//! failed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::runtime::cloud_api_client::{post_cloud_json, post_cloud_to_file};

use super::managed_kernel_quiescence::{hmac_signature, identity_values, QuiescenceBinding};

const POLL_ENDPOINT: &str = "/v1/managed-kernels/release-update/poll";
const ARTIFACT_ENDPOINT: &str = "/v1/managed-kernels/release-update/artifact";
const POLL_INTERVAL: Duration = Duration::from_secs(60);
const UPDATE_UNIT: &str = "chariox-release-update";
const DOWNLOAD_ROOT: &str = "/var/tmp/chariox-release-update";
const STAGING_ROOT: &str = "/var/lib/chariox-release-update";
const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const CURRENT_RELEASE: &str = "/usr/lib/chariox/current";
const TRUSTED_BUILDER_PUBLIC_KEY: &str = "/etc/chariox/trusted-builder-public-key";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PollResponse {
    protocol_version: u8,
    update: Option<UpdateCommand>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCommand {
    update_id: String,
    from_runtime_release_digest: String,
    target_runtime_release_digest: String,
}

/// The update this kernel last started, kept so a restarted kernel can tell a
/// rollback from an update still running.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct UpdateAttempt {
    update_id: String,
    target_runtime_release_digest: String,
}

pub(crate) struct ManagedReleaseUpdateClient {
    binding: QuiescenceBinding,
    receipt_path: PathBuf,
    attempt_path: PathBuf,
}

impl ManagedReleaseUpdateClient {
    pub(crate) fn from_runtime(
        config: &DaemonConfig,
        registration: Option<&ConfirmedManagedKernelRegistration>,
    ) -> Result<Option<Self>, DaemonError> {
        let (Some(registration), Ok(crate::managed_bootstrap::ManagedProviderTopology::Path1)) = (
            registration,
            crate::managed_bootstrap::managed_provider_topology(),
        ) else {
            return Ok(None);
        };
        let profile = config
            .cloud_relay
            .as_ref()
            .ok_or_else(|| update_error("confirmed managed kernel has no Cloud relay profile"))?;
        let receipt_path = std::env::var_os("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT")
            .map(PathBuf::from)
            .ok_or_else(|| update_error("managed kernel has no bootstrap receipt"))?;
        Ok(Some(Self {
            binding: QuiescenceBinding::from_runtime(config, registration, profile)?,
            receipt_path,
            attempt_path: config
                .private_runtime_state_root()
                .join("release-update-attempt.json"),
        }))
    }

    pub(crate) async fn run(self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        loop {
            if let Err(error) = self.poll_once().await {
                crate::logging::warn_with_fields(
                    "managed_kernel.release_update",
                    "release update poll failed; retrying",
                    json!({ "error": error.to_string() }),
                );
            }
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return;
                    }
                }
                _ = tokio::time::sleep(POLL_INTERVAL) => {}
            }
        }
    }

    async fn poll_once(&self) -> Result<(), DaemonError> {
        // The upgraded kernel runs before health admission and the commit; its
        // release is not the machine's until the unit settles.
        if unit_is_active() {
            return Ok(());
        }
        let running = running_release_digest(&self.receipt_path)?;
        let attempt = read_attempt(&self.attempt_path);
        let failed = failed_attempt(attempt.as_ref(), &running);
        let mut values = identity_values(&self.binding);
        values.insert("protocolVersion", json!(1));
        values.insert("action", json!("release_update_poll"));
        values.insert("runtimeReleaseDigest", json!(running));
        if let Some(update_id) = &failed {
            values.insert("failedUpdateId", json!(update_id));
        }
        let response: PollResponse = post_cloud_json(
            self.binding.api_url.clone(),
            POLL_ENDPOINT,
            self.signed(values)?,
        )
        .await?;
        if response.protocol_version != 1 {
            return Err(update_error(
                "Cloud returned an unsupported release update protocol",
            ));
        }
        if failed.is_some()
            || attempt
                .as_ref()
                .is_some_and(|attempt| attempt.target_runtime_release_digest == running)
        {
            let _ = std::fs::remove_file(&self.attempt_path);
        }
        let Some(update) = response.update else {
            return Ok(());
        };
        // Cloud values reach a root shell: accept only exact identifiers and digests.
        if !valid_update_id(&update.update_id)
            || update.from_runtime_release_digest != running
            || !valid_digest(&update.target_runtime_release_digest)
            || attempt
                .as_ref()
                .is_some_and(|attempt| attempt.update_id == update.update_id && failed.is_none())
        {
            return Ok(());
        }
        self.start_update(update).await
    }

    async fn start_update(&self, update: UpdateCommand) -> Result<(), DaemonError> {
        let archive = Path::new(DOWNLOAD_ROOT).join(format!("{}.tar.gz", update.update_id));
        std::fs::create_dir_all(DOWNLOAD_ROOT)
            .map_err(|error| update_error(format!("create download directory: {error}")))?;
        let mut values = identity_values(&self.binding);
        values.insert("protocolVersion", json!(1));
        values.insert("action", json!("release_update_artifact"));
        values.insert("updateId", json!(update.update_id));
        post_cloud_to_file(
            self.binding.api_url.clone(),
            ARTIFACT_ENDPOINT,
            self.signed(values)?,
            archive.clone(),
            MAX_ARTIFACT_BYTES,
        )
        .await?;
        let attempt = UpdateAttempt {
            update_id: update.update_id.clone(),
            target_runtime_release_digest: update.target_runtime_release_digest.clone(),
        };
        let bytes =
            serde_json::to_vec(&attempt).map_err(|error| update_error(error.to_string()))?;
        std::fs::write(&self.attempt_path, bytes)
            .map_err(|error| update_error(format!("record release update attempt: {error}")))?;
        let tooling = std::fs::canonicalize(CURRENT_RELEASE)
            .map_err(|error| update_error(format!("resolve current release: {error}")))?;
        let status = Command::new("sudo")
            .args([
                "-n",
                "systemd-run",
                "--unit",
                UPDATE_UNIT,
                "--collect",
                "--quiet",
            ])
            .args(["/bin/sh", "-c"])
            .arg(update_script(
                &tooling,
                &archive,
                Path::new(STAGING_ROOT),
                &update,
            ))
            .status()
            .map_err(|error| update_error(format!("start release update unit: {error}")))?;
        if !status.success() {
            let _ = std::fs::remove_file(&self.attempt_path);
            return Err(update_error(format!(
                "release update unit did not start: {status}"
            )));
        }
        crate::logging::info_with_fields(
            "managed_kernel.release_update",
            "started Cloud-authorized release update",
            json!({
                "update_id": update.update_id,
                "target_runtime_release_digest": update.target_runtime_release_digest,
            }),
        );
        Ok(())
    }

    fn signed(&self, values: BTreeMap<&'static str, Value>) -> Result<Value, DaemonError> {
        let signature = hmac_signature(&self.binding.machine_credential, &values)?;
        let mut body = serde_json::Map::new();
        for (key, value) in values {
            body.insert(key.to_string(), value);
        }
        body.insert(
            "machineCredential".into(),
            json!(self.binding.machine_credential),
        );
        body.insert("signature".into(), json!(signature));
        Ok(Value::Object(body))
    }
}

/// A settled update that did not reach its target was rolled back or failed.
fn failed_attempt(attempt: Option<&UpdateAttempt>, running: &str) -> Option<String> {
    attempt
        .filter(|attempt| attempt.target_runtime_release_digest != running)
        .map(|attempt| attempt.update_id.clone())
}

/// The root unit extracts the archive and runs the signed upgrade transaction with
/// the installed release's tooling, resolved to its physical directory so the
/// activation of `current` cannot swap scripts mid-transaction. The archive and
/// its bounded extraction are removed however the unit ends.
fn update_script(
    tooling_release: &Path,
    archive: &Path,
    staging_root: &Path,
    update: &UpdateCommand,
) -> String {
    let staging = staging_root.join(&update.update_id);
    let tooling = tooling_release.join("usr/lib/chariox/slice-build-context/deploy/managed-kernel");
    let release_key = tooling_release.join("usr/lib/chariox/release-public-key");
    format!(
        "set -eu; trap \"rm -rf '{staging}' '{archive}'\" EXIT; rm -rf '{staging}'; mkdir -p '{staging_root}'; \
         python3 '{tooling}/extract-release.py' '{archive}' '{staging}'; \
         rm -f '{archive}'; CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1 CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY={builder_key} \
         sh '{tooling}/upgrade-image.sh' '{staging}/rootfs' '{from}' '{target}' '{release_key}'",
        staging = staging.display(),
        archive = archive.display(),
        staging_root = staging_root.display(),
        builder_key = TRUSTED_BUILDER_PUBLIC_KEY,
        tooling = tooling.display(),
        from = update.from_runtime_release_digest,
        target = update.target_runtime_release_digest,
        release_key = release_key.display(),
    )
}

fn running_release_digest(receipt_path: &Path) -> Result<String, DaemonError> {
    let bytes = std::fs::read(receipt_path)
        .map_err(|error| update_error(format!("read bootstrap receipt: {error}")))?;
    let receipt: Value = serde_json::from_slice(&bytes)
        .map_err(|error| update_error(format!("decode bootstrap receipt: {error}")))?;
    receipt
        .get("runtimeReleaseDigest")
        .and_then(Value::as_str)
        .filter(|digest| valid_digest(digest))
        .map(str::to_string)
        .ok_or_else(|| update_error("bootstrap receipt has no runtime release digest"))
}

fn read_attempt(path: &Path) -> Option<UpdateAttempt> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn unit_is_active() -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", UPDATE_UNIT])
        .status()
        .is_ok_and(|status| status.success())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid_update_id(value: &str) -> bool {
    value
        .strip_prefix("managed_release_update_")
        .is_some_and(|uuid| {
            uuid.len() == 36
                && uuid
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'-'))
        })
}

fn update_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "manage kernel release update",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(fill: char) -> String {
        format!("sha256:{}", fill.to_string().repeat(64))
    }

    #[test]
    fn a_settled_attempt_that_missed_its_target_is_reported_failed() {
        let attempt = UpdateAttempt {
            update_id: "managed_release_update_1".into(),
            target_runtime_release_digest: digest('b'),
        };
        assert_eq!(
            failed_attempt(Some(&attempt), &digest('a')).as_deref(),
            Some("managed_release_update_1"),
            "rolled back"
        );
        assert_eq!(
            failed_attempt(Some(&attempt), &digest('b')),
            None,
            "reached its target"
        );
        assert_eq!(failed_attempt(None, &digest('a')), None);
    }

    #[test]
    fn the_update_runs_the_installed_tooling_and_removes_its_files_however_it_ends() {
        let root = std::env::temp_dir().join(format!(
            "chariox-release-update-script-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let tooling =
            root.join("release/usr/lib/chariox/slice-build-context/deploy/managed-kernel");
        std::fs::create_dir_all(&tooling).expect("tooling");
        std::fs::write(
            tooling.join("extract-release.py"),
            include_bytes!("../../../../deploy/managed-kernel/extract-release.py"),
        )
        .expect("extractor");
        std::fs::create_dir_all(root.join("source/rootfs")).expect("rootfs");
        std::fs::write(root.join("source/rootfs/marker"), "release").expect("marker");
        // Incompressible, so a truncated archive fails after extracting the marker.
        let noise: Vec<u8> = (0..1_000_000u32)
            .map(|index| (index.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        std::fs::write(root.join("source/rootfs/noise"), noise).expect("noise");
        std::fs::write(
            tooling.join("upgrade-image.sh"),
            format!(
                "test -f \"$1/marker\"; echo \"$*\" > '{}'\n",
                root.join("invoked").display()
            ),
        )
        .expect("upgrade script");
        let update = UpdateCommand {
            update_id: "managed_release_update_1".into(),
            from_runtime_release_digest: digest('a'),
            target_runtime_release_digest: digest('b'),
        };
        let staging = root.join("staging");
        let archive = root.join("download.tar.gz");
        let run = |archive_bytes: &[u8]| {
            std::fs::write(&archive, archive_bytes).expect("archive");
            let status = Command::new("sh")
                .arg("-c")
                .arg(update_script(
                    &root.join("release"),
                    &archive,
                    &staging,
                    &update,
                ))
                .status()
                .expect("run update script");
            assert!(!archive.exists() && !staging.join(&update.update_id).exists());
            status.success()
        };
        let packed = Command::new("tar")
            .args(["-czf", "-", "-C"])
            .arg(root.join("source"))
            .args(["rootfs"])
            .output()
            .expect("pack archive")
            .stdout;

        assert!(
            !run(&packed[..packed.len() / 2]),
            "a truncated archive fails"
        );
        assert!(!root.join("invoked").exists());
        assert!(run(&packed));
        let invoked = std::fs::read_to_string(root.join("invoked")).expect("invoked");
        assert_eq!(
            invoked.trim(),
            format!(
                "{} {} {} {}",
                staging.join("managed_release_update_1/rootfs").display(),
                digest('a'),
                digest('b'),
                root.join("release/usr/lib/chariox/release-public-key")
                    .display()
            )
        );
        assert!(valid_digest(&digest('b')) && !valid_digest("sha256:../../etc"));
        assert!(valid_update_id(
            "managed_release_update_0123abcd-0000-4000-8000-0123456789ab"
        ));
        assert!(!valid_update_id("managed_release_update_1'; rm -rf / #"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
