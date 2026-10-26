//! Cloud-coordinated in-place release update of a Path-1 managed kernel (plan,
//! locked decisions for 2026-09-30).
//!
//! The kernel polls Cloud with its running release. For an authorized update it
//! downloads the signed release archive and starts a detached root unit that runs
//! the signed upgrade transaction from the currently installed release. The
//! kernel stays silent until a root-owned durable transaction result admits the
//! target release. An interrupted activation remains pending until recovery
//! commits or finishes rollback, even if its transient unit has already stopped.

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

#[path = "managed_release_update_evidence.rs"]
mod evidence;
use evidence::{
    prepare_archive, read_evidence, settled_report, unit_settled, UpdateIdentity, UpdateReport,
};

const POLL_ENDPOINT: &str = "/v1/managed-kernels/release-update/poll";
const ARTIFACT_ENDPOINT: &str = "/v1/managed-kernels/release-update/artifact";
const POLL_INTERVAL: Duration = Duration::from_secs(60);
const UPDATE_UNIT: &str = "chariox-release-update";
const DOWNLOAD_ROOT: &str = "/var/tmp/chariox-release-update";
const STAGING_ROOT: &str = "/var/lib/chariox-release-update";
const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const CURRENT_RELEASE: &str = "/usr/lib/chariox/current";
const UPDATE_EVIDENCE: &str = "/usr/lib/chariox/.managed-kernel-upgrade-result";
const UPDATE_TRANSACTION: &str = "/usr/lib/chariox/.managed-kernel-upgrade";
const TERMINAL_TRANSACTION: &str = "/usr/lib/chariox/.managed-kernel-upgrade.terminal";
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
    #[serde(default)]
    from_runtime_release_digest: Option<String>,
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
        // systemd errors are not evidence that an update has settled.
        if !update_unit_settled()? {
            return Ok(());
        }
        let running = running_release_digest(&self.receipt_path)?;
        let attempt = read_attempt(&self.attempt_path)?;
        let recovery_pending = recovery_pending()?;
        // MP-07: after a stopped unit or reboot, the durable attempt owns
        // recovery. Waiting silently would strand both Cloud and the journal.
        if recovery_pending {
            let attempt = attempt
                .as_ref()
                .ok_or_else(|| update_error("release recovery has no durable update attempt"))?;
            let from = attempt
                .from_runtime_release_digest
                .as_ref()
                .ok_or_else(|| update_error("release recovery has no original release digest"))?;
            return self.start_update_unit(
                &UpdateCommand {
                    update_id: attempt.update_id.clone(),
                    from_runtime_release_digest: from.clone(),
                    target_runtime_release_digest: attempt.target_runtime_release_digest.clone(),
                },
                &Path::new(DOWNLOAD_ROOT).join(format!("{}.tar.gz", attempt.update_id)),
            );
        }
        let terminal_evidence = read_evidence(Path::new(UPDATE_EVIDENCE))
            .map_err(|error| update_error(format!("read release update evidence: {error}")))?;
        let report = attempt.as_ref().map(|attempt| {
            settled_report(
                &running,
                &UpdateIdentity {
                    update_id: &attempt.update_id,
                    from_digest: attempt.from_runtime_release_digest.as_deref(),
                    target_digest: &attempt.target_runtime_release_digest,
                    environment_id: &self.binding.environment_id,
                    machine_id: &self.binding.machine_id,
                    kernel_id: &self.binding.kernel_id,
                },
                terminal_evidence.as_deref(),
                recovery_pending,
            )
        });
        if report == Some(UpdateReport::Pending) {
            return Ok(());
        }
        // Reclaim the persisted, settled attempt before polling. Cloud can
        // hand out a successor in that response; cleanup failure must not claim
        // a command that this kernel cannot yet durably prepare.
        if matches!(report, Some(UpdateReport::Failed | UpdateReport::Applied)) {
            if let Some(attempt) = &attempt {
                cleanup_attempt_storage(&attempt.update_id)?;
            }
        }
        let failed = attempt
            .as_ref()
            .filter(|_| report == Some(UpdateReport::Failed))
            .map(|attempt| attempt.update_id.clone());
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
        if matches!(report, Some(UpdateReport::Failed | UpdateReport::Applied)) {
            std::fs::remove_file(&self.attempt_path)
                .map_err(|error| update_error(format!("clear release update attempt: {error}")))?;
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
        let attempt = UpdateAttempt {
            update_id: update.update_id.clone(),
            from_runtime_release_digest: Some(update.from_runtime_release_digest.clone()),
            target_runtime_release_digest: update.target_runtime_release_digest.clone(),
        };
        let bytes =
            serde_json::to_vec(&attempt).map_err(|error| update_error(error.to_string()))?;
        let mut values = identity_values(&self.binding);
        values.insert("protocolVersion", json!(1));
        values.insert("action", json!("release_update_artifact"));
        values.insert("updateId", json!(update.update_id));
        let mut prepared = prepare_archive(
            &self.attempt_path,
            &archive,
            &bytes,
            async {
                std::fs::create_dir_all(DOWNLOAD_ROOT)
                    .map_err(|error| update_error(format!("create download directory: {error}")))?;
                post_cloud_to_file(
                    self.binding.api_url.clone(),
                    ARTIFACT_ENDPOINT,
                    self.signed(values)?,
                    archive.clone(),
                    MAX_ARTIFACT_BYTES,
                )
                .await
            },
            |error| update_error(format!("record release update attempt: {error}")),
        )
        .await?;
        prepared.delegate();
        self.start_update_unit(&update, &archive)
    }

    fn start_update_unit(&self, update: &UpdateCommand, archive: &Path) -> Result<(), DaemonError> {
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
                archive,
                Path::new(STAGING_ROOT),
                update,
            ))
            .status()
            .map_err(|error| update_error(format!("start release update unit: {error}")))?;
        if !status.success() {
            // The CLI reply can be ambiguous. The next poll checks the unit
            // and recovery journal before it reports this attempt as failed.
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

/// The root unit extracts the archive and runs the signed upgrade transaction with
/// the installed release's tooling, resolved to its physical directory so the
/// activation of `current` cannot swap scripts mid-transaction. MP-07: retain
/// extracted recovery inputs while a journal remains, including after interruption.
/// Reclaim the archive immediately after extraction; recovery needs no archive.
/// The settled attempt reclaims scratch through the same owned storage helper.
fn update_script(
    tooling_release: &Path,
    archive: &Path,
    staging_root: &Path,
    update: &UpdateCommand,
) -> String {
    let staging = staging_root.join(&update.update_id);
    let tooling = tooling_release.join("usr/lib/chariox/slice-build-context/deploy/managed-kernel");
    let release_key = tooling_release.join("usr/lib/chariox/release-public-key");
    let publication_root = tooling_release.parent().unwrap_or(tooling_release);
    let authority_root = publication_root.parent().unwrap_or(publication_root);
    format!(
        "set -eu; pending() {{ [ -e '{authority_root}/.managed-kernel-upgrade' ] || [ -L '{authority_root}/.managed-kernel-upgrade' ] || \
         [ -e '{authority_root}/.managed-kernel-upgrade.terminal' ] || [ -L '{authority_root}/.managed-kernel-upgrade.terminal' ]; }}; \
         trap \"if ! pending; then python3 '{tooling}/release-update-storage.py' cleanup '{staging_root}' '{update_id}'; rm -f '{archive}'; fi\" EXIT; \
         run_upgrade() {{ CHARIOX_MANAGED_UPGRADE_RECOVER_ONLY=$1 CHARIOX_MANAGED_RELEASE_UPDATE_ID='{update_id}' \
         CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1 CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY={builder_key} \
         sh '{tooling}/upgrade-image.sh' '{staging}/extracted/rootfs' '{from}' '{target}' '{release_key}'; }}; \
         if pending; then run_upgrade 1; else \
         python3 '{tooling}/release-update-storage.py' prepare '{staging_root}' '{update_id}'; \
         python3 '{tooling}/extract-release.py' '{archive}' '{staging}/extracted' '{publication_root}'; \
         rm -f '{archive}'; TMPDIR='{staging}' run_upgrade 0; fi",
        staging = staging.display(),
        archive = archive.display(),
        staging_root = staging_root.display(),
        publication_root = publication_root.display(),
        authority_root = authority_root.display(),
        update_id = update.update_id,
        builder_key = TRUSTED_BUILDER_PUBLIC_KEY,
        tooling = tooling.display(),
        from = update.from_runtime_release_digest,
        target = update.target_runtime_release_digest,
        release_key = release_key.display(),
    )
}

fn cleanup_attempt_storage(update_id: &str) -> Result<(), DaemonError> {
    let tooling = std::fs::canonicalize(CURRENT_RELEASE)
        .map_err(|error| update_error(format!("resolve cleanup tooling: {error}")))?
        .join(
            "usr/lib/chariox/slice-build-context/deploy/managed-kernel/release-update-storage.py",
        );
    let status = Command::new("sudo")
        .args(["-n", "python3"])
        .arg(tooling)
        .args(["cleanup", STAGING_ROOT, update_id])
        .status()
        .map_err(|error| update_error(format!("clean settled release update: {error}")))?;
    if !status.success() {
        return Err(update_error(
            "settled release update scratch could not be reclaimed",
        ));
    }
    for suffix in ["tar.gz", "tar.partial"] {
        let path = Path::new(DOWNLOAD_ROOT).join(format!("{update_id}.{suffix}"));
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(update_error(format!(
                    "clean settled release download: {error}"
                )))
            }
        }
    }
    Ok(())
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

fn read_attempt(path: &Path) -> Result<Option<UpdateAttempt>, DaemonError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(update_error(format!(
                "read release update attempt: {error}"
            )))
        }
    };
    let attempt: UpdateAttempt = serde_json::from_slice(&bytes)
        .map_err(|error| update_error(format!("decode release update attempt: {error}")))?;
    if !valid_update_id(&attempt.update_id)
        || !valid_digest(&attempt.target_runtime_release_digest)
        || attempt
            .from_runtime_release_digest
            .as_deref()
            .is_some_and(|from| !valid_digest(from))
    {
        return Err(update_error("release update attempt identity is invalid"));
    }
    Ok(Some(attempt))
}

fn recovery_pending() -> Result<bool, DaemonError> {
    for path in [UPDATE_TRANSACTION, TERMINAL_TRANSACTION] {
        match std::fs::symlink_metadata(path) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(update_error(format!(
                    "inspect release update recovery journal: {error}"
                )));
            }
        }
    }
    Ok(false)
}

fn update_unit_settled() -> Result<bool, DaemonError> {
    let output = Command::new("systemctl")
        .args([
            "show",
            "--property=LoadState",
            "--property=ActiveState",
            UPDATE_UNIT,
        ])
        .output()
        .map_err(|error| update_error(format!("inspect release update unit: {error}")))?;
    let state = std::str::from_utf8(&output.stdout)
        .map_err(|_| update_error("release update unit state is not UTF-8"))?;
    unit_settled(output.status.code(), state).map_err(update_error)
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

    #[tokio::test]
    async fn a_failed_download_retains_the_exact_cloud_update_for_failure_reporting() {
        let root = std::env::temp_dir().join(format!(
            "chariox-release-download-failure-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir(&root).expect("scratch");
        let path = root.join("attempt.json");
        let archive = root.join("release.tar.gz");
        let attempt = UpdateAttempt {
            update_id: "managed_release_update_0123abcd-0000-4000-8000-0123456789ab".into(),
            from_runtime_release_digest: Some(digest('a')),
            target_runtime_release_digest: digest('b'),
        };
        let bytes = serde_json::to_vec(&attempt).expect("attempt");
        let result = prepare_archive(
            &path,
            &archive,
            &bytes,
            async { Err(update_error("artifact unavailable")) },
            |error| update_error(error.to_string()),
        )
        .await;
        assert!(result.is_err());
        let retained = read_attempt(&path)
            .expect("valid attempt")
            .expect("persisted attempt");
        assert_eq!(
            retained, attempt,
            "the next Cloud poll retains its failedUpdateId"
        );
        assert_eq!(
            settled_report(
                &digest('a'),
                &UpdateIdentity {
                    update_id: &retained.update_id,
                    from_digest: retained.from_runtime_release_digest.as_deref(),
                    target_digest: &retained.target_runtime_release_digest,
                    environment_id: "environment-1",
                    machine_id: "machine-1",
                    kernel_id: "kernel-1",
                },
                None,
                false,
            ),
            UpdateReport::Failed,
        );
        std::fs::remove_dir_all(root).expect("known synthetic scratch");
    }

    #[test]
    fn the_update_runs_installed_tooling_and_retains_recovery_inputs_until_settled() {
        let root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary root")
            .join(format!(
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
        std::fs::write(
            tooling.join("storage-implementation.py"),
            include_bytes!("../../../../deploy/managed-kernel/release-update-storage.py"),
        )
        .expect("storage implementation");
        std::fs::write(tooling.join("release-update-storage.py"),
            "import os, runpy, sys\nfrom pathlib import Path\nm = runpy.run_path(str(Path(__file__).with_name('storage-implementation.py')))\nm[sys.argv[1]](sys.argv[2], sys.argv[3], expected_uid=os.geteuid())\n")
            .expect("storage fixture");
        std::fs::create_dir_all(root.join("source/rootfs")).expect("rootfs");
        std::fs::write(root.join("source/rootfs/marker"), "release").expect("marker");
        // Incompressible, so a truncated archive fails after extracting the marker.
        let noise: Vec<u8> = (0..1_000_000u32)
            .map(|index| (index.wrapping_mul(2_654_443_761) >> 13) as u8)
            .collect();
        std::fs::write(root.join("source/rootfs/noise"), noise).expect("noise");
        std::fs::write(
            tooling.join("upgrade-image.sh"),
            format!(
                "set -eu; test -f \"$1/marker\"; test \"$TMPDIR\" = \"$(dirname \"$(dirname \"$1\")\")\"; mkdir \"$TMPDIR/child-scratch\"; echo \"$*\" > '{}'\n",
                root.join("invoked").display()
            ),
        )
        .expect("upgrade script");
        let update = UpdateCommand {
            update_id: "managed_release_update_01234567-89ab-cdef-0123-456789abcdef".into(),
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
                staging
                    .join(&update.update_id)
                    .join("extracted/rootfs")
                    .display(),
                digest('a'),
                digest('b'),
                root.join("release/usr/lib/chariox/release-public-key")
                    .display()
            )
        );
        // MP-07: an interrupted activation must remain recoverable when the
        // same root unit restarts, without another Cloud artifact download.
        let transaction = root.join(".managed-kernel-upgrade");
        std::fs::write(
            tooling.join("upgrade-image.sh"),
            format!(
                "set -eu; if [ ! -f '{0}/interrupted' ]; then test -f \"$1/marker\"; mkdir '{1}'; echo activated > '{1}/phase'; touch '{0}/interrupted'; exit 1; fi; test \"$CHARIOX_MANAGED_UPGRADE_RECOVER_ONLY\" = 1; test -f '{1}/phase'; rm -rf '{1}'\n",
                root.display(), transaction.display(),
            ),
        ).expect("interrupted upgrade fixture");
        std::fs::write(&archive, &packed).expect("archive");
        let restart = || {
            Command::new("sh")
                .arg("-c")
                .arg(update_script(
                    &root.join("releases/release"),
                    &archive,
                    &staging,
                    &update,
                ))
                .status()
                .expect("root updater")
        };
        // Use a release-layout path so the production wrapper sees the actual
        // journal authority beside releases, independently of current.
        std::fs::create_dir_all(root.join("releases")).unwrap();
        std::fs::rename(root.join("release"), root.join("releases/release")).unwrap();
        assert!(!restart().success());
        assert!(
            !archive.exists(),
            "MP-07 extraction must reclaim the archive before activation/recovery"
        );
        assert!(
            staging.join(&update.update_id).exists(),
            "MP-07 retain owned scratch until recovery"
        );
        assert!(
            restart().success(),
            "MP-07 restart must reach recovery without an archive"
        );
        assert!(!archive.exists() && !staging.join(&update.update_id).exists());
        assert!(!transaction.exists());
        assert!(valid_digest(&digest('b')) && !valid_digest("sha256:../../etc"));
        assert!(valid_update_id(
            "managed_release_update_0123abcd-0000-4000-8000-0123456789ab"
        ));
        assert!(!valid_update_id("managed_release_update_1'; rm -rf / #"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
