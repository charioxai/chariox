//! MP-07/MP-11 security anchor: an interrupted Path-1 release activation can leave the
//! pinned receipt and the `current` link on different releases. The kernel, which
//! normally runs the recovery pass, then never starts, so the bootstrap triggers
//! the same root recovery pass itself, and only for a root-private journal.
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::DaemonError;

const JOURNALS: [&str; 2] = [
    "/usr/lib/chariox/.managed-kernel-upgrade",
    "/usr/lib/chariox/.managed-kernel-upgrade.terminal",
];
const CURRENT_RELEASE: &str = "/usr/lib/chariox/current";
const TRUSTED_BUILDER_PUBLIC_KEY: &str = "/etc/chariox/trusted-builder-public-key";
const RECOVERY_UNIT: &str = "chariox-release-recovery";
const RECOVERY_TIMEOUT: &str = "600s";
/// The journal identity is `1`, update id, from digest, target digest, ...; the
/// updater re-validates the whole journal against these before it changes anything.
const RECOVERY_SCRIPT: &str = r#"set -eu
for journal in "$1" "$2"; do
  [ -d "$journal" ] && [ ! -L "$journal" ] && [ -f "$journal/update-result-identity" ] && break
  journal=
done
[ -n "$journal" ]
identity() { sed -n "$1p" "$journal/update-result-identity"; }
id=$(identity 2); from=$(identity 3); target=$(identity 4)
printf '%s' "$id" | grep -Eq '^managed_release_update_[a-f0-9-]{36}$'
printf '%s' "$from" | grep -Eq '^sha256:[a-f0-9]{64}$'
printf '%s' "$target" | grep -Eq '^sha256:[a-f0-9]{64}$'
CHARIOX_MANAGED_UPGRADE_RECOVER_ONLY=1 CHARIOX_MANAGED_RELEASE_UPDATE_ID=$id \
  CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1 CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY="$4" \
  exec sh "$3/usr/lib/chariox/slice-build-context/deploy/managed-kernel/upgrade-image.sh" \
  /nonexistent "$from" "$target" "$3/usr/lib/chariox/release-public-key"
"#;

/// Retry `verify` once after a pending interrupted update was recovered.
pub(super) fn verify_or_recover<T>(
    mut verify: impl FnMut() -> Result<T, DaemonError>,
    recover: impl FnOnce() -> Result<bool, DaemonError>,
) -> Result<T, DaemonError> {
    match verify() {
        Err(error) => {
            if recover()? {
                verify()
            } else {
                Err(error)
            }
        }
        ok => ok,
    }
}

/// True when a root-owned, non-writable journal directory exists. Anything the
/// unprivileged service could have planted is ignored.
pub(super) fn journal_pending(journals: &[PathBuf], root_uid: u32) -> bool {
    journals.iter().any(|path| {
        fs::symlink_metadata(path).is_ok_and(|metadata| {
            metadata.is_dir() && metadata.uid() == root_uid && metadata.mode() & 0o022 == 0
        })
    })
}

fn start_recovery(journals: &[PathBuf]) -> Result<(), DaemonError> {
    let tooling = fs::canonicalize(CURRENT_RELEASE)
        .map_err(|_| super::bootstrap_error("current release could not be resolved"))?;
    let status = Command::new("/usr/bin/sudo")
        .args([
            "-n",
            "systemd-run",
            "--quiet",
            "--collect",
            "--wait",
            "--unit",
        ])
        .arg(RECOVERY_UNIT)
        .arg(format!("--property=RuntimeMaxSec={RECOVERY_TIMEOUT}"))
        .args(["/bin/sh", "-c", RECOVERY_SCRIPT, "recover"])
        .args(journals)
        .arg(tooling)
        .arg(TRUSTED_BUILDER_PUBLIC_KEY)
        .status()
        .map_err(|_| super::bootstrap_error("interrupted release recovery could not start"))?;
    if status.success() {
        Ok(())
    } else {
        Err(super::bootstrap_error(
            "interrupted release recovery failed",
        ))
    }
}

/// Recover when `journals` hold a pending journal owned by `root_uid`.
pub(super) fn recover_pending_with(
    journals: &[PathBuf],
    root_uid: u32,
    start: impl FnOnce(&[PathBuf]) -> Result<(), DaemonError>,
) -> Result<bool, DaemonError> {
    if !journal_pending(journals, root_uid) {
        return Ok(false);
    }
    start(journals).map(|()| true)
}

pub(super) fn recover_pending() -> Result<bool, DaemonError> {
    if std::env::var(super::MANAGED_PROVIDER_TOPOLOGY_ENV).as_deref() != Ok("path1") {
        return Ok(false);
    }
    let journals: Vec<PathBuf> = JOURNALS
        .iter()
        .map(Path::new)
        .map(Path::to_path_buf)
        .collect();
    recover_pending_with(&journals, 0, start_recovery)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::os::unix::fs::{symlink, PermissionsExt};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "chariox-release-recovery-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn error() -> DaemonError {
        super::super::bootstrap_error("verification failed")
    }

    fn own_uid(path: &Path) -> u32 {
        fs::metadata(path).unwrap().uid()
    }

    fn journal(root: &Path) -> PathBuf {
        let path = root.join(".managed-kernel-upgrade");
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn mismatch_with_a_journal_recovers_once_and_retries_verification() {
        let calls = Cell::new(0);
        let verified = verify_or_recover(
            || {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    Err(error())
                } else {
                    Ok("verified")
                }
            },
            || Ok(true),
        );
        assert_eq!(verified.unwrap(), "verified");
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn mismatch_without_a_journal_is_still_refused() {
        let root = Scratch::new("case");
        let journals = [root.path().join(".managed-kernel-upgrade")];
        let started = Cell::new(false);
        let recovered = verify_or_recover(
            || Err::<(), _>(error()),
            || {
                recover_pending_with(&journals, own_uid(root.path()), |_| {
                    started.set(true);
                    Ok(())
                })
            },
        );
        assert!(recovered
            .unwrap_err()
            .to_string()
            .contains("verification failed"));
        assert!(!started.get());
    }

    #[test]
    fn a_journal_the_service_could_have_forged_is_ignored() {
        let root = Scratch::new("case");
        let real = journal(root.path());
        let uid = own_uid(root.path());
        assert!(journal_pending(&[real.clone()], uid));
        // Not owned by root.
        assert!(!journal_pending(&[real.clone()], uid + 1));
        // Writable by others.
        fs::set_permissions(&real, fs::Permissions::from_mode(0o770)).unwrap();
        assert!(!journal_pending(&[real.clone()], uid));
        // A symlink to a valid directory, or a plain file.
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        let file = root.path().join("file");
        fs::write(&file, b"x").unwrap();
        assert!(!journal_pending(&[link, file], uid));
    }

    fn run_script(root: &Path, identity: &str) -> (bool, String) {
        let journal = journal(root);
        fs::write(journal.join("update-result-identity"), identity).unwrap();
        let tooling =
            root.join("tooling/usr/lib/chariox/slice-build-context/deploy/managed-kernel");
        fs::create_dir_all(&tooling).unwrap();
        fs::write(
            tooling.join("upgrade-image.sh"),
            "printf '%s|%s|%s|%s|%s|%s|%s' \"$CHARIOX_MANAGED_UPGRADE_RECOVER_ONLY\" \
             \"$CHARIOX_MANAGED_RELEASE_UPDATE_ID\" \"$CHARIOX_MANAGED_PROVIDER_TOPOLOGY\" \"$1\" \"$2\" \"$3\" \"$4\"",
        )
        .unwrap();
        let output = Command::new("/bin/sh")
            .args(["-c", RECOVERY_SCRIPT, "recover"])
            .arg(root.join("missing"))
            .arg(&journal)
            .arg(root.join("tooling"))
            .arg("/etc/builder-key")
            .output()
            .unwrap();
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    #[test]
    fn the_recovery_pass_takes_its_identity_from_the_journal() {
        let root = Scratch::new("script");
        let (from, target) = (
            format!("sha256:{}", "a".repeat(64)),
            format!("sha256:{}", "b".repeat(64)),
        );
        let id = "managed_release_update_01234567-89ab-cdef-0123-456789abcdef";
        let (ok, out) = run_script(
            root.path(),
            &format!("1\n{id}\n{from}\n{target}\nx\ny\nz\n"),
        );
        assert!(ok);
        let key = format!(
            "{}/tooling/usr/lib/chariox/release-public-key",
            root.path().display()
        );
        assert_eq!(
            out,
            format!("1|{id}|path1|/nonexistent|{from}|{target}|{key}")
        );
    }

    #[test]
    fn a_malformed_journal_identity_is_refused_before_any_update_runs() {
        let root = Scratch::new("malformed");
        let (ok, out) = run_script(root.path(), "1\nnot-an-update\nsha256:00\nsha256:11\n");
        assert!(!ok);
        assert!(out.is_empty());
    }

    #[test]
    fn a_pending_root_journal_starts_the_recovery_pass() {
        let root = Scratch::new("case");
        let journals = [journal(root.path())];
        let started = Cell::new(false);
        let recovered = recover_pending_with(&journals, own_uid(root.path()), |_| {
            started.set(true);
            Ok(())
        });
        assert!(recovered.unwrap());
        assert!(started.get());
    }
}
