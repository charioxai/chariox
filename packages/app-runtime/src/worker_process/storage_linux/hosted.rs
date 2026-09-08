//! These ignored tests use the installed production helper on a dedicated host.
//! No disk image, mount or privileged helper is created by an ordinary test run.
use super::{client::Lease, model::Enrollment, CONFIG, DATA_BYTES, TMP_BYTES};
use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
    process::Command,
    time::Duration,
};

struct Context {
    cgroup: PathBuf,
    leaf: String,
}
impl Context {
    fn open(suffix: &str) -> Self {
        assert_eq!(std::env::var("GITHUB_ACTIONS").unwrap(), "true");
        assert_eq!(
            std::env::var("RUNNER_ENVIRONMENT").unwrap(),
            "github-hosted"
        );
        assert_eq!(
            std::env::var("GITHUB_REPOSITORY").unwrap(),
            "charioxai/chariox"
        );
        assert_eq!(
            std::env::var("CHARIOX_STORAGE_HOSTED").unwrap(),
            "fixed-production-helper"
        );
        let uid = unsafe { libc::geteuid() };
        assert_ne!(uid, 0);
        let config: Enrollment = serde_json::from_slice(&fs::read(CONFIG).unwrap()).unwrap();
        config.validate().unwrap();
        let root = &config
            .owners
            .iter()
            .find(|owner| owner.uid == uid)
            .unwrap()
            .cgroup_root;
        assert_eq!(root, super::provision::APPS);
        let leaf = format!("app-{suffix}");
        let cgroup = PathBuf::from(root).join(&leaf);
        fs::create_dir(&cgroup).unwrap();
        Self { cgroup, leaf }
    }
    fn lease(&self, installation: &str, generation: u64) -> Lease {
        Lease::acquire("hosted-owner", installation, generation, &self.leaf).unwrap()
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.cgroup);
    }
}
fn exhaust(path: &std::path::Path, limit: u64) -> u64 {
    let mut file = File::create(path).unwrap();
    let buffer = vec![0x35; 1024 * 1024];
    let mut written = 0;
    loop {
        match file.write(&buffer) {
            Ok(0) => panic!("empty successful storage write"),
            Ok(count) => {
                written += count as u64;
                assert!(written <= limit);
            }
            Err(error) => {
                assert_eq!(error.raw_os_error(), Some(libc::ENOSPC));
                break;
            }
        }
    }
    assert!(written > limit / 2 && written < limit);
    drop(file);
    fs::remove_file(path).unwrap();
    written
}
#[test]
#[ignore = "requires dedicated hosted Linux root helper and fixed ext4 volumes"]
fn hosted_private_capacity_persistence_tmp_reset_and_noexec() {
    let context = Context::open("11111111111111111111111111111111");
    let mut lease = context.lease("persistent", 1);
    let observations = lease.mount_observations();
    for (helper, kernel) in observations {
        assert_ne!(
            helper, kernel,
            "fixture must observe propagated private-namespace mounts"
        );
    }
    println!("Root helper / kernel mount IDs: {observations:?}; device/inode identities matched");
    let data = lease.data_path();
    let temporary = lease.temporary_path();
    fs::write(data.join("sentinel"), b"persistent data").unwrap();
    fs::write(temporary.join("sentinel"), b"temporary data").unwrap();
    fs::copy(data.join("sentinel"), data.join("copy")).unwrap();
    assert_eq!(fs::read(data.join("copy")).unwrap(), b"persistent data");
    let data_written = exhaust(&data.join("quota"), DATA_BYTES);
    let tmp_written = exhaust(&temporary.join("quota"), TMP_BYTES);
    fs::copy("/usr/bin/true", data.join("executable")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(data.join("executable"), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        Command::new(data.join("executable"))
            .status()
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EACCES)
    );
    lease.release().unwrap();
    let mut lease = context.lease("persistent", 2);
    assert_eq!(
        fs::read(lease.data_path().join("sentinel")).unwrap(),
        b"persistent data"
    );
    assert!(!lease.temporary_path().join("sentinel").exists());
    lease.release().unwrap();
    println!("private ext4 data ENOSPC at {data_written}/{DATA_BYTES}; tmp at {tmp_written}/{TMP_BYTES}; noexec and remount persistence passed");
}
#[test]
#[ignore = "requires dedicated hosted Linux root helper and owned process cancellation"]
fn hosted_crash_fixture_holds_lease_until_owner_is_killed() {
    let context = Context::open("22222222222222222222222222222222");
    let lease = context.lease("crash", 1);
    let mut file = File::create(lease.data_path().join("crash-sentinel")).unwrap();
    file.write_all(b"saved before abrupt helper failure")
        .unwrap();
    file.sync_all().unwrap();
    drop(file);
    let marker = std::env::var("CHARIOX_STORAGE_CRASH_MARKER").unwrap();
    assert_eq!(marker, "/var/lib/chariox/home/storage-crash-ready");
    fs::write(marker, "ready\n").unwrap();
    std::thread::sleep(Duration::from_secs(60));
    panic!("root fixture did not cancel its own held storage process");
}
#[test]
#[ignore = "requires dedicated hosted Linux helper restart after abrupt cancellation"]
fn hosted_recovery_preserves_data_after_abrupt_helper_exit() {
    let context = Context::open("33333333333333333333333333333333");
    let mut lease = context.lease("crash", 2);
    assert_eq!(
        fs::read(lease.data_path().join("crash-sentinel")).unwrap(),
        b"saved before abrupt helper failure"
    );
    lease.release().unwrap();
}
