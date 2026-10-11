//! MP-08 / MP-10 / MP-11: bounded durable snapshots without a rename per Check update.
use super::environment_error;
use crate::error::DaemonError;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
const LIMIT: u64 = 8 * 1024 * 1024 + 1;

pub(super) fn read(path: &Path) -> Result<Option<Vec<u8>>, DaemonError> {
    let mut file = match open(path, false) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(environment_error("Check journal unavailable")),
    };
    latest(&mut file).map(|(bytes, _)| Some(bytes))
}

// Callers hold the Project lock. First publication and bounded compaction use
// the existing parent-relative atomic writer. An existing inode needs one data
// sync; its directory entry has already been persisted. Never acknowledge
// Requested or settlement until that sync completes.
pub(super) fn append(path: &Path, bytes: &[u8]) -> Result<(), DaemonError> {
    let mut record = Vec::with_capacity(bytes.len() + 1);
    record.extend_from_slice(bytes);
    record.push(b'\n');
    let mut file = match open(path, true) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return crate::config::write_private_file(path, &record)
                .map_err(|_| environment_error("Check journal write failed"));
        }
        Err(_) => return Err(environment_error("Check journal write failed")),
    };
    let (_, complete_end) = latest(&mut file)?;
    if complete_end + record.len() as u64 > LIMIT {
        return crate::config::write_private_file(path, &record)
            .map_err(|_| environment_error("Check journal write failed"));
    }
    // A crash can leave an incomplete suffix after the last durable snapshot.
    // Drop only that suffix; complete malformed records remain a read error.
    if file
        .metadata()
        .map_err(|_| environment_error("Check journal unavailable"))?
        .len()
        != complete_end
    {
        file.set_len(complete_end)
            .map_err(|_| environment_error("Check journal write failed"))?;
    }
    file.write_all(&record)
        .and_then(|_| file.sync_data())
        .map_err(|_| environment_error("Check journal write failed"))
}

fn latest(file: &mut File) -> Result<(Vec<u8>, u64), DaemonError> {
    let len = file
        .metadata()
        .map_err(|_| environment_error("Check journal unavailable"))?
        .len();
    if len == 0 || len > LIMIT {
        return Err(environment_error("Check journal exceeds bounds"));
    }
    let mut window = len.min(64 * 1024);
    loop {
        let offset = len - window;
        file.seek(SeekFrom::Start(offset))
            .map_err(|_| environment_error("Check journal unavailable"))?;
        let mut tail = Vec::with_capacity(window as usize);
        (&mut *file)
            .take(window)
            .read_to_end(&mut tail)
            .map_err(|_| environment_error("Check journal unavailable"))?;
        if let Some(end) = tail.iter().rposition(|byte| *byte == b'\n') {
            let start = tail[..end].iter().rposition(|byte| *byte == b'\n');
            if start.is_some() || offset == 0 {
                return Ok((
                    tail[start.map_or(0, |p| p + 1)..end].to_vec(),
                    offset + end as u64 + 1,
                ));
            }
        }
        if window == len {
            return Err(environment_error("invalid Check journal"));
        }
        window = (window * 2).min(len);
    }
}

fn open(path: &Path, write: bool) -> std::io::Result<File> {
    #[cfg(unix)]
    let file = {
        use std::ffi::CString;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::OpenOptionsExt;
        let parent = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path.parent().ok_or(std::io::ErrorKind::InvalidInput)?)?;
        let name = CString::new(
            path.file_name()
                .ok_or(std::io::ErrorKind::InvalidInput)?
                .as_bytes(),
        )
        .map_err(|_| std::io::ErrorKind::InvalidInput)?;
        let flags = if write {
            libc::O_RDWR | libc::O_APPEND
        } else {
            libc::O_RDONLY
        };
        // SAFETY: owned parent descriptor and NUL-terminated basename remain live.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: openat returned a new descriptor whose ownership passes to File.
        unsafe { File::from_raw_fd(fd) }
    };
    #[cfg(not(unix))]
    let file = std::fs::OpenOptions::new()
        .read(true)
        .append(write)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Append must never mutate a linked external inode or weaken private mode.
        if metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(file)
}
