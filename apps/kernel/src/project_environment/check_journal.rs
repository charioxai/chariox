//! MP-08 / MP-10 / MP-11: alternating durable snapshots on a bounded preallocated inode.
use super::environment_error;
use crate::error::DaemonError;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
const LIMIT: u64 = 8 * 1024 * 1024;
const PAGE: u64 = 4096;
const MAGIC: &[u8; 8] = b"CXCHECK2";

struct Snapshot {
    sequence: u64,
    slot: u64,
    bytes: Vec<u8>,
}

pub(super) fn read(path: &Path) -> Result<Option<Vec<u8>>, DaemonError> {
    let mut file = match open(path, false) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(environment_error("Check journal unavailable")),
    };
    match capacity(&mut file)? {
        Some(capacity) => Ok(Some(latest(&mut file, capacity)?.bytes)),
        None => legacy_latest(&mut file).map(|(bytes, _)| Some(bytes)),
    }
}

// The Project lock serializes writers. Both slots have allocated space before
// first publication. Updating the inactive slot needs one data sync and no
// inode growth or directory mutation. A torn write fails its checksum and
// leaves the previous slot recoverable. Acknowledgement follows the data sync.
pub(super) fn append(path: &Path, bytes: &[u8]) -> Result<(), DaemonError> {
    if bytes.len() as u64 > LIMIT {
        return Err(environment_error("Check journal exceeds bounds"));
    }
    let mut file = match open(path, true) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return publish(path, bytes, 1),
        Err(_) => return Err(environment_error("Check journal write failed")),
    };
    let Some(capacity) = capacity(&mut file)? else {
        legacy_latest(&mut file)?;
        return publish(path, bytes, 1);
    };
    let prior = latest(&mut file, capacity)?;
    let sequence = prior
        .sequence
        .checked_add(1)
        .ok_or_else(|| environment_error("Check journal sequence exhausted"))?;
    if bytes.len() as u64 > capacity {
        return publish(path, bytes, sequence);
    }
    let offset = PAGE + (1 - prior.slot) * (PAGE + capacity);
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.write_all(&record(bytes, sequence)))
        .and_then(|_| file.sync_data())
        .map_err(|_| environment_error("Check journal write failed"))
}

fn publish(path: &Path, bytes: &[u8], sequence: u64) -> Result<(), DaemonError> {
    let capacity = bytes.len().max(64 * 1024).next_power_of_two() as u64;
    let mut image = vec![0; (PAGE + 2 * (PAGE + capacity)) as usize];
    image[..8].copy_from_slice(MAGIC);
    image[8..16].copy_from_slice(&capacity.to_le_bytes());
    let record = record(bytes, sequence);
    image[PAGE as usize..PAGE as usize + record.len()].copy_from_slice(&record);
    // Existing private writer syncs the fully allocated file and its directory.
    crate::config::write_private_file(path, &image)
        .map_err(|_| environment_error("Check journal write failed"))
}

fn record(bytes: &[u8], sequence: u64) -> Vec<u8> {
    let mut record = vec![0; PAGE as usize + bytes.len()];
    record[..8].copy_from_slice(&sequence.to_le_bytes());
    record[8..16].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
    let mut hash = Sha256::new();
    hash.update(&record[..16]);
    hash.update(bytes);
    record[16..48].copy_from_slice(&hash.finalize());
    record[PAGE as usize..].copy_from_slice(bytes);
    record
}

fn capacity(file: &mut File) -> Result<Option<u64>, DaemonError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| environment_error("Check journal unavailable"))?;
    let mut header = [0; 16];
    file.read_exact(&mut header)
        .map_err(|_| environment_error("invalid Check journal"))?;
    if &header[..8] != MAGIC {
        return if header[0] == b'{' {
            Ok(None)
        } else {
            Err(environment_error("invalid Check journal format"))
        };
    }
    let capacity = u64::from_le_bytes(header[8..16].try_into().unwrap());
    if !(64 * 1024..=LIMIT).contains(&capacity)
        || !capacity.is_power_of_two()
        || file
            .metadata()
            .map_err(|_| environment_error("Check journal unavailable"))?
            .len()
            != PAGE + 2 * (PAGE + capacity)
    {
        return Err(environment_error("invalid Check journal bounds"));
    }
    Ok(Some(capacity))
}

fn latest(file: &mut File, capacity: u64) -> Result<Snapshot, DaemonError> {
    let mut best: Option<Snapshot> = None;
    for slot in 0..2 {
        file.seek(SeekFrom::Start(PAGE + slot * (PAGE + capacity)))
            .map_err(|_| environment_error("Check journal unavailable"))?;
        let mut header = [0; 48];
        file.read_exact(&mut header)
            .map_err(|_| environment_error("Check journal unavailable"))?;
        let sequence = u64::from_le_bytes(header[..8].try_into().unwrap());
        let len = u64::from_le_bytes(header[8..16].try_into().unwrap());
        if sequence == 0 || len > capacity {
            continue;
        }
        file.seek(SeekFrom::Current(PAGE as i64 - 48))
            .map_err(|_| environment_error("Check journal unavailable"))?;
        let mut bytes = vec![0; len as usize];
        file.read_exact(&mut bytes)
            .map_err(|_| environment_error("Check journal unavailable"))?;
        let mut hash = Sha256::new();
        hash.update(&header[..16]);
        hash.update(&bytes);
        if hash.finalize()[..] != header[16..48] {
            continue;
        }
        if best.as_ref().is_some_and(|b| b.sequence == sequence) {
            return Err(environment_error("invalid Check journal sequence"));
        }
        if best.as_ref().is_none_or(|b| sequence > b.sequence) {
            best = Some(Snapshot {
                sequence,
                slot,
                bytes,
            });
        }
    }
    best.ok_or_else(|| environment_error("invalid Check journal"))
}

fn legacy_latest(file: &mut File) -> Result<(Vec<u8>, u64), DaemonError> {
    let len = file
        .metadata()
        .map_err(|_| environment_error("Check journal unavailable"))?
        .len();
    if len == 0 || len > LIMIT + 1 {
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
        let flags = if write { libc::O_RDWR } else { libc::O_RDONLY };
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
        .write(write)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Updates must never mutate a linked external inode or weaken private mode.
        if metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(file)
}
