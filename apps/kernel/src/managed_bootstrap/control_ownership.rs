//! MP-03/MP-07: protect legacy control state after the source updater has settled.
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use fs2::FileExt;

use crate::config::write_private_file;
use crate::error::DaemonError;

use super::control_state::{error, protected, validate_parent, MAX_BYTES};

const MANAGED_NAMES: &[&str] = &[
    "bootstrap-receipt.json",
    "bootstrap-grant-binding.json",
    "release-override.json",
];
const WORKER_NAMES: &[&str] = &["bootstrap-receipt.json", "release-override.json"];

fn control_names(parent: &Path) -> &'static [&'static str] {
    if parent.file_name().and_then(|name| name.to_str()) == Some("disposable-worker") {
        WORKER_NAMES
    } else {
        MANAGED_NAMES
    }
}

pub(super) fn io_lock() -> Result<File, DaemonError> {
    let file = lock_file(Path::new("/run/chariox-managed-control.lock"), true)?;
    file.lock_exclusive()
        .map_err(|_| error("control lock failed"))?;
    Ok(file)
}

fn lock_file(path: &Path, private: bool) -> Result<File, DaemonError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| error("control lock could not open"))?;
    let metadata = file
        .metadata()
        .map_err(|_| error("control lock could not be inspected"))?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.gid() != 0
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
        || (private && metadata.mode() & 0o777 != 0o600)
    {
        return Err(error("control lock is unsafe"));
    }
    Ok(file)
}

fn legacy_identity() -> Result<(u32, u32), DaemonError> {
    // getpwnam_r avoids the shared static buffer of getpwnam in a multithreaded kernel.
    let name = CString::new("chariox").unwrap();
    let mut entry = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0u8; 16 * 1024];
    let status = unsafe {
        libc::getpwnam_r(
            name.as_ptr(),
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status == 0 && !result.is_null() {
        return Ok((entry.pw_uid, entry.pw_gid));
    }
    #[cfg(test)]
    {
        let name = CString::new("nobody").unwrap();
        let status = unsafe {
            libc::getpwnam_r(
                name.as_ptr(),
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == 0 && !result.is_null() {
            return Ok((entry.pw_uid, entry.pw_gid));
        }
    }
    Err(error("managed service principal is unavailable"))
}

fn directory(path: &Path, identity: (u32, u32)) -> Result<File, DaemonError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| error("control directory could not open"))?;
    let m = file
        .metadata()
        .map_err(|_| error("control directory could not be inspected"))?;
    if !m.is_dir()
        || m.mode() & 0o022 != 0
        || !((m.uid(), m.gid()) == (0, 0) || (m.uid(), m.gid()) == identity)
    {
        return Err(error("legacy control directory is unsafe"));
    }
    Ok(file)
}

fn seal(file: &File, mode: u32) -> Result<(), DaemonError> {
    if unsafe { libc::fchown(file.as_raw_fd(), 0, 0) } != 0
        || unsafe { libc::fchmod(file.as_raw_fd(), mode) } != 0
    {
        return Err(error("control directory protection failed"));
    }
    Ok(())
}

/// Caller holds the control I/O lock. A source updater still holding its install
/// lock or recovery journal must retain legacy access, even after phase=committed.
pub(super) fn prepare(path: &Path) -> Result<bool, DaemonError> {
    let parent = path
        .parent()
        .ok_or_else(|| error("missing control parent"))?;
    let state_root = parent
        .parent()
        .ok_or_else(|| error("missing control root"))?;
    let parent_metadata =
        fs::symlink_metadata(parent).map_err(|_| error("control parent missing"))?;
    if parent_metadata.is_dir()
        && parent_metadata.uid() == 0
        && parent_metadata.gid() == 0
        && parent_metadata.mode() & 0o777 == 0o700
    {
        // A worker v1 StateDirectory can also change an ancestor or files.
        if validate_parent(path).is_ok()
            && control_names(parent).iter().all(|name| {
                match fs::symlink_metadata(parent.join(name)) {
                    Ok(m) => {
                        m.is_file()
                            && m.uid() == 0
                            && m.gid() == 0
                            && m.mode() & 0o777 == 0o600
                            && m.nlink() == 1
                            && m.len() <= MAX_BYTES
                    }
                    Err(e) => e.kind() == std::io::ErrorKind::NotFound,
                }
            })
        {
            return Ok(true);
        }
    }
    // Root-controlled ancestors pin both mutable directories against namespace moves.
    validate_parent(state_root)?;
    let identity = legacy_identity()?;
    if identity.0 == 0 {
        return Err(error("legacy service principal must be unprivileged"));
    }
    let root = directory(state_root, identity)?;
    let parent_file = directory(parent, identity)?;
    let upgrade_path = if protected(path) {
        std::path::PathBuf::from("/run/lock/chariox-managed-image-install.lock")
    } else {
        // Only internal tests use arbitrary paths; the public helper fences exact paths.
        state_root.join(".upgrade.lock")
    };
    let upgrade_lock = lock_file(&upgrade_path, false)?;
    match upgrade_lock.try_lock_exclusive() {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
        Err(_) => return Err(error("upgrade lock failed")),
    }
    let journal = if protected(path) {
        std::path::PathBuf::from("/usr/lib/chariox/.managed-kernel-upgrade")
    } else {
        state_root.join(".managed-kernel-upgrade")
    };
    if protected(path) {
        validate_parent(&journal)?;
    }
    match fs::symlink_metadata(&journal) {
        Ok(m) => {
            if !m.is_dir() || m.uid() != 0 || m.gid() != 0 || m.mode() & 0o077 != 0 {
                return Err(error("upgrade recovery journal is unsafe"));
            }
            return Ok(false);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(error("upgrade recovery journal could not be inspected")),
    }
    // Pin and preflight every allowed entry before changing any ownership.
    let mut entries = Vec::new();
    for name in control_names(parent) {
        let entry = parent.join(name);
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&entry)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(error("legacy control file could not open")),
        };
        let m = file
            .metadata()
            .map_err(|_| error("legacy control file could not be inspected"))?;
        if !m.is_file()
            || m.nlink() != 1
            || m.mode() & 0o777 != 0o600
            || m.len() > MAX_BYTES
            || !((m.uid(), m.gid()) == (0, 0) || (m.uid(), m.gid()) == identity)
        {
            return Err(error("legacy control file is unsafe"));
        }
        entries.push((entry, file, m));
    }
    seal(&root, 0o755)?;
    let admitted = parent_file
        .metadata()
        .map_err(|_| error("control parent inspection failed"))?;
    let live = fs::symlink_metadata(parent).map_err(|_| error("control parent disappeared"))?;
    if live.dev() != admitted.dev() || live.ino() != admitted.ino() {
        return Err(error("control parent changed during protection"));
    }
    seal(&parent_file, 0o700)?;
    // Once sealed, path lookups cannot be replaced by the service principal.
    for (entry, mut file, original) in entries {
        let live = fs::symlink_metadata(&entry).map_err(|_| error("control file disappeared"))?;
        if live.dev() != original.dev() || live.ino() != original.ino() {
            return Err(error("control file changed during protection"));
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error("legacy control read failed"))?;
        if bytes.len() as u64 > MAX_BYTES
            || serde_json::from_slice::<serde_json::Value>(&bytes).is_err()
        {
            return Err(error("legacy control is not bounded JSON"));
        }
        // Replacement revokes writes through any pre-opened legacy descriptor.
        write_private_file(&entry, &bytes).map_err(|_| error("control replacement failed"))?;
    }
    parent_file
        .sync_all()
        .map_err(|_| error("control directory sync failed"))?;
    root.sync_all()
        .map_err(|_| error("control root sync failed"))?;
    validate_parent(path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};
    use std::os::unix::fs::{symlink, PermissionsExt};

    struct Fixture {
        root: std::path::PathBuf,
        uid: u32,
        gid: u32,
    }
    impl Fixture {
        fn new() -> Self {
            let (uid, gid) = legacy_identity().unwrap();
            let root = std::env::temp_dir().join(format!(
                "chariox-control-transition-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            fs::create_dir_all(root.join("managed")).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
            let fixture = Self { root, uid, gid };
            write_private_file(&fixture.receipt(), b"{}").unwrap();
            fixture.own(&fixture.root.join("managed"));
            fixture.own(&fixture.receipt());
            fs::set_permissions(
                fixture.root.join("managed"),
                fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            fixture
        }
        fn receipt(&self) -> std::path::PathBuf {
            self.root.join("managed/bootstrap-receipt.json")
        }
        fn own(&self, path: &Path) {
            let name = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::chown(name.as_ptr(), self.uid, self.gid) }, 0);
        }
        fn journal(&self) -> std::path::PathBuf {
            self.root.join(".managed-kernel-upgrade")
        }
        fn assert_legacy(&self) {
            assert_eq!(
                fs::metadata(self.root.join("managed")).unwrap().uid(),
                self.uid
            );
            assert_eq!(fs::metadata(self.receipt()).unwrap().uid(), self.uid);
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn legacy_control_transition_defers_while_updater_lock_or_journal_exists() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let f = Fixture::new();
        let lock = lock_file(&f.root.join(".upgrade.lock"), false).unwrap();
        lock.lock_exclusive().unwrap();
        assert!(!prepare(&f.receipt()).unwrap());
        f.assert_legacy();
        FileExt::unlock(&lock).unwrap();
        fs::create_dir(&f.journal()).unwrap();
        fs::set_permissions(f.journal(), fs::Permissions::from_mode(0o700)).unwrap();
        // A committed phase still has fallible publication/tombstoning work.
        fs::write(f.journal().join("phase"), b"committed").unwrap();
        assert!(!prepare(&f.receipt()).unwrap());
        f.assert_legacy();
        fs::remove_dir_all(f.journal()).unwrap();
        assert!(prepare(&f.receipt()).unwrap());
        assert_eq!(fs::metadata(f.receipt()).unwrap().uid(), 0);
    }

    #[test]
    fn legacy_control_transition_replaces_preopened_writable_inode() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let f = Fixture::new();
        let mut retained = OpenOptions::new().write(true).open(f.receipt()).unwrap();
        let old_inode = retained.metadata().unwrap().ino();
        assert!(prepare(&f.receipt()).unwrap());
        assert_ne!(fs::metadata(f.receipt()).unwrap().ino(), old_inode);
        retained.seek(SeekFrom::Start(0)).unwrap();
        retained.write_all(b"corrupt").unwrap();
        assert_eq!(fs::read(f.receipt()).unwrap(), b"{}");
    }

    #[test]
    fn legacy_control_transition_protects_worker_state_ancestor_and_preserves_workspace() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let f = Fixture::new();
        f.own(&f.root);
        fs::set_permissions(&f.root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(f.root.join("workspace")).unwrap();
        f.own(&f.root.join("workspace"));
        assert!(prepare(&f.receipt()).unwrap());
        assert_eq!(fs::metadata(&f.root).unwrap().uid(), 0);
        assert_eq!(fs::metadata(&f.root).unwrap().mode() & 0o777, 0o755);
        assert_eq!(fs::metadata(f.root.join("workspace")).unwrap().uid(), f.uid);
    }

    #[test]
    fn legacy_control_transition_promotes_all_managed_controls() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let f = Fixture::new();
        let parent = f.root.join("managed");
        for name in ["bootstrap-grant-binding.json", "release-override.json"] {
            let path = parent.join(name);
            write_private_file(&path, b"{}").unwrap();
            f.own(&path);
        }
        assert!(prepare(&f.receipt()).unwrap());
        for name in MANAGED_NAMES {
            let m = fs::metadata(parent.join(name)).unwrap();
            assert_eq!((m.uid(), m.gid(), m.mode() & 0o777), (0, 0, 0o600));
        }
        assert!(
            prepare(&f.receipt()).unwrap(),
            "private transition is idempotent"
        );
    }

    #[test]
    fn legacy_control_transition_rejects_aliases_before_ownership_changes() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        for kind in ["symlink", "hardlink", "mode", "owner", "journal"] {
            let f = Fixture::new();
            match kind {
                "symlink" => {
                    fs::remove_file(f.receipt()).unwrap();
                    symlink(f.root.join("outside"), f.receipt()).unwrap();
                }
                "hardlink" => fs::hard_link(f.receipt(), f.root.join("alias")).unwrap(),
                "mode" => {
                    fs::set_permissions(f.receipt(), fs::Permissions::from_mode(0o640)).unwrap()
                }
                "owner" => {
                    let name = CString::new(f.receipt().as_os_str().as_encoded_bytes()).unwrap();
                    assert_eq!(unsafe { libc::chown(name.as_ptr(), f.uid + 1, f.gid) }, 0);
                }
                "journal" => symlink(f.root.join("outside"), f.journal()).unwrap(),
                _ => unreachable!(),
            }
            assert!(prepare(&f.receipt()).is_err(), "reject {kind}");
            assert_eq!(fs::metadata(f.root.join("managed")).unwrap().uid(), f.uid);
        }
    }
}
