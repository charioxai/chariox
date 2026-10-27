use super::*;
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    root: PathBuf,
    previous_path: Option<std::ffi::OsString>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-progress-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir(&root).unwrap();
        let docker = root.join("docker");
        std::fs::write(
            &docker,
            r#"#!/bin/sh
case "$4" in
 startup) sleep 0.3; printf private ;;
 bytes) printf private; sleep 0.3 ;;
 eof) printf private; exec 1>&-; sleep 0.3 ;;
 descendant) sleep 0.3 & printf private; exit 0 ;;
 delayed) (sleep 0.04; printf suffix) & printf prefix; exit 0 ;;
 healthy) for n in 1 2 3 4 5 6; do printf x; sleep 0.04; done ;;
 *) printf recovered ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700)).unwrap();
        let previous_path = std::env::var_os("PATH");
        let path = std::env::join_paths(std::iter::once(root.clone()).chain(
            std::env::split_paths(&previous_path.clone().unwrap_or_default()),
        ))
        .unwrap();
        std::env::set_var("PATH", path);
        Self {
            root,
            previous_path,
        }
    }
    fn capture(
        &self,
        mode: &str,
        timeout: Duration,
    ) -> Result<(PathBuf, u64, String), DaemonError> {
        capture_with_progress_timeout(
            mode,
            &self.root.join(mode),
            "test",
            || Ok(u64::MAX),
            timeout,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        match &self.previous_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn home_archive_stream_stalled_start_bytes_and_eof_settle_and_recover() {
    let _lock = crate::env_lock::lock();
    let fixture = Fixture::new();
    let previous = fixture.root.join("previous");
    std::fs::write(&previous, b"known-good").unwrap();
    for mode in ["startup", "bytes", "eof", "descendant"] {
        let started = Instant::now();
        let result = fixture.capture(mode, Duration::from_millis(70));
        assert!(
            matches!(result, Err(DaemonError::LocalTransport { ref message, .. }) if message.contains("made no progress")),
            "{mode} must report inactivity"
        );
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "{mode} must settle its producer"
        );
        assert!(
            !fixture.root.join(mode).exists(),
            "{mode} must discard only its partial"
        );
        assert_eq!(std::fs::read(&previous).unwrap(), b"known-good");
        let recovery = fixture.root.join("success");
        fixture
            .capture("success", Duration::from_millis(70))
            .unwrap();
        assert_eq!(std::fs::read(&recovery).unwrap(), b"recovered");
        std::fs::remove_file(recovery).unwrap();
    }
}

#[test]
fn home_archive_stream_healthy_progress_and_delayed_descendant_outlive_parent() {
    let _lock = crate::env_lock::lock();
    let fixture = Fixture::new();
    let started = Instant::now();
    let (path, size, _) = fixture
        .capture("healthy", Duration::from_millis(150))
        .unwrap();
    assert!(started.elapsed() > Duration::from_millis(200));
    assert_eq!(size, 6);
    assert_eq!(std::fs::read(path).unwrap(), b"xxxxxx");
    let (path, _, _) = fixture
        .capture("delayed", Duration::from_millis(150))
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"prefixsuffix");
}

#[test]
fn home_archive_stream_uses_shared_five_minute_progress_policy() {
    assert_eq!(progress_timeout(), Duration::from_millis(300_000));
}

#[test]
fn home_archive_stream_stall_automatically_recovers_public_live_snapshot_and_retains_generation() {
    let _lock = crate::env_lock::lock();
    let fixture = Fixture::new();
    let docker = fixture.root.join("docker");
    std::fs::write(&docker, r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
 "info --format {{.DockerRootDir}}") printf '/tmp\n'; exit 0 ;;
 *"inspect --size --format {{.SizeRw}}"*) printf '1048576\n'; exit 0 ;;
 *" du -sb /home-src") printf '1048576 /home-src\n'; exit 0 ;;
 *" find /home-src -printf . | wc -c") printf '1\n'; exit 0 ;;
 *" df -B1 --output=avail /tmp") printf '107374182400\n'; exit 0 ;;
 *"State.Running"*) cat "$root/status"; exit 0 ;;
 *"io.chariox.snapshot-helper"*) printf '%s\n' "$4"; exit 0 ;;
 *"tar --zstd -C /home-src -cf - .") printf 'private-partial'; sleep 0.3; exit 0 ;;
esac
case "$1" in
 ps) printf 'chariox-slice-dev\n'; test ! -f "$root/helpers" || cat "$root/helpers" ;;
 create) printf '%s\n' "$3" >> "$root/helpers" ;;
 pause) printf 'true paused\n' > "$root/status" ;;
 unpause) printf 'true running\n' > "$root/status" ;;
 rm) if test -f "$root/helpers"; then grep -v -F "$3" "$root/helpers" > "$root/next"; mv "$root/next" "$root/helpers"; fi ;;
esac
exit 0
"#).unwrap();
    std::fs::write(fixture.root.join("status"), "true running\n").unwrap();
    let record = super::super::tests::test_record();
    let mut options = super::super::tests::test_options();
    options.root = fixture.root.clone();
    let state_dir = fixture.root.join("states/dev");
    std::fs::create_dir_all(&state_dir).unwrap();
    let prior_archive = state_dir.join("prior.tar.zst");
    std::fs::write(&prior_archive, b"known-good-private-home").unwrap();
    std::fs::set_permissions(&prior_archive, std::fs::Permissions::from_mode(0o600)).unwrap();
    let manifest = state_dir.join("manifest.json");
    let prior = crate::slice::SliceSavedStateRecord {
        id: "dev".into(),
        slice_name: record.name.clone(),
        source_slice_id: record.id.clone(),
        backend: record.backend.clone(),
        os: record.os.clone(),
        image_ref: "chariox-slice-state:prior".into(),
        home_archive_path: prior_archive.display().to_string(),
        manifest_path: manifest.display().to_string(),
        created_at_ms: 1,
        updated_at_ms: 2,
        size_bytes: Some(23),
        last_operation: Some("state.save".into()),
        last_operation_status: Some(crate::slice::SliceOperationStatus::Completed),
        last_error: None,
    };
    super::super::state::write_state_manifest(&manifest, &prior).unwrap();
    let prior_manifest = std::fs::read(&manifest).unwrap();
    let started = Instant::now();
    let result = super::super::disk_admission::with_test_disk_admission_lock_path(
        &fixture.root.join("engine.lock"),
        || {
            with_test_progress_timeout(Duration::from_millis(70), || {
                super::super::capture_preflight::with_test_verified_layout(|| {
                    super::super::state::save_local_docker_slice_state_live(&record, &options)
                })
            })
        },
    );
    assert!(
        matches!(result, Err(DaemonError::LocalTransport { ref message, .. }) if message.contains("made no progress")),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(std::fs::read(&manifest).unwrap(), prior_manifest);
    assert_eq!(
        std::fs::read(&prior_archive).unwrap(),
        b"known-good-private-home"
    );
    assert_eq!(
        std::fs::read_dir(&state_dir).unwrap().count(),
        2,
        "no failed archive generation survives"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("status")).unwrap(),
        "true running\n"
    );
    assert!(!fixture
        .root
        .join("runtime")
        .join(&record.id)
        .join("snapshot-resume.json")
        .exists());
    let calls = std::fs::read_to_string(fixture.root.join("calls")).unwrap();
    let capture = calls.rfind("tar --zstd -C /home-src -cf - .").unwrap();
    let helper_removal = calls
        .rfind("rm -f chariox-slice-dev-home-archive-")
        .unwrap();
    let unpause = calls.rfind("unpause chariox-slice-dev").unwrap();
    let desktop_start = calls.rfind("slice-screen.sh start").unwrap();
    assert!(
        capture < helper_removal && helper_removal < unpause && unpause < desktop_start,
        "{calls}"
    );
    assert_eq!(progress_timeout(), Duration::from_millis(300_000));
}

#[test]
fn home_archive_stream_test_timeout_override_restores_after_panic() {
    let result = std::panic::catch_unwind(|| {
        with_test_progress_timeout(Duration::from_millis(1), || {
            assert_eq!(progress_timeout(), Duration::from_millis(1));
            panic!("synthetic scoped override failure");
        });
    });
    assert!(result.is_err());
    assert_eq!(progress_timeout(), Duration::from_millis(300_000));
}
