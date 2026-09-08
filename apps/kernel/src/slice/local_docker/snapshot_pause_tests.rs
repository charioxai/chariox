use super::*;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

struct Fixture {
    record: SliceRecord,
    options: LocalDockerSliceOptions,
    old_path: Option<std::ffi::OsString>,
}

impl Fixture {
    fn new() -> Self {
        let record = super::super::tests::test_record();
        let mut options = super::super::tests::test_options();
        options.root = std::env::temp_dir().join(format!(
            "chariox-snapshot-pause-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&options.root).unwrap();
        let docker = options.root.join("docker");
        std::fs::write(&docker, r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$1" in
  inspect) test ! -f "$root/inspect-fails" || exit 1; cat "$root/status" ;;
  pause) printf 'true paused\n' > "$root/status" ;;
  unpause) test ! -f "$root/unpause-fails" || exit 1; printf 'true running\n' > "$root/status" ;;
  exec) test ! -f "$root/desktop-fails" || exit 1 ;;
  ps) test ! -f "$root/list-fails" || exit 1; test ! -f "$root/present" || printf 'chariox-slice-dev\n' ;;
  *) exit 2 ;;
esac
exit 0
"#).unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700)).unwrap();
        let old_path = std::env::var_os("PATH");
        let mut paths = vec![options.root.clone()];
        if let Some(path) = &old_path {
            paths.extend(std::env::split_paths(path));
        }
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        let fixture = Self {
            record,
            options,
            old_path,
        };
        fixture.status("true paused");
        fixture
    }

    fn status(&self, status: &str) {
        std::fs::write(self.options.root.join("status"), status).unwrap();
    }
    fn flag(&self, name: &str) {
        std::fs::write(self.options.root.join(name), "").unwrap();
    }
    fn clear(&self, name: &str) {
        std::fs::remove_file(self.options.root.join(name)).unwrap();
    }
    fn journal(&self) -> PathBuf {
        path(&self.record, &self.options).unwrap()
    }
    fn calls(&self) -> String {
        std::fs::read_to_string(self.options.root.join("calls")).unwrap_or_default()
    }
    fn begin(&self) {
        begin(&self.record, &self.options).unwrap();
    }
    fn recover(&self) -> Result<(), DaemonError> {
        recover(&self.record, &self.options)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        match &self.old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
        let _ = std::fs::remove_dir_all(&self.options.root);
    }
}

#[test]
fn snapshot_resume_unpauses_then_restarts_desktop_and_retires_record() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new();
    f.begin();
    assert!(f.journal().exists());
    assert!(begin(&f.record, &f.options).is_err());
    f.recover().unwrap();
    assert!(!f.journal().exists());
    let calls = f.calls();
    assert!(calls.find("unpause ").unwrap() < calls.find("exec ").unwrap());
    f.recover().unwrap();
    assert_eq!(f.calls(), calls, "retired recovery is a no-op");
}

#[test]
fn snapshot_resume_retains_record_until_unpause_and_desktop_both_succeed() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new();
    f.begin();
    f.flag("unpause-fails");
    assert!(f.recover().is_err());
    assert!(f.journal().exists());
    assert!(!f.calls().contains("exec "));
    f.clear("unpause-fails");
    f.flag("desktop-fails");
    assert!(f.recover().is_err());
    assert!(f.journal().exists());
    f.clear("desktop-fails");
    let unpauses = f.calls().matches("unpause ").count();
    f.recover().unwrap();
    assert_eq!(f.calls().matches("unpause ").count(), unpauses);
    assert!(!f.journal().exists());
}

#[test]
fn snapshot_resume_handles_crash_before_pause_without_unpausing_running_container() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new();
    f.status("true running");
    f.begin();
    f.recover().unwrap();
    assert!(!f.calls().contains("unpause "));
    assert!(f.calls().contains("exec "));
}

#[test]
fn snapshot_resume_never_starts_a_stopped_container_or_an_unowned_pause() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new();
    f.recover().unwrap();
    assert!(f.calls().is_empty());
    f.begin();
    f.status("false exited");
    f.recover().unwrap();
    assert!(!f.calls().contains("exec "));
    assert!(!f.calls().contains("unpause "));
    assert!(!f.journal().exists());
}

#[test]
fn snapshot_resume_distinguishes_missing_container_from_daemon_failure() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new();
    f.begin();
    f.flag("inspect-fails");
    f.flag("list-fails");
    assert!(f.recover().is_err());
    assert!(f.journal().exists());
    f.clear("list-fails");
    f.flag("present");
    assert!(f.recover().is_err());
    assert!(f.journal().exists());
    f.clear("present");
    f.recover().unwrap();
    assert!(!f.journal().exists());
}

#[test]
fn snapshot_resume_rejects_corrupt_wrong_identity_and_symlink_records() {
    let _lock = crate::env_lock::lock();
    let f = Fixture::new();
    f.begin();
    let original = std::fs::read(f.journal()).unwrap();
    for payload in [b"{".to_vec(), vec![b'x'; 4097], {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value["container"] = "chariox-slice-other".into();
        serde_json::to_vec(&value).unwrap()
    }] {
        std::fs::write(f.journal(), payload).unwrap();
        assert!(f.recover().is_err());
        assert!(f.journal().exists());
        assert!(f.calls().is_empty());
    }
    std::fs::remove_file(f.journal()).unwrap();
    std::fs::write(f.options.root.join("elsewhere"), original).unwrap();
    std::os::unix::fs::symlink(f.options.root.join("elsewhere"), f.journal()).unwrap();
    assert!(f.recover().is_err());
    assert!(f.calls().is_empty());
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn snapshot_resume_survives_sigkill_between_pause_and_resume() {
    const CHILD_ROOT: &str = "CHARIOX_SNAPSHOT_PAUSE_TEST_CHILD";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let mut options = super::super::tests::test_options();
        options.root = root.into();
        let mut record = super::super::tests::test_record();
        record.id = std::fs::read_to_string(options.root.join("slice-id")).unwrap();
        begin(&record, &options).unwrap();
        assert!(docker_command()
            .args(["pause", &local_docker_container_name(&record)])
            .status()
            .unwrap()
            .success());
        std::fs::write(options.root.join("paused-ready"), "").unwrap();
        loop {
            std::thread::park();
        }
    }
    let _lock = crate::env_lock::lock();
    let f = Fixture::new();
    f.status("true running");
    std::fs::write(f.options.root.join("slice-id"), &f.record.id).unwrap();
    let name = "slice::local_docker::snapshot_pause::tests::snapshot_resume_survives_sigkill_between_pause_and_resume";
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD_ROOT, &f.options.root)
            .spawn()
            .unwrap(),
    );
    let started = Instant::now();
    while !f.options.root.join("paused-ready").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "child failed to reach pause"
        );
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before pause"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    assert!(f.journal().exists());
    f.recover().unwrap();
    assert_eq!(
        std::fs::read_to_string(f.options.root.join("status"))
            .unwrap()
            .trim(),
        "true running"
    );
    assert!(f.calls().contains("exec "));
    assert!(!f.journal().exists());
}
