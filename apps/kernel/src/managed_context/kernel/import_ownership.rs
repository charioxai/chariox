//! Private publication journal: rollback requires both inode identity and exact tree content.
use super::*;
use std::io::Read;

pub(super) fn entry_fingerprint(path: &Path) -> Result<((u64, u64), String), DaemonError> {
    let filesystem_root =
        crate::managed_context::development::directory::open_root(Path::new("/"))?;
    let file = crate::managed_context::development::directory::open_relative(
        &filesystem_root,
        path.strip_prefix("/")
            .map_err(|_| import_error("published entry path must be absolute"))?,
    )
    .map_err(|error| import_io_error("open published entry components", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| import_io_error("inspect published entry", error))?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    return Err(import_error(
        "published entry identity requires a Unix host",
    ));
    #[cfg(unix)]
    {
        let mut hash = Sha256::new();
        let mut bytes = 0_u64;
        let mut entries = 0_u64;
        hash_entry(&file, path, &mut hash, &mut bytes, &mut entries, 0)?;
        Ok((identity, format!("{:x}", hash.finalize())))
    }
}

#[cfg(unix)]
fn hash_entry(
    file: &File,
    path: &Path,
    hash: &mut Sha256,
    bytes: &mut u64,
    entries: &mut u64,
    depth: usize,
) -> Result<(), DaemonError> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if depth > 128 {
        return Err(import_error("published tree exceeds its depth limit"));
    }
    *entries += 1;
    if *entries > MAX_MATERIALIZED_CONTEXT_ENTRIES {
        return Err(import_error("published entry exceeds tree budget"));
    }
    let metadata = file
        .metadata()
        .map_err(|error| import_io_error("inspect published tree", error))?;
    // MP-11: identical bytes do not confer ownership on a replacement inode.
    hash.update(metadata.dev().to_le_bytes());
    hash.update(metadata.ino().to_le_bytes());
    hash.update(metadata.permissions().mode().to_le_bytes());
    if metadata.is_file() {
        hash.update(b"file");
        hash.update(metadata.len().to_le_bytes());
        let mut source = file
            .try_clone()
            .map_err(|error| import_io_error("clone published file", error))?;
        let mut buffer = [0; 65536];
        loop {
            let count = source
                .read(&mut buffer)
                .map_err(|error| import_io_error("hash published file", error))?;
            if count == 0 {
                break;
            }
            *bytes += count as u64;
            if *bytes > MAX_MATERIALIZED_CONTEXT_BYTES {
                return Err(import_error("published entry exceeds byte budget"));
            }
            hash.update(&buffer[..count]);
        }
    } else if metadata.is_dir() {
        hash.update(b"directory");
        let mut children = fs::read_dir(path)
            .and_then(|children| children.collect::<Result<Vec<_>, _>>())
            .map_err(|error| import_io_error("enumerate published tree", error))?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let name = child.file_name();
            let name_bytes = name.as_bytes();
            hash.update((name_bytes.len() as u64).to_le_bytes());
            hash.update(name_bytes);
            let name = std::ffi::CString::new(name_bytes)
                .map_err(|_| import_error("invalid published name"))?;
            let fd = unsafe {
                libc::openat(
                    file.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ELOOP) {
                    return Err(import_io_error("open published tree component", error));
                }
                let mut before: libc::stat = unsafe { std::mem::zeroed() };
                if unsafe {
                    libc::fstatat(
                        file.as_raw_fd(),
                        name.as_ptr(),
                        &mut before,
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                } != 0
                    || before.st_mode & libc::S_IFMT != libc::S_IFLNK
                {
                    return Err(import_error("published link changed during inspection"));
                }
                let mut target = [0_u8; 4097];
                let length = unsafe {
                    libc::readlinkat(
                        file.as_raw_fd(),
                        name.as_ptr(),
                        target.as_mut_ptr().cast(),
                        target.len(),
                    )
                };
                let mut after: libc::stat = unsafe { std::mem::zeroed() };
                if length < 0
                    || length as usize >= target.len()
                    || unsafe {
                        libc::fstatat(
                            file.as_raw_fd(),
                            name.as_ptr(),
                            &mut after,
                            libc::AT_SYMLINK_NOFOLLOW,
                        )
                    } != 0
                    || (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino)
                {
                    return Err(import_error("published link changed during inspection"));
                }
                *entries += 1;
                if *entries > MAX_MATERIALIZED_CONTEXT_ENTRIES {
                    return Err(import_error("published entry exceeds tree budget"));
                }
                hash.update(before.st_dev.to_le_bytes());
                hash.update(before.st_ino.to_le_bytes());
                hash.update(b"symlink");
                hash.update((length as u64).to_le_bytes());
                hash.update(&target[..length as usize]);
                continue;
            }
            let child_file = unsafe { File::from_raw_fd(fd) };
            hash_entry(&child_file, &child.path(), hash, bytes, entries, depth + 1)?;
        }
    } else {
        return Err(import_error("published tree contains a special file"));
    }
    let after = file
        .metadata()
        .map_err(|error| import_io_error("recheck published tree", error))?;
    let stamp = |value: &fs::Metadata| {
        (
            value.dev(),
            value.ino(),
            value.mode(),
            value.len(),
            value.mtime(),
            value.mtime_nsec(),
            value.ctime(),
            value.ctime_nsec(),
        )
    };
    if stamp(&metadata) != stamp(&after) {
        return Err(import_error(
            "published tree changed during ownership inspection",
        ));
    }
    Ok(())
}
