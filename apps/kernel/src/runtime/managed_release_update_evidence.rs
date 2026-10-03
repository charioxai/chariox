//! Decides when a settled managed update can be reported to Cloud.

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UpdateReport {
    Pending,
    Applied,
    Failed,
}

pub(crate) struct UpdateIdentity<'a> {
    pub(crate) update_id: &'a str,
    pub(crate) from_digest: Option<&'a str>,
    pub(crate) target_digest: &'a str,
    pub(crate) environment_id: &'a str,
    pub(crate) machine_id: &'a str,
    pub(crate) kernel_id: &'a str,
}

pub(crate) fn settled_report(
    running: &str,
    identity: &UpdateIdentity<'_>,
    evidence: Option<&str>,
    recovery_pending: bool,
) -> UpdateReport {
    // Even terminal journals must finish restart recovery before the attempt
    // is retired. This also prevents an older matching result from admitting
    // a repeated attempt while its new activation is still provisional.
    if recovery_pending {
        return UpdateReport::Pending;
    }
    if let Some(evidence) = evidence {
        let fields: Vec<_> = evidence.lines().collect();
        if fields.len() == 8
            && fields[0] == "1"
            && fields[1] == identity.update_id
            && identity.from_digest.is_none_or(|from| fields[2] == from)
            && fields[3] == identity.target_digest
            && fields[4] == identity.environment_id
            && fields[5] == identity.machine_id
            && fields[6] == identity.kernel_id
        {
            return match fields[7] {
                "committed" if running == identity.target_digest => UpdateReport::Applied,
                "rolled_back" if running == fields[2] => UpdateReport::Failed,
                _ => UpdateReport::Pending,
            };
        }
    }
    // Receipt B is installed before its supervisor starts. Receipt A can also
    // return before rollback passes health admission. Keep the attempt in both
    // windows. No journal and a previous release means failure before mutation
    // or a completed legacy rollback.
    if running == identity.target_digest {
        UpdateReport::Pending
    } else {
        UpdateReport::Failed
    }
}

/// A collected transient unit is absent, not an unknown systemd result. Accept
/// only explicit known states, including the full not-found/inactive pair.
pub(crate) fn unit_settled(status: Option<i32>, output: &str) -> Result<bool, &'static str> {
    let fields: Vec<_> = output.lines().collect();
    if fields.len() != 2 {
        return Err("could not inspect release update unit state");
    }
    let load = fields
        .iter()
        .find_map(|field| field.strip_prefix("LoadState="));
    let active = fields
        .iter()
        .find_map(|field| field.strip_prefix("ActiveState="));
    match (status, load, active) {
        (Some(0 | 1), Some("not-found"), Some("inactive")) => Ok(true),
        (Some(0), Some("loaded"), Some("inactive" | "failed")) => Ok(true),
        (Some(0), Some("loaded"), Some("active" | "activating" | "deactivating" | "reloading")) => {
            Ok(false)
        }
        _ => Err("could not verify release update unit state"),
    }
}

/// The result is public identity evidence written by root, separate from the
/// private recovery journal. Validate the opened file as well as the pathname.
#[cfg(unix)]
pub(crate) fn read_evidence(path: &std::path::Path) -> std::io::Result<Option<String>> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;

    let invalid = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unsafe release update evidence",
        )
    };
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let parent = std::fs::symlink_metadata(path.parent().ok_or_else(invalid)?)?;
    if !parent.is_dir()
        || parent.uid() != 0
        || parent.mode() & 0o022 != 0
        || !metadata.is_file()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || metadata.len() > 4096
    {
        return Err(invalid());
    }
    let file = std::fs::File::open(path)?;
    let opened = file.metadata()?;
    if opened.dev() != metadata.dev()
        || opened.ino() != metadata.ino()
        || !opened.is_file()
        || opened.uid() != 0
        || opened.mode() & 0o022 != 0
    {
        return Err(invalid());
    }
    let mut contents = String::new();
    file.take(4097).read_to_string(&mut contents)?;
    if contents.len() > 4096 {
        return Err(invalid());
    }
    Ok(Some(contents))
}

#[cfg(not(unix))]
pub(crate) fn read_evidence(_path: &std::path::Path) -> std::io::Result<Option<String>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "managed release update evidence requires Unix ownership checks",
    ))
}

/// Before delegation, only this kernel can own these named download files.
/// Once launch is attempted the root unit may own them, even if its CLI errors.
pub(crate) struct PreparedArchive {
    archive: std::path::PathBuf,
    delegated: bool,
}

impl PreparedArchive {
    pub(crate) fn delegate(&mut self) {
        self.delegated = true;
    }
}

impl Drop for PreparedArchive {
    fn drop(&mut self) {
        if !self.delegated {
            let _ = std::fs::remove_file(&self.archive);
            let _ = std::fs::remove_file(self.archive.with_extension("partial"));
        }
    }
}

/// Preserve the Cloud command across download failure or kernel interruption.
pub(crate) async fn prepare_archive<E>(
    attempt_path: &std::path::Path,
    archive: &std::path::Path,
    attempt_bytes: &[u8],
    download: impl std::future::Future<Output = Result<(), E>>,
    io_error: impl Fn(std::io::Error) -> E,
) -> Result<PreparedArchive, E> {
    let prepared = PreparedArchive {
        archive: archive.to_path_buf(),
        delegated: false,
    };
    std::fs::write(attempt_path, attempt_bytes)
        .and_then(|()| std::fs::File::open(attempt_path)?.sync_all())
        .and_then(|()| {
            std::fs::File::open(
                attempt_path
                    .parent()
                    .ok_or_else(|| std::io::Error::other("release update attempt has no parent"))?,
            )?
            .sync_all()
        })
        .map_err(io_error)?;
    download.await?;
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> UpdateIdentity<'static> {
        UpdateIdentity {
            update_id: "managed_release_update_0123abcd-0000-4000-8000-0123456789ab",
            from_digest: Some("release-a"),
            target_digest: "release-b",
            environment_id: "environment-1",
            machine_id: "machine-1",
            kernel_id: "kernel-1",
        }
    }

    fn evidence(phase: &str) -> String {
        format!(
            "1\n{}\nrelease-a\nrelease-b\nenvironment-1\nmachine-1\nkernel-1\n{phase}\n",
            identity().update_id
        )
    }

    #[test]
    fn a_clean_commit_and_completed_rollback_are_reportable() {
        assert_eq!(
            settled_report(
                "release-b",
                &identity(),
                Some(&evidence("committed")),
                false
            ),
            UpdateReport::Applied
        );
        assert_eq!(
            settled_report(
                "release-a",
                &identity(),
                Some(&evidence("rolled_back")),
                false
            ),
            UpdateReport::Failed
        );
        assert_eq!(
            settled_report("release-a", &identity(), None, false),
            UpdateReport::Failed,
            "preflight failure has no recovery journal"
        );
    }

    #[test]
    fn stale_or_foreign_terminal_evidence_does_not_admit_the_target() {
        let committed = evidence("committed");
        for (old, new) in [
            (identity().update_id, "another-update"),
            ("release-a", "another-from"),
            ("release-b", "another-target"),
            ("environment-1", "another-environment"),
            ("machine-1", "another-machine"),
            ("kernel-1", "another-kernel"),
            ("committed", "activated"),
        ] {
            assert_eq!(
                settled_report(
                    "release-b",
                    &identity(),
                    Some(&committed.replace(old, new)),
                    false
                ),
                UpdateReport::Pending,
                "{old}"
            );
        }
        assert_eq!(
            settled_report("release-b", &identity(), None, false),
            UpdateReport::Pending
        );
        assert_eq!(
            settled_report("release-b", &identity(), Some(&committed), true),
            UpdateReport::Pending,
            "recovery journal from a repeated attempt"
        );
        assert_eq!(
            settled_report(
                "release-b",
                &identity(),
                Some(&evidence("rolled_back")),
                false
            ),
            UpdateReport::Pending
        );
    }

    #[test]
    fn systemctl_errors_and_unknown_states_fail_closed() {
        for (status, output) in [
            (Some(1), ""),
            (Some(1), "LoadState=loaded\nActiveState=inactive\n"),
            (Some(0), "LoadState=loaded\nActiveState=unknown\n"),
            (Some(0), "LoadState=not-found\nActiveState=active\n"),
            (None, "LoadState=loaded\nActiveState=inactive\n"),
            (
                Some(0),
                "LoadState=loaded\nActiveState=inactive\nextra=field\n",
            ),
        ] {
            assert!(unit_settled(status, output).is_err(), "{output}");
        }
        assert_eq!(
            unit_settled(Some(0), "ActiveState=failed\nLoadState=loaded\n"),
            Ok(true)
        );
        assert_eq!(
            unit_settled(Some(0), "LoadState=loaded\nActiveState=active\n"),
            Ok(false)
        );
        assert_eq!(
            unit_settled(Some(1), "LoadState=not-found\nActiveState=inactive\n"),
            Ok(true)
        );
    }

    #[cfg(unix)]
    #[test]
    fn evidence_files_must_be_bounded_root_owned_regular_files() {
        use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
        let root =
            std::env::temp_dir().join(format!("chariox-update-evidence-{}", std::process::id()));
        std::fs::create_dir(&root).expect("scratch");
        let path = root.join("result");
        assert_eq!(read_evidence(&path).expect("missing"), None);
        std::fs::write(&path, evidence("committed")).expect("write");
        let is_root = std::fs::metadata(&path).expect("metadata").uid() == 0;
        if is_root {
            assert_eq!(
                read_evidence(&path).expect("evidence"),
                Some(evidence("committed"))
            );
        } else {
            assert!(read_evidence(&path).is_err());
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).expect("mode");
        assert!(read_evidence(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("mode");
        std::fs::write(&path, "x".repeat(4097)).expect("oversized");
        assert!(read_evidence(&path).is_err());
        let linked = root.join("linked");
        symlink(&path, &linked).expect("symlink");
        assert!(read_evidence(&linked).is_err());
        std::fs::remove_dir_all(root).expect("cleanup known test scratch");
    }

    fn ready<F: std::future::Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(result) => result,
            std::task::Poll::Pending => panic!("test download must complete immediately"),
        }
    }

    #[test]
    fn an_unavailable_artifact_keeps_the_attempt_for_the_next_cloud_poll() {
        let root =
            std::env::temp_dir().join(format!("chariox-download-failure-{}", std::process::id()));
        std::fs::create_dir(&root).expect("scratch");
        let attempt = root.join("attempt.json");
        let archive = root.join("release.tar.gz");
        let bytes = b"Cloud-authorized attempt";
        let download = async {
            std::fs::write(&archive, "stale incomplete archive").expect("archive");
            std::fs::write(archive.with_extension("partial"), "partial").expect("partial");
            Err(std::io::Error::other("artifact unavailable"))
        };
        assert!(ready(prepare_archive(
            &attempt,
            &archive,
            bytes,
            download,
            |error| error
        ))
        .is_err());
        assert_eq!(
            std::fs::read(&attempt).expect("Cloud RUNNING must retain a failedUpdateId"),
            bytes
        );
        assert!(!archive.exists() && !archive.with_extension("partial").exists());
        assert_eq!(
            settled_report("release-a", &identity(), None, false),
            UpdateReport::Failed
        );
        std::fs::remove_dir_all(root).expect("known synthetic scratch");
    }

    #[test]
    fn the_attempt_is_durable_before_requesting_the_artifact() {
        let root =
            std::env::temp_dir().join(format!("chariox-download-order-{}", std::process::id()));
        std::fs::create_dir(&root).expect("scratch");
        let attempt = root.join("attempt.json");
        let archive = root.join("release.tar.gz");
        let bytes = b"Cloud-authorized attempt";
        let download = async {
            assert_eq!(
                std::fs::read(&attempt).expect("attempt precedes artifact request"),
                bytes
            );
            std::fs::write(&archive, "downloaded").expect("archive");
            Ok::<_, std::io::Error>(())
        };
        let prepared = ready(prepare_archive(
            &attempt,
            &archive,
            bytes,
            download,
            |error| error,
        ))
        .expect("prepare");
        drop(prepared);
        assert!(attempt.exists() && !archive.exists());
        std::fs::remove_dir_all(root).expect("known synthetic scratch");
    }

    #[test]
    fn delegated_files_remain_owned_by_the_possible_root_unit() {
        let root = std::env::temp_dir().join(format!(
            "chariox-download-delegation-{}",
            std::process::id()
        ));
        std::fs::create_dir(&root).expect("scratch");
        let attempt = root.join("attempt.json");
        let archive = root.join("release.tar.gz");
        let download = async {
            std::fs::write(&archive, "downloaded").expect("archive");
            Ok::<_, std::io::Error>(())
        };
        let mut prepared = ready(prepare_archive(
            &attempt,
            &archive,
            b"attempt",
            download,
            |error| error,
        ))
        .expect("prepare");
        prepared.delegate();
        drop(prepared);
        assert!(
            attempt.exists() && archive.exists(),
            "a failed CLI reply cannot discard state the root unit may still use"
        );
        assert_eq!(
            settled_report("release-a", &identity(), None, true),
            UpdateReport::Pending
        );
        assert_eq!(
            settled_report("release-a", &identity(), None, false),
            UpdateReport::Failed
        );
        std::fs::remove_dir_all(root).expect("known synthetic scratch");
    }

    #[test]
    fn interrupted_target_is_not_reported_before_recovery() {
        assert_eq!(
            settled_report("release-b", &identity(), None, true),
            UpdateReport::Pending,
            "receipt B is provisional until a durable committed result exists",
        );
    }

    #[test]
    fn an_attempt_is_retained_until_rollback_finishes() {
        assert_eq!(
            settled_report("release-a", &identity(), None, true),
            UpdateReport::Pending,
            "a restored receipt does not prove rollback health admission finished",
        );
    }
}
