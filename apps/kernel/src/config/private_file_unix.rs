//! Parent-relative private publication; paths are never reopened after admission.
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub(super) fn write(path: &Path, parent: &Path, contents: &[u8]) -> io::Result<()> {
    write_before_publish(path, parent, contents, || Ok(()))
}

fn write_before_publish(
    path: &Path,
    parent: &Path,
    contents: &[u8],
    before_publish: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let parent = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(parent)?;
    let destination = CString::new(
        path.file_name()
            .ok_or(io::ErrorKind::InvalidInput)?
            .as_bytes(),
    )
    .map_err(|_| io::ErrorKind::InvalidInput)?;
    let temporary = CString::new(format!(
        ".chariox-private-{:032x}.tmp",
        rand::random::<u128>()
    ))
    .unwrap();
    // SAFETY: the parent descriptor and NUL-terminated names remain owned throughout.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            temporary.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let identity = file.metadata()?;
    let result = (|| {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(contents)?;
        file.sync_all()?;
        before_publish()?;
        if unsafe {
            libc::renameat(
                parent.as_raw_fd(),
                temporary.as_ptr(),
                parent.as_raw_fd(),
                destination.as_ptr(),
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        parent.sync_all()
    })();
    // MP-11: stat device/inode typedef widths differ across Unix targets.
    #[allow(clippy::unnecessary_cast)]
    if result.is_err() {
        let mut current: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                temporary.as_ptr(),
                &mut current,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
            && current.st_dev as u64 == identity.dev()
            && current.st_ino as u64 == identity.ino()
        {
            unsafe {
                libc::unlinkat(parent.as_raw_fd(), temporary.as_ptr(), 0);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp11_interrupted_config_publication_preserves_deny_and_always_security_policy() {
        let root = std::env::temp_dir().join(format!(
            "mp11-config-interrupted-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("config.toml");
        let mut config = crate::config::CharioxUserConfig::default();
        config.credential_vault.unlock_policy = crate::config::CredentialVaultUnlockPolicy::Always;
        config.credential_vault.agent_management =
            crate::config::CredentialVaultAgentManagementPolicy::Deny;
        let original = toml::to_string(&config).unwrap();
        write(&path, &root, original.as_bytes()).unwrap();
        let defaults = toml::to_string(&crate::config::CharioxUserConfig::default()).unwrap();
        assert!(
            write_before_publish(&path, &root, defaults.as_bytes(), || Err(
                io::ErrorKind::Interrupted.into()
            ))
            .is_err()
        );
        let loaded = crate::config::load_user_config_from_path(&path);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let entries = std::fs::read_dir(&root).unwrap().count();
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(
            loaded.credential_vault.unlock_policy,
            crate::config::CredentialVaultUnlockPolicy::Always
        );
        assert_eq!(
            loaded.credential_vault.agent_management,
            crate::config::CredentialVaultAgentManagementPolicy::Deny
        );
        assert_eq!(mode, 0o600);
        assert_eq!(entries, 1);
    }
}
