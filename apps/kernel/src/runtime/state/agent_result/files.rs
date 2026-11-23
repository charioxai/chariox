use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Component, Path};

const DIRECTORY_FLAGS: i32 = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW;
const READ_FLAGS: i32 = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK;

pub(super) fn open_source(scratch: &Path, name: &str) -> Result<File, String> {
    if matches!(name, "" | "." | "..") || name.contains('/') || name.chars().any(char::is_control) {
        return Err("Use a single filename in your own TMPDIR".into());
    }
    let name = CString::new(name).map_err(|_| "Use a valid filename")?;
    let directory = scratch_directory(scratch).map_err(scratch_error)?;
    let file = open_at(&directory, &name, READ_FLAGS)
        .map_err(|error| format!("Could not open the file in your TMPDIR: {error}"))?;
    let metadata = file.metadata().map_err(scratch_error)?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err("Publish a regular file you created in TMPDIR".into());
    }
    Ok(file)
}

pub(super) fn materialize(scratch: &Path, name: &str, data: &[u8]) -> Result<(), String> {
    let directory = scratch_directory(scratch).map_err(scratch_error)?;
    let name = CString::new(name).map_err(|_| "Invalid result filename")?;
    let stage_name = CString::new(format!(".result-{:032x}", rand::random::<u128>())).unwrap();
    let mut staged = open_at(
        &directory,
        &stage_name,
        libc::O_WRONLY | libc::O_EXCL | libc::O_CREAT,
    )
    .map_err(scratch_error)?;
    let result = (|| {
        staged.write_all(data)?;
        staged.sync_all()?;
        // SAFETY: both names are in the pinned directory and all arguments stay valid.
        let status = unsafe {
            libc::linkat(
                directory.as_raw_fd(),
                stage_name.as_ptr(),
                directory.as_raw_fd(),
                name.as_ptr(),
                0,
            )
        };
        if status == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
        let existing = open_at(&directory, &name, READ_FLAGS)?;
        let meta = existing.metadata()?;
        if meta.is_file() && meta.len() == data.len() as u64 {
            let mut existing_bytes = Vec::with_capacity(data.len());
            existing
                .take(data.len() as u64 + 1)
                .read_to_end(&mut existing_bytes)?;
            if existing_bytes == data {
                return Ok(());
            }
        }
        Err(io::Error::new(
            error.kind(),
            "remove the changed result copy from TMPDIR and fetch again",
        ))
    })();
    // SAFETY: descriptor and name are valid for this call.
    unsafe { libc::unlinkat(directory.as_raw_fd(), stage_name.as_ptr(), 0) };
    result.map_err(scratch_error)
}

fn scratch_directory(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(io::Error::other("temporary directory must be absolute"));
    }
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(DIRECTORY_FLAGS)
        .open("/")?;
    for component in path.components().skip(1) {
        let Component::Normal(name) = component else {
            return Err(io::Error::other("invalid temporary directory path"));
        };
        let name = CString::new(name.as_bytes())?;
        directory = open_at(&directory, &name, DIRECTORY_FLAGS)?;
    }
    Ok(directory)
}

fn open_at(directory: &File, name: &CStr, flags: i32) -> io::Result<File> {
    // SAFETY: directory is open, name is NUL-terminated, and a success returns a new descriptor.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn scratch_error(error: io::Error) -> String {
    format!("Could not use your private TMPDIR: {error}")
}
