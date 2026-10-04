//! Both resource reservations use the same host-wide lock names as older kernels.
//! Provisioning changes permissions in place; it never swaps a live lock inode.
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) fn host_path(resource: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/chariox-docker-{resource}-admission.lock"))
}

pub(super) fn path(resource: &str) -> PathBuf {
    #[cfg(not(test))]
    {
        host_path(resource)
    }
    #[cfg(test)]
    {
        test_directory().join(format!("{resource}.lock"))
    }
}

pub(super) fn open(path: &Path) -> io::Result<File> {
    #[cfg(not(test))]
    let owner = 0;
    #[cfg(test)]
    let owner = if path.parent() == Some(test_directory().as_path()) {
        unsafe { libc::geteuid() }
    } else {
        0
    };
    open_for_owner(path, owner)
}

#[cfg(test)]
struct TestLocks {
    directory: PathBuf,
    owned: bool,
}
#[cfg(test)]
static TEST_LOCKS: std::sync::OnceLock<TestLocks> = std::sync::OnceLock::new();

#[cfg(test)]
pub(super) fn test_directory() -> &'static PathBuf {
    &TEST_LOCKS
        .get_or_init(|| {
            use std::os::unix::fs::PermissionsExt;
            let inherited = std::env::var_os("CHARIOX_TEST_DOCKER_ADMISSION_LOCK_DIR");
            let owned = inherited.is_none();
            let directory = inherited.map(PathBuf::from).unwrap_or_else(|| {
                std::env::temp_dir().join(format!(
                    "chariox-admission-tests-{}-{:032x}",
                    std::process::id(),
                    rand::random::<u128>()
                ))
            });
            if owned {
                std::fs::create_dir(&directory).expect("owned admission fixture directory");
                std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
                for resource in ["memory", "disk"] {
                    let file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o444)
                        .open(directory.join(format!("{resource}.lock")))
                        .unwrap();
                    drop(file);
                }
            }
            for resource in ["memory", "disk"] {
                open_for_owner(&directory.join(format!("{resource}.lock")), unsafe {
                    libc::geteuid()
                })
                .expect("fixture lock owner and inode must be valid");
            }
            if owned {
                unsafe {
                    libc::atexit(cleanup_test_locks);
                }
            }
            TestLocks { directory, owned }
        })
        .directory
}

#[cfg(test)]
extern "C" fn cleanup_test_locks() {
    if let Some(locks) = TEST_LOCKS.get() {
        if locks.owned {
            for resource in ["memory", "disk"] {
                let _ = std::fs::remove_file(locks.directory.join(format!("{resource}.lock")));
            }
            let _ = std::fs::remove_dir(&locks.directory);
        }
    }
}

// The macOS pkg builder requires this contract in the release kernel before
// installing root-owned 0444 locks. Keep it in the runtime diagnostic so it
// survives release linking without a second capability/protocol path.
const PROVISIONED_LOCK_CONTRACT: &str = "chariox.docker-admission-locks.read-only.v1";

fn provisioning_help(macos: bool) -> &'static str {
    if macos {
        "on macOS, install the Chariox pkg or run sudo /usr/bin/python3 deploy/local-macos/install-docker-admission-locks.py from a checkout to install the boot LaunchDaemon; for an installed pkg, run sudo /usr/bin/python3 /usr/local/libexec/chariox/provision-docker-admission-locks.py"
    } else {
        "provision the host-wide locks as root with deploy/local-linux/provision-docker-admission-locks.py"
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!(
        "{message} [{PROVISIONED_LOCK_CONTRACT}]; {}; if a legacy lock is unsafe, stop all kernels sharing Docker before administrator repair; never replace a live lock",
        provisioning_help(cfg!(target_os = "macos"))
    ))
}

fn open_for_owner(path: &Path, owner: u32) -> io::Result<File> {
    // /tmp is a root-owned symlink to /private/tmp on macOS. Anchor operations
    // to its validated directory descriptor rather than following a leaf link.
    let parent = path
        .parent()
        .ok_or_else(|| invalid("admission lock has no parent"))?;
    let parent = std::fs::canonicalize(parent)
        .map_err(|error| invalid(&format!("failed to resolve admission lock parent: {error}")))?;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(parent)
        .map_err(|error| invalid(&format!("failed to open admission lock parent: {error}")))?;
    let metadata = directory.metadata()?;
    let writable = metadata.mode() & 0o022 != 0;
    if metadata.uid() != owner || (writable && metadata.mode() & 0o1000 == 0) {
        return Err(invalid(
            "admission lock parent must be root-owned and sticky when writable",
        ));
    }
    let name = path
        .file_name()
        .ok_or_else(|| invalid("admission lock has no filename"))?;
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(name.as_bytes())
        .map_err(|_| invalid("invalid admission lock filename"))?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(invalid(&format!(
            "failed to open admission lock: {}",
            io::Error::last_os_error()
        )));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != owner
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
        || metadata.len() != 0
    {
        return Err(invalid("admission lock must be an empty root-owned regular file with one link and no group/other writes"));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs2::FileExt;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn provisioning_help_names_macos_boot_setup_and_installed_repair() {
        let help = provisioning_help(true);
        assert!(help.contains("deploy/local-macos/install-docker-admission-locks.py"));
        assert!(help.contains("/usr/local/libexec/chariox/provision-docker-admission-locks.py"));
        assert!(help.contains("LaunchDaemon"));
        assert!(provisioning_help(false).contains("deploy/local-linux/"));
        assert!(invalid("unsafe lock")
            .to_string()
            .contains("stop all kernels"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn provisioned_root_lock_uses_kernel_opener_as_ordinary_uid() {
        use std::os::unix::process::CommandExt;
        if let Some(path) = std::env::var_os("CHARIOX_ADMISSION_TEST_LOCK") {
            let lock = open_for_owner(Path::new(&path), 0).unwrap();
            assert_eq!(unsafe { libc::geteuid() }, 65534);
            let flags = unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_GETFL) };
            assert_eq!(flags & libc::O_ACCMODE, libc::O_RDONLY);
            assert!(std::fs::remove_file(Path::new(&path)).is_err());
            if std::env::var_os("CHARIOX_ADMISSION_TEST_HELD").is_some() {
                assert!(lock.try_lock_exclusive().is_err());
            } else {
                lock.try_lock_exclusive().unwrap();
            }
            return;
        }
        if unsafe { libc::geteuid() } != 0 {
            return; // Cross-UID proof runs on the disposable root builder.
        }
        // The ordinary UID must not depend on a private TMPDIR or checkout.
        let fixture = Fixture::new_in(Path::new("/tmp"));
        let executable = fixture.0.join("worker-test");
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let provisioned = std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../deploy/local-linux/provision-docker-admission-locks.py"
            ))
            .arg("--root")
            .arg(&fixture.0)
            .output()
            .unwrap();
        assert!(
            provisioned.status.success(),
            "{}",
            String::from_utf8_lossy(&provisioned.stderr)
        );
        let path = fixture.0.join("tmp/chariox-docker-memory-admission.lock");
        let held = open_for_owner(&path, 0).unwrap();
        held.lock_exclusive().unwrap();
        let probe = |blocked| {
            let mut command = std::process::Command::new(&executable);
            command
                .args(["--exact", "slice::local_docker::admission_lock::tests::provisioned_root_lock_uses_kernel_opener_as_ordinary_uid"])
                .uid(65534)
                .gid(65534)
                .env("CHARIOX_ADMISSION_TEST_LOCK", &path)
                .env_remove("CHARIOX_ADMISSION_TEST_HELD");
            if blocked {
                command.env("CHARIOX_ADMISSION_TEST_HELD", "1");
            }
            let result = command.output().unwrap();
            assert!(
                result.status.success(),
                "ordinary UID probe failed ({}):\n{}\n{}",
                result.status,
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        };
        probe(true);
        drop(held);
        probe(false);
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self::new_in(&std::env::temp_dir())
        }
        fn new_in(parent: &Path) -> Self {
            let p = parent.join(format!("chariox-admission-{:032x}", rand::random::<u128>()));
            std::fs::create_dir(&p).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self(p)
        }
        fn lock(&self) -> PathBuf {
            let p = self.0.join("memory.lock");
            File::create(&p).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
            p
        }
        fn open(&self, p: &Path) -> io::Result<File> {
            open_for_owner(p, unsafe { libc::geteuid() })
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn test_resource_locks_are_owned_disposable_and_not_host_paths() {
        for resource in ["memory", "disk"] {
            let fixture = path(resource);
            assert_ne!(fixture, host_path(resource));
            let file = open(&fixture).unwrap();
            assert_eq!(file.metadata().unwrap().uid(), unsafe { libc::geteuid() });
            assert_eq!(file.metadata().unwrap().nlink(), 1);
        }
    }

    #[test]
    fn read_only_lock_contends_on_same_inode_and_recovers() {
        let f = Fixture::new();
        let p = f.lock();
        let first = f.open(&p).unwrap();
        first.lock_exclusive().unwrap();
        let second = f.open(&p).unwrap();
        assert_eq!(
            first.metadata().unwrap().ino(),
            second.metadata().unwrap().ino()
        );
        assert!(second.try_lock_exclusive().is_err());
        let flags = unsafe { libc::fcntl(second.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(flags & libc::O_ACCMODE, libc::O_RDONLY);
        drop(first);
        second.try_lock_exclusive().unwrap();
    }

    #[test]
    fn rejects_replaceable_parent_and_symlink_or_hardlink_leaf() {
        let f = Fixture::new();
        let p = f.lock();
        std::fs::set_permissions(&f.0, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(f.open(&p).is_err());
        std::fs::set_permissions(&f.0, std::fs::Permissions::from_mode(0o1777)).unwrap();
        assert!(f.open(&p).is_ok());
        let link = f.0.join("link");
        symlink(&p, &link).unwrap();
        assert!(f.open(&link).is_err());
        std::fs::remove_file(link).unwrap();
        let link = f.0.join("hardlink");
        std::fs::hard_link(&p, &link).unwrap();
        assert!(f.open(&p).is_err());
    }

    #[test]
    fn rejects_foreign_owner_writable_or_nonempty_leaf() {
        let f = Fixture::new();
        let p = f.lock();
        assert!(open_for_owner(&p, unsafe { libc::geteuid() }.wrapping_add(1)).is_err());
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(f.open(&p).is_err());
        std::fs::write(&p, b"unexpected").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(f.open(&p).is_err());
    }
}
