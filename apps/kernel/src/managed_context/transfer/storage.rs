use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::config::write_private_file;
use crate::error::DaemonError;

use super::policy::transfer_error;

const MAX_STATE_FILE_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn write_private_state_file(path: &Path, bytes: &[u8]) -> Result<(), DaemonError> {
    if bytes.len() as u64 > MAX_STATE_FILE_BYTES {
        return Err(transfer_error(
            "managed context transfer state exceeds its size limit",
        ));
    }
    write_private_file(path, bytes)
        .map_err(|error| transfer_io_error("persist managed context transfer state", error))
}

pub(super) fn read_private_state_file(path: &Path) -> Result<Option<Vec<u8>>, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(transfer_io_error(
                "read managed context transfer state",
                error,
            ))
        }
    };
    let metadata = file
        .metadata()
        .map_err(|error| transfer_io_error("inspect managed context transfer state", error))?;
    if !metadata.is_file() || metadata.len() > MAX_STATE_FILE_BYTES {
        return Err(transfer_error(
            "managed context transfer state must be a bounded regular file",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_STATE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| transfer_io_error("read managed context transfer state", error))?;
    if bytes.len() as u64 > MAX_STATE_FILE_BYTES {
        return Err(transfer_error(
            "managed context transfer state exceeds its size limit",
        ));
    }
    Ok(Some(bytes))
}

pub(super) fn sha256_file(path: &Path) -> Result<String, DaemonError> {
    let mut file = open_private_archive(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| transfer_io_error("hash managed context archive", error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(super) fn ensure_private_directory(path: &Path) -> Result<(), DaemonError> {
    fs::create_dir_all(path)
        .map_err(|error| transfer_io_error("create managed context transfer directory", error))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| transfer_io_error("inspect managed context transfer directory", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(transfer_error(
            "managed context transfer root must be a real directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            transfer_io_error("secure managed context transfer directory", error)
        })?;
    }
    Ok(())
}

pub(super) fn create_or_validate_empty_archive(path: &Path) -> Result<(), DaemonError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(file) => file
            .sync_all()
            .map_err(|error| transfer_io_error("create managed context archive", error)),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let file = open_private_archive(path)?;
            if file
                .metadata()
                .map_err(|error| transfer_io_error("inspect managed context archive", error))?
                .len()
                == 0
            {
                Ok(())
            } else {
                Err(transfer_error(
                    "armed managed context transfer has a nonempty archive",
                ))
            }
        }
        Err(error) => Err(transfer_io_error("create managed context archive", error)),
    }
}

pub(super) fn remove_archive_if_present(path: &Path) -> Result<(), DaemonError> {
    match fs::remove_file(path) {
        Ok(()) => sync_parent_directory(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(transfer_io_error("remove managed context archive", error)),
    }
}

pub(super) fn open_private_archive(path: &Path) -> Result<File, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    configure_no_follow(&mut options);
    let file = options
        .open(path)
        .map_err(|error| transfer_io_error("open managed context archive", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| transfer_io_error("inspect managed context archive", error))?;
    if !metadata.is_file() {
        return Err(transfer_error(
            "managed context archive must be a regular file",
        ));
    }
    Ok(file)
}

fn configure_no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
}

fn sync_parent_directory(path: &Path) -> Result<(), DaemonError> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| transfer_io_error("sync managed context transfer directory", error))?;
    }
    Ok(())
}

pub(super) fn transfer_io_error(operation: &'static str, error: io::Error) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_unavailable",
        operation,
        message: error.to_string(),
        retryable: true,
    }
}
