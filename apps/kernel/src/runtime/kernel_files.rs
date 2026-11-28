//! File access on this machine for the kernel file requests (External Agents v2).
//!
//! The kernel runs as an OS user and can touch what that user can: these
//! requests add no power over a terminal on the kernel, only a transfer that
//! is chunked, resumable and checked by sha256. Paths are absolute or `~/`.
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::local::{
    KernelDirectoryEntry, KernelFileKind, KernelFileStat, ListKernelDirectoryRequest,
    LocalDaemonResponse, ReadKernelFileRequest, StatKernelFileRequest, WriteKernelFileRequest,
    KERNEL_FILE_MAX_BYTES, KERNEL_FILE_MAX_CHUNK_BYTES,
};
use crate::machine_files::StateGuard;

/// Directory listings stop here and say so.
const MAX_DIRECTORY_ENTRIES: usize = 2000;
const PARTIAL_SUFFIX: &str = ".chariox-partial";
/// A client sends the chunks of an upload without waiting for each answer, and each runs on its own
/// thread, so one can start before the chunk ahead of it has landed. It waits that long for the
/// staged bytes to reach its offset before it is refused as out of order. On a slow uplink the chunk
/// ahead can land a whole queue of chunks later: as long as the client waits for any answer.
const TURN_WAIT: Duration = Duration::from_secs(30);
const TURN_POLL: Duration = Duration::from_millis(2);

/// Uploads whose first chunk this kernel has staged, by staging path. A later chunk that finds its
/// folder gone can then say the folder was removed. Without the record, a chunk that starts ahead
/// of the first one (which creates the folders) cannot tell "not started yet" from "removed", so it
/// waits its turn instead. Bounded: an abandoned upload only costs its entry until the cap evicts it.
static STARTED_UPLOADS: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, Instant>>> =
    std::sync::Mutex::new(None);
const STARTED_UPLOADS_CAP: usize = 4096;

fn note_upload_started(staging: &Path) {
    note_upload_activity_at(staging, Instant::now());
}

fn note_upload_activity_at(staging: &Path, at: Instant) {
    let mut started = STARTED_UPLOADS.lock().unwrap_or_else(|e| e.into_inner());
    let started = started.get_or_insert_with(Default::default);
    if started.len() >= STARTED_UPLOADS_CAP {
        if let Some(oldest) = started
            .iter()
            .min_by_key(|(_, at)| **at)
            .map(|(path, _)| path.clone())
        {
            started.remove(&oldest);
        }
    }
    started.insert(staging.to_path_buf(), at);
}

/// An upload counts as established here while its chunks keep arriving: one silent for longer than
/// a chunk may wait for its turn is abandoned, and a later upload to the same path starts over.
fn upload_started(staging: &Path) -> bool {
    upload_started_at(staging, Instant::now())
}

fn upload_started_at(staging: &Path, now: Instant) -> bool {
    STARTED_UPLOADS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|started| started.get(staging))
        .is_some_and(|last| now.saturating_duration_since(*last) <= TURN_WAIT)
}

fn forget_upload(staging: &Path) {
    if let Some(started) = STARTED_UPLOADS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
    {
        started.remove(staging);
    }
}

pub(super) fn fail(code: &str, message: impl Into<String>) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.into(),
        message: message.into(),
    }
}

pub(super) fn io_failure(path: &Path, error: &std::io::Error) -> DaemonError {
    match error.kind() {
        std::io::ErrorKind::NotFound => fail(
            "file_not_found",
            format!(
                "{} does not exist; check the path with a directory listing.",
                path.display()
            ),
        ),
        std::io::ErrorKind::PermissionDenied => fail(
            "file_permission_denied",
            format!("The kernel's user may not access {}.", path.display()),
        ),
        _ => fail(
            "file_io_failed",
            format!("Could not access {}: {error}.", path.display()),
        ),
    }
}

/// `~/x` becomes the kernel user's home; anything else must be absolute. The kernel's own state
/// and the Vault are not files of the machine as far as this service goes.
pub(super) fn resolve(path: &str, guard: &StateGuard) -> Result<PathBuf, DaemonError> {
    let resolved = match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| {
                fail(
                    "file_path_invalid",
                    "This kernel has no home directory; use an absolute path.",
                )
            })?
            .join(rest),
        None => PathBuf::from(path),
    };
    if !resolved.is_absolute() {
        return Err(fail(
            "file_path_invalid",
            format!("\"{path}\" is not an absolute path; start it with / or ~/."),
        ));
    }
    guard
        .check(&resolved)
        .map_err(|message| fail("file_state_protected", message))?;
    Ok(resolved)
}

fn folder_failure(folder: &Path, error: &std::io::Error) -> DaemonError {
    use std::io::ErrorKind;
    let (code, reason) = match error.kind() {
        ErrorKind::NotFound => return fail(
            "file_destination_missing",
            format!("Destination folder {} no longer exists. Recreate it or choose another folder, then upload again.", folder.display()),
        ),
        ErrorKind::NotADirectory | ErrorKind::AlreadyExists => {
            ("file_not_a_file", "a file blocks the path")
        }
        ErrorKind::PermissionDenied => (
            "file_permission_denied",
            "the kernel's user lacks permission",
        ),
        _ => {
            return fail(
                "file_io_failed",
                format!(
                "Could not prepare folder {}: {error}. Check the path or choose another folder.",
                folder.display()
            ),
            )
        }
    };
    fail(
        code,
        format!("Could not prepare folder {} because {reason}. Fix the folder path or choose another folder.", folder.display()),
    )
}

/// A machine destination is checked before any parent is created and again at its resolved path.
pub(crate) fn prepare_write_target(
    path: &Path,
    guard: &StateGuard,
) -> Result<PathBuf, DaemonError> {
    let name = path.file_name().ok_or_else(|| {
        fail(
            "file_path_invalid",
            format!("{} names no file. Choose a file path.", path.display()),
        )
    })?;
    if path.components().any(|part| part == Component::ParentDir) {
        return Err(fail(
            "file_path_invalid",
            format!(
                "{} contains '..'. Use the full path to the destination.",
                path.display()
            ),
        ));
    }
    let folder = path.parent().expect("an absolute file path has a parent");
    guard
        .check(folder)
        .map_err(|message| fail("file_state_protected", message))?;
    std::fs::create_dir_all(folder).map_err(|error| folder_failure(folder, &error))?;
    let target = folder
        .canonicalize()
        .map_err(|error| folder_failure(folder, &error))?
        .join(name);
    guard
        .check(&target)
        .map_err(|message| fail("file_state_protected", message))?;
    Ok(target)
}

fn partial_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(PARTIAL_SUFFIX);
    PathBuf::from(name)
}

fn kind_of(file_type: std::fs::FileType) -> KernelFileKind {
    if file_type.is_symlink() {
        KernelFileKind::Symlink
    } else if file_type.is_dir() {
        KernelFileKind::Directory
    } else if file_type.is_file() {
        KernelFileKind::File
    } else {
        KernelFileKind::Other
    }
}

fn modified_ms(metadata: &std::fs::Metadata) -> Option<u64> {
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(modified.as_millis()).ok()
}

fn sha256_of(path: &Path) -> Result<String, DaemonError> {
    let file = std::fs::File::open(path).map_err(|e| io_failure(path, &e))?;
    sha256_of_open(&file, path)
}

/// Hashes the open file from its first byte, whatever its descriptor offset.
pub(crate) fn sha256_of_open(file: &std::fs::File, path: &Path) -> Result<String, DaemonError> {
    use std::os::unix::fs::FileExt;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    let mut offset = 0_u64;
    loop {
        let read = file
            .read_at(&mut buffer, offset)
            .map_err(|e| io_failure(path, &e))?;
        if read == 0 {
            return Ok(format!("{:x}", hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
        offset += read as u64;
    }
}

pub(crate) fn stat(
    request: &StatKernelFileRequest,
    guard: &StateGuard,
) -> Result<LocalDaemonResponse, DaemonError> {
    let path = resolve(&request.path, guard)?;
    let partial_size = std::fs::metadata(partial_of(&path)).ok().map(|m| m.len());
    let stat = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            let followed = std::fs::metadata(&path).unwrap_or_else(|_| metadata.clone());
            let kind = kind_of(followed.file_type());
            let hashed = request.with_sha256 && kind == KernelFileKind::File;
            if hashed && followed.len() > KERNEL_FILE_MAX_BYTES {
                return Err(too_large(followed.len()));
            }
            KernelFileStat {
                path: path.display().to_string(),
                exists: true,
                kind: Some(kind),
                size: Some(followed.len()),
                modified_ms: modified_ms(&followed),
                revision: Some(crate::agent_file_copy::revision(&followed)),
                mode: Some(followed.mode() & 0o7777),
                sha256: if hashed {
                    Some(sha256_of(&path)?)
                } else {
                    None
                },
                partial_size,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => KernelFileStat {
            path: path.display().to_string(),
            exists: false,
            kind: None,
            size: None,
            modified_ms: None,
            revision: None,
            mode: None,
            sha256: None,
            partial_size,
        },
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotADirectory | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            return Err(folder_failure(path.parent().unwrap_or(&path), &error));
        }
        Err(error) => return Err(io_failure(&path, &error)),
    };
    Ok(LocalDaemonResponse::KernelFileStat { stat })
}

pub(crate) fn list(
    request: &ListKernelDirectoryRequest,
    guard: &StateGuard,
) -> Result<LocalDaemonResponse, DaemonError> {
    let path = resolve(&request.path, guard)?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(|e| io_failure(&path, &e))? {
        let entry = entry.map_err(|e| io_failure(&path, &e))?;
        // The kernel's own folders are not listed, not even by name.
        if guard.check(&entry.path()).is_err() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        // A link shows what it points to, as a file manager does; a broken one stays a link.
        let followed = std::fs::metadata(entry.path());
        let target_kind = match (&followed, metadata.file_type().is_symlink()) {
            (Ok(followed), true) => Some(kind_of(followed.file_type())),
            _ => None,
        };
        let followed = followed.unwrap_or_else(|_| metadata.clone());
        entries.push(KernelDirectoryEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            kind: kind_of(metadata.file_type()),
            target_kind,
            size: followed.len(),
            modified_ms: modified_ms(&followed),
            revision: crate::agent_file_copy::revision(&followed),
            mode: followed.mode() & 0o7777,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let first = usize::try_from(request.offset).unwrap_or(usize::MAX);
    let page: Vec<_> = entries
        .iter()
        .skip(first)
        .take(MAX_DIRECTORY_ENTRIES)
        .cloned()
        .collect();
    let truncated = first.saturating_add(page.len()) < entries.len();
    Ok(LocalDaemonResponse::KernelDirectoryListed {
        path: path.display().to_string(),
        entries: page,
        truncated,
    })
}

fn too_large(size: u64) -> DaemonError {
    fail(
        "file_too_large",
        format!("A file may be at most {KERNEL_FILE_MAX_BYTES} bytes; this one is {size}."),
    )
}

pub(crate) fn read(
    request: &ReadKernelFileRequest,
    guard: &StateGuard,
) -> Result<LocalDaemonResponse, DaemonError> {
    if request.length > KERNEL_FILE_MAX_CHUNK_BYTES {
        return Err(fail(
            "file_chunk_too_large",
            format!("Read at most {KERNEL_FILE_MAX_CHUNK_BYTES} bytes per request."),
        ));
    }
    let path = resolve(&request.path, guard)?;
    let mut file = std::fs::File::open(&path).map_err(|e| io_failure(&path, &e))?;
    let metadata = file.metadata().map_err(|e| io_failure(&path, &e))?;
    if !metadata.is_file() {
        return Err(fail(
            "file_not_a_file",
            format!(
                "{} is not a regular file; list a directory to see its files.",
                path.display()
            ),
        ));
    }
    if metadata.len() > KERNEL_FILE_MAX_BYTES {
        return Err(too_large(metadata.len()));
    }
    file.seek(SeekFrom::Start(request.offset))
        .map_err(|e| io_failure(&path, &e))?;
    let mut data = Vec::new();
    file.take(request.length)
        .read_to_end(&mut data)
        .map_err(|e| io_failure(&path, &e))?;
    let eof = request.offset.saturating_add(data.len() as u64) >= metadata.len();
    Ok(LocalDaemonResponse::KernelFileChunk {
        path: path.display().to_string(),
        offset: request.offset,
        data_base64: STANDARD.encode(&data),
        size: metadata.len(),
        eof,
    })
}

fn missing_destination(path: &Path) -> Option<DaemonError> {
    let folder = path.parent()?;
    let error = std::fs::metadata(folder).err()?;
    (error.kind() == std::io::ErrorKind::NotFound).then(|| folder_failure(folder, &error))
}

fn upload_failure(path: &Path, error: &std::io::Error) -> DaemonError {
    if error.kind() == std::io::ErrorKind::NotFound {
        if let Some(error) = missing_destination(path) {
            return error;
        }
    }
    io_failure(path, error)
}

/// Opens the upload's staging file without trusting what is already at the predictable name:
/// a first chunk replaces whatever is there, a later chunk continues only the private regular
/// file this user made (another user of a shared directory may have planted a link or a file).
fn open_staging(partial: &Path, path: &Path, offset: u64) -> Result<std::fs::File, DaemonError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let untrusted = || {
        fail(
            "file_staging_untrusted",
            format!(
                "The unfinished upload for {} has changed. Choose another destination or restart the upload from offset 0.",
                path.display()
            ),
        )
    };
    if offset == 0 {
        match std::fs::remove_file(partial) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_failure(path, &e)),
        }
        // Private from the first byte: the unfinished upload may become a credentials file.
        // A new file stays private; replacing one takes the replaced file's permissions on finish.
        return std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(partial)
            .map(|file| {
                note_upload_started(partial);
                file
            })
            .map_err(|e| upload_failure(path, &e));
    }
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(partial)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            forget_upload(partial);
            return Err(missing_destination(path).unwrap_or_else(|| offset_mismatch(path, 0)));
        }
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Err(untrusted()),
        Err(e) => return Err(io_failure(path, &e)),
    };
    let metadata = file.metadata().map_err(|e| io_failure(path, &e))?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let own_uid = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || metadata.uid() != own_uid
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(untrusted());
    }
    if metadata.len() != offset {
        return Err(offset_mismatch(path, metadata.len()));
    }
    Ok(file)
}

/// Holds a chunk that is ahead of the staged bytes until the chunks before it have landed. A chunk
/// at a wrong offset still ends as `file_offset_mismatch`, after this wait.
fn wait_for_turn(partial: &Path, path: &Path, offset: u64) -> Result<(), DaemonError> {
    let deadline = Instant::now() + TURN_WAIT;
    while offset > 0 && Instant::now() < deadline {
        match std::fs::symlink_metadata(partial) {
            Ok(metadata) if metadata.len() >= offset => break,
            // The first chunk creates the folders and may still be on its way: only an upload
            // already started here has lost its folder.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if upload_started(partial) {
                    if let Some(error) = missing_destination(path) {
                        // This attempt ends here; a retry to the same path starts over.
                        forget_upload(partial);
                        return Err(error);
                    }
                }
            }
            _ => {}
        }
        std::thread::sleep(TURN_POLL);
    }
    Ok(())
}

fn offset_mismatch(path: &Path, have: u64) -> DaemonError {
    fail(
        "file_offset_mismatch",
        format!(
            "{} has {have} bytes of this upload; continue at offset {have}.",
            path.display()
        ),
    )
}

pub(crate) fn write(
    request: &WriteKernelFileRequest,
    guard: &StateGuard,
) -> Result<LocalDaemonResponse, DaemonError> {
    if request.total_size > KERNEL_FILE_MAX_BYTES {
        return Err(too_large(request.total_size));
    }
    let data = STANDARD
        .decode(&request.data_base64)
        .map_err(|_| fail("file_path_invalid", "data_base64 is not valid base64."))?;
    if data.len() as u64 > KERNEL_FILE_MAX_CHUNK_BYTES {
        return Err(fail(
            "file_chunk_too_large",
            format!("Write at most {KERNEL_FILE_MAX_CHUNK_BYTES} bytes per request."),
        ));
    }
    let path = resolve(&request.path, guard)?;
    let target = if request.offset == 0 {
        prepare_write_target(&path, guard)?
    } else {
        path.clone()
    };
    let staging = partial_of(&target);
    guard.check(&staging).map_err(|_| fail(
        "file_state_protected",
        format!("The upload for {} overlaps the kernel's own state or Vault. Choose another file path.", path.display()),
    ))?;
    let written = write_at(
        &target,
        &staging,
        &StagedChunk {
            offset: request.offset,
            total_size: request.total_size,
            data: &data,
            overwrite: request.overwrite,
            sha256: request.sha256.as_deref(),
        },
    )?;
    Ok(LocalDaemonResponse::KernelFileWritten {
        path: path.display().to_string(),
        received: written.received,
        complete: written.sha256.is_some(),
        sha256: written.sha256,
    })
}

/// One chunk of an upload into an already resolved path.
pub(crate) struct StagedChunk<'a> {
    pub offset: u64,
    pub total_size: u64,
    pub data: &'a [u8],
    pub overwrite: bool,
    /// Checked against the whole file before it appears under its name.
    pub sha256: Option<&'a str>,
}

/// Where an upload stands after a chunk: the bytes staged so far, and the digest of the file once
/// it is complete and installed.
pub(crate) struct StagedWrite {
    pub received: u64,
    pub sha256: Option<String>,
}

/// Stages `chunk` for `path` in the file `staging` and installs the file when its last byte lands;
/// nothing appears under the name before then, and an existing file is replaced only with
/// `overwrite`.
pub(crate) fn write_at(
    path: &Path,
    staging: &Path,
    chunk: &StagedChunk<'_>,
) -> Result<StagedWrite, DaemonError> {
    let partial = staging.to_path_buf();
    let existing = std::fs::symlink_metadata(path);
    if existing.is_ok() && !chunk.overwrite {
        return Err(fail(
            "file_exists",
            format!(
                "{} already exists; send overwrite to replace it.",
                path.display()
            ),
        ));
    }
    if existing.as_ref().is_ok_and(|m| m.is_dir()) {
        return Err(fail(
            "file_not_a_file",
            format!(
                "{} is a directory; choose a file path inside it.",
                path.display()
            ),
        ));
    }
    let end = chunk.offset.saturating_add(chunk.data.len() as u64);
    if end > chunk.total_size {
        return Err(fail(
            "file_offset_mismatch",
            format!(
                "The chunk ends at {end}, past total_size {}.",
                chunk.total_size
            ),
        ));
    }
    wait_for_turn(&partial, path, chunk.offset)?;
    let mut file = open_staging(&partial, path, chunk.offset)?;
    file.write_all(chunk.data)
        .map_err(|e| io_failure(path, &e))?;
    file.flush().map_err(|e| io_failure(path, &e))?;
    note_upload_started(&partial);
    if end < chunk.total_size {
        return Ok(StagedWrite {
            received: end,
            sha256: None,
        });
    }
    let sha256 = publish(&file, &partial, path, chunk.overwrite, chunk.sha256)?;
    // Published: this upload is over. A refused publish keeps its staged bytes, so it stays known.
    forget_upload(&partial);
    Ok(StagedWrite {
        received: end,
        sha256: Some(sha256),
    })
}

/// Bytes the staging file `staging` of an unfinished upload holds.
pub(crate) fn staged_bytes(staging: &Path) -> u64 {
    std::fs::symlink_metadata(staging)
        .ok()
        .filter(std::fs::Metadata::is_file)
        .map_or(0, |metadata| metadata.len())
}

/// Drops the staging file `staging` of an unfinished upload.
pub(crate) fn discard_staging(staging: &Path) {
    forget_upload(staging);
    let _ = std::fs::remove_file(staging);
}

fn staging_replaced(path: &Path) -> DaemonError {
    if let Some(error) = missing_destination(path) {
        return error;
    }
    fail(
        "file_staging_untrusted",
        format!(
            "The unfinished upload for {} was replaced while it finished. Upload the file again.",
            path.display()
        ),
    )
}

fn names_the_open_file(path: &Path, file: &std::fs::File) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(path), file.metadata()) {
        (Ok(named), Ok(open)) => named.dev() == open.dev() && named.ino() == open.ino(),
        _ => false,
    }
}

/// Installs the staged file over `path`. The file being replaced stays under another name until
/// the install is verified, so a staging name swapped at the last moment costs nothing: the
/// foreign file is removed and the original put back.
fn replace(file: &std::fs::File, partial: &Path, path: &Path) -> Result<(), DaemonError> {
    let mut previous = path.as_os_str().to_owned();
    previous.push(format!(
        ".chariox-previous-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let previous = PathBuf::from(previous);
    let kept = std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
        && std::fs::hard_link(path, &previous).is_ok();
    if let Err(error) = std::fs::rename(partial, path) {
        if kept {
            let _ = std::fs::remove_file(&previous);
        }
        return Err(upload_failure(path, &error));
    }
    if names_the_open_file(path, file) {
        if kept {
            let _ = std::fs::remove_file(&previous);
        }
        return Ok(());
    }
    let _ = std::fs::remove_file(path);
    if kept {
        let _ = std::fs::rename(&previous, path);
    }
    Err(staging_replaced(path))
}

/// Verifies, sets the permissions of and installs the staged upload through the descriptor that
/// received it: another user of a shared directory can swap the staging name at any moment, so
/// no step trusts the name, and a swap that wins the race is removed again and reported.
fn publish(
    file: &std::fs::File,
    partial: &Path,
    path: &Path,
    overwrite: bool,
    expected_sha256: Option<&str>,
) -> Result<String, DaemonError> {
    let sha256 = sha256_of_open(file, path)?;
    if let Some(expected) = expected_sha256 {
        if !expected.eq_ignore_ascii_case(&sha256) {
            let _ = std::fs::remove_file(partial);
            return Err(fail(
                "file_hash_mismatch",
                format!("The received file hashes to {sha256}, not {expected}; nothing was kept. Upload it again."),
            ));
        }
    }
    if !names_the_open_file(partial, file) {
        return Err(staging_replaced(path));
    }
    if overwrite {
        // Replacing a file keeps its permissions (a 0600 credentials file stays private).
        if let Ok(existing) = std::fs::metadata(path) {
            file.set_permissions(existing.permissions())
                .map_err(|e| io_failure(path, &e))?;
        }
        replace(file, partial, path)?;
    } else {
        // A hard link fails when something appeared at `path` meanwhile; a rename would replace it.
        std::fs::hard_link(partial, path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                fail(
                    "file_exists",
                    format!(
                        "{} already exists; send overwrite to replace it.",
                        path.display()
                    ),
                )
            } else {
                upload_failure(path, &error)
            }
        })?;
        if !names_the_open_file(path, file) {
            let _ = std::fs::remove_file(path);
            return Err(staging_replaced(path));
        }
        let _ = std::fs::remove_file(partial);
    }
    Ok(sha256)
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests run with nothing protected; the guard has its own tests.
    fn stat(request: &StatKernelFileRequest) -> Result<LocalDaemonResponse, DaemonError> {
        super::stat(request, &StateGuard::default())
    }
    fn list(request: &ListKernelDirectoryRequest) -> Result<LocalDaemonResponse, DaemonError> {
        super::list(request, &StateGuard::default())
    }
    fn read(request: &ReadKernelFileRequest) -> Result<LocalDaemonResponse, DaemonError> {
        super::read(request, &StateGuard::default())
    }
    fn write(request: &WriteKernelFileRequest) -> Result<LocalDaemonResponse, DaemonError> {
        super::write(request, &StateGuard::default())
    }

    fn scratch() -> crate::test_support::TestTempDir {
        crate::test_support::TestTempDir::new("kernel-files")
    }

    fn chunk(
        path: &Path,
        offset: u64,
        bytes: &[u8],
        total: u64,
        sha: Option<&str>,
    ) -> WriteKernelFileRequest {
        WriteKernelFileRequest {
            path: path.display().to_string(),
            offset,
            data_base64: STANDARD.encode(bytes),
            total_size: total,
            sha256: sha.map(str::to_string),
            overwrite: false,
        }
    }

    fn code(error: DaemonError) -> String {
        match error {
            DaemonError::MachineRequest { code, .. } => code,
            other => panic!("not a kernel file error: {other}"),
        }
    }

    fn written(response: LocalDaemonResponse) -> (u64, bool, Option<String>) {
        let LocalDaemonResponse::KernelFileWritten {
            received,
            complete,
            sha256,
            ..
        } = response
        else {
            panic!("not a write answer")
        };
        (received, complete, sha256)
    }

    #[test]
    fn first_upload_chunk_creates_parents_only_inside_the_allowed_path() {
        let dir = scratch();
        let target = dir.join("a/b/c/file.txt");
        assert!(written(write(&chunk(&target, 0, b"new", 3, None)).unwrap()).1);
        assert_eq!(std::fs::read(&target).unwrap(), b"new");

        let protected = dir.join("private");
        std::fs::create_dir(&protected).unwrap();
        let guard = StateGuard::new(Some(protected.to_str().unwrap()));
        let link = dir.join("shortcut");
        std::os::unix::fs::symlink(&protected, &link).unwrap();
        for target in [link.join("new/file"), dir.join("a/../private/new/file")] {
            let error = super::write(&chunk(&target, 0, b"new", 3, None), &guard).unwrap_err();
            assert_eq!(code(error), "file_state_protected");
        }
        assert!(!protected.join("new").exists());
    }

    #[test]
    fn parent_and_staging_errors_refer_to_the_requested_path() {
        let dir = scratch();
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"file").unwrap();
        let target = blocked.join("folder/file");
        let probe = stat(&StatKernelFileRequest {
            path: target.display().to_string(),
            with_sha256: false,
        })
        .unwrap_err();
        let write_error = write(&chunk(&target, 0, b"new", 3, None)).unwrap_err();
        for error in [probe, write_error] {
            let message = error.to_string();
            assert!(message.contains(&blocked.join("folder").display().to_string()));
            assert!(message.contains("a file blocks the path"));
            assert!(!message.contains(PARTIAL_SUFFIX));
        }

        let target = dir.join("upload");
        std::fs::create_dir(partial_of(&target)).unwrap();
        let error = write(&chunk(&target, 0, b"new", 3, None)).unwrap_err();
        for error in [error, staging_replaced(&target)] {
            let message = error.to_string();
            assert!(message.contains(&target.display().to_string()));
            assert!(!message.contains(PARTIAL_SUFFIX));
        }
    }

    #[test]
    fn continuing_an_upload_names_the_destination_folder_that_disappeared() {
        let dir = scratch();
        let folder = dir.join("incoming");
        let target = folder.join("upload.txt");
        write(&chunk(&target, 0, b"a", 2, None)).unwrap();
        std::fs::remove_dir_all(&folder).unwrap();

        let error = write(&chunk(&target, 1, b"b", 2, None)).unwrap_err();
        let message = error.to_string();
        assert_eq!(code(error), "file_destination_missing");
        assert!(message.contains(&format!(
            "Destination folder {} no longer exists",
            folder.display()
        )));
        assert!(message.contains("Recreate it or choose another folder"));
        assert!(!message.contains(PARTIAL_SUFFIX));
        assert!(!folder.exists());
    }

    // The record of an upload that failed (its folder was removed) must not make a retried
    // upload to the same path reject its own out-of-order startup.
    #[test]
    fn a_retried_upload_after_a_removed_folder_still_waits_for_its_first_chunk() {
        let dir = scratch();
        let target = dir.join("incoming/nested/retry.bin");
        let sha = format!("{:x}", Sha256::digest(b"abcdef"));
        write(&chunk(&target, 0, b"abc", 6, Some(&sha))).unwrap();
        std::fs::remove_dir_all(dir.join("incoming")).unwrap();
        let error = write(&chunk(&target, 3, b"def", 6, Some(&sha))).unwrap_err();
        assert_eq!(code(error), "file_destination_missing");

        // The retry starts from offset 0, but its later chunk runs first and the folders are gone.
        let first = chunk(&target, 0, b"abc", 6, Some(&sha));
        let second = chunk(&target, 3, b"def", 6, Some(&sha));
        let late = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            write(&first)
        });
        let outcome = written(write(&second).unwrap());
        assert_eq!(outcome.0, 6);
        assert_eq!(written(late.join().unwrap().unwrap()).0, 3);
        assert_eq!(std::fs::read(&target).unwrap(), b"abcdef");
    }

    // The record of an abandoned upload must not make a retried
    // upload to the same path reject its own out-of-order startup.
    #[test]
    fn an_abandoned_upload_without_a_reported_error_does_not_reject_the_retry() {
        let dir = scratch();
        let target = dir.join("incoming/nested/retry.bin");
        let sha = format!("{:x}", Sha256::digest(b"abcdef"));
        write(&chunk(&target, 0, b"abc", 6, Some(&sha))).unwrap();
        std::fs::remove_dir_all(dir.join("incoming")).unwrap();

        // The retry starts from offset 0, but its later chunk runs first and the folders are gone.
        let first = chunk(&target, 0, b"abc", 6, Some(&sha));
        let second = chunk(&target, 3, b"def", 6, Some(&sha));
        let late = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            write(&first)
        });
        let outcome = written(write(&second).unwrap());
        assert_eq!(outcome.0, 6);
        assert_eq!(written(late.join().unwrap().unwrap()).0, 3);
        assert_eq!(std::fs::read(&target).unwrap(), b"abcdef");
    }

    // An upload that has been silent for longer than a chunk waits for its turn is abandoned: its
    // record no longer authorizes an immediate failure for a later upload to the same path.
    #[test]
    fn a_silent_uploads_record_expires() {
        let staging = scratch().join("silent.bin.chariox-partial");
        let long_ago = Instant::now()
            .checked_sub(TURN_WAIT + Duration::from_secs(1))
            .expect("the machine has been up for more than a chunk's wait");
        note_upload_activity_at(&staging, long_ago);
        assert!(!upload_started(&staging));
        note_upload_started(&staging);
        assert!(upload_started(&staging));
        forget_upload(&staging);
        assert!(!upload_started(&staging));
    }

    // EA2-07: an upload resumes at the offset the target reports, and the file
    // appears only when it is complete and its hash matches.
    #[test]
    fn an_upload_resumes_where_it_stopped_and_appears_only_when_complete_and_verified() {
        let dir = scratch();
        let target = dir.join("bundle.bin");
        let whole: Vec<u8> = (0..300_u32).map(|i| (i % 251) as u8).collect();
        let sha = format!("{:x}", Sha256::digest(&whole));
        let (received, complete, _) =
            written(write(&chunk(&target, 0, &whole[..100], 300, Some(&sha))).unwrap());
        assert_eq!((received, complete), (100, false));
        assert!(
            !target.exists(),
            "an unfinished upload is not presented as the file"
        );
        let LocalDaemonResponse::KernelFileStat { stat } = stat(&StatKernelFileRequest {
            path: target.display().to_string(),
            with_sha256: false,
        })
        .unwrap() else {
            panic!()
        };
        assert_eq!((stat.exists, stat.partial_size), (false, Some(100)));
        assert_eq!(
            code(write(&chunk(&target, 50, &whole[50..150], 300, None)).unwrap_err()),
            "file_offset_mismatch"
        );
        let (received, complete, _) =
            written(write(&chunk(&target, 100, &whole[100..200], 300, Some(&sha))).unwrap());
        assert_eq!((received, complete), (200, false));
        let (received, complete, hash) =
            written(write(&chunk(&target, 200, &whole[200..], 300, Some(&sha))).unwrap());
        assert_eq!(
            (received, complete, hash.as_deref()),
            (300, true, Some(sha.as_str()))
        );
        assert_eq!(std::fs::read(&target).unwrap(), whole);
        assert!(!partial_of(&target).exists());
    }

    #[test]
    fn a_wrong_hash_keeps_nothing_and_an_existing_file_needs_overwrite() {
        let dir = scratch();
        let target = dir.join("a.txt");
        let error = write(&chunk(&target, 0, b"hello", 5, Some(&"0".repeat(64)))).unwrap_err();
        assert_eq!(code(error), "file_hash_mismatch");
        assert!(!target.exists() && !partial_of(&target).exists());
        write(&chunk(&target, 0, b"hello", 5, None)).unwrap();
        assert_eq!(
            code(write(&chunk(&target, 0, b"again", 5, None)).unwrap_err()),
            "file_exists"
        );
        let mut replace = chunk(&target, 0, b"again", 5, None);
        replace.overwrite = true;
        write(&replace).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"again");
    }

    #[test]
    fn replacing_a_private_file_keeps_it_private_and_a_new_file_never_replaces() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let target = dir.join("credentials");
        std::fs::write(&target, b"old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut replace = chunk(&target, 0, b"new", 3, None);
        replace.overwrite = true;
        write(&replace).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // Nobody else can read the bytes of an unfinished upload, and a new file stays private.
        let fresh = dir.join("fresh");
        write(&chunk(&fresh, 0, b"ab", 4, None)).unwrap();
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&partial_of(&fresh)), 0o600);
        write(&chunk(&fresh, 2, b"cd", 4, None)).unwrap();
        assert_eq!(mode(&fresh), 0o600);
        // Something appears at the path while the upload is unfinished: no chunk replaces it.
        let racing = dir.join("racing");
        write(&chunk(&racing, 0, b"ab", 4, None)).unwrap();
        std::fs::write(&racing, b"someone else").unwrap();
        assert_eq!(
            code(write(&chunk(&racing, 2, b"cd", 4, None)).unwrap_err()),
            "file_exists"
        );
        assert_eq!(std::fs::read(&racing).unwrap(), b"someone else");
    }

    #[test]
    fn reads_are_chunked_and_the_caps_are_typed_errors() {
        let dir = scratch();
        let file = dir.join("r.bin");
        std::fs::write(&file, b"0123456789").unwrap();
        let LocalDaemonResponse::KernelFileChunk {
            data_base64,
            eof,
            size,
            ..
        } = read(&ReadKernelFileRequest {
            path: file.display().to_string(),
            offset: 4,
            length: 4,
        })
        .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (STANDARD.decode(data_base64).unwrap(), eof, size),
            (b"4567".to_vec(), false, 10)
        );
        let LocalDaemonResponse::KernelFileChunk { eof, .. } = read(&ReadKernelFileRequest {
            path: file.display().to_string(),
            offset: 8,
            length: 100,
        })
        .unwrap() else {
            panic!()
        };
        assert!(eof);
        let too_big = ReadKernelFileRequest {
            path: file.display().to_string(),
            offset: 0,
            length: KERNEL_FILE_MAX_CHUNK_BYTES + 1,
        };
        assert_eq!(code(read(&too_big).unwrap_err()), "file_chunk_too_large");
        let huge = chunk(&dir.join("h"), 0, b"", KERNEL_FILE_MAX_BYTES + 1, None);
        assert_eq!(code(write(&huge).unwrap_err()), "file_too_large");
        let missing = ReadKernelFileRequest {
            path: dir.join("nope").display().to_string(),
            offset: 0,
            length: 1,
        };
        assert_eq!(code(read(&missing).unwrap_err()), "file_not_found");
        let relative = ReadKernelFileRequest {
            path: "relative/file".into(),
            offset: 0,
            length: 1,
        };
        assert_eq!(code(read(&relative).unwrap_err()), "file_path_invalid");
        let directory = ReadKernelFileRequest {
            path: dir.display().to_string(),
            offset: 0,
            length: 1,
        };
        assert_eq!(code(read(&directory).unwrap_err()), "file_not_a_file");
    }

    // Chunks sent without waiting for each answer run on separate threads and may start in any order.
    #[test]
    fn a_chunk_that_starts_ahead_of_the_one_before_it_waits_for_it() {
        let dir = scratch();
        // The second target sits under folders that only the first chunk creates.
        for target in [dir.join("early.bin"), dir.join("new/nested/early.bin")] {
            let sha = format!("{:x}", Sha256::digest(b"abcdef"));
            let first = chunk(&target, 0, b"abc", 6, Some(&sha));
            let second = chunk(&target, 3, b"def", 6, Some(&sha));
            let late = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                write(&first)
            });
            let outcome = written(write(&second).unwrap());
            assert_eq!(outcome.0, 6);
            assert!(outcome.1);
            assert_eq!(written(late.join().unwrap().unwrap()).0, 3);
            assert_eq!(std::fs::read(&target).unwrap(), b"abcdef");
        }
    }

    // On a slow uplink the chunk ahead can land seconds after the one behind it.
    #[test]
    fn a_chunk_waits_for_the_one_ahead_of_it_when_that_lands_seconds_later() {
        let dir = scratch();
        let target = dir.join("late.bin");
        let sha = format!("{:x}", Sha256::digest(b"abcdef"));
        let first = chunk(&target, 0, b"abc", 6, Some(&sha));
        let second = chunk(&target, 3, b"def", 6, Some(&sha));
        let late = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(2500));
            write(&first)
        });
        let outcome = written(write(&second).unwrap());
        assert_eq!(outcome.0, 6);
        assert_eq!(written(late.join().unwrap().unwrap()).0, 3);
        assert_eq!(std::fs::read(&target).unwrap(), b"abcdef");
    }

    // A request of exactly the cap is served whole; one byte more is refused, for reads and writes.
    #[test]
    fn a_request_of_exactly_the_chunk_cap_is_served_and_one_byte_more_is_refused() {
        let cap = KERNEL_FILE_MAX_CHUNK_BYTES as usize;
        let dir = scratch();
        let file = dir.join("cap.bin");
        let bytes = vec![7u8; cap + 1];
        std::fs::write(&file, &bytes).unwrap();
        let LocalDaemonResponse::KernelFileChunk { data_base64, .. } =
            read(&ReadKernelFileRequest {
                path: file.display().to_string(),
                offset: 0,
                length: cap as u64,
            })
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(STANDARD.decode(data_base64).unwrap().len(), cap);
        let target = dir.join("up.bin");
        let total = bytes.len() as u64;
        assert_eq!(
            written(write(&chunk(&target, 0, &bytes[..cap], total, None)).unwrap()),
            (cap as u64, false, None)
        );
        assert_eq!(
            code(write(&chunk(&target, cap as u64, &bytes, total, None)).unwrap_err()),
            "file_chunk_too_large"
        );
    }

    // EA2-07 review: the staging name is predictable, so another user of a shared
    // directory can plant something at it before the upload starts.
    #[test]
    fn a_planted_staging_symlink_is_replaced_and_its_target_left_alone() {
        let dir = scratch();
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, b"private").unwrap();
        let target = dir.join("upload.bin");
        std::os::unix::fs::symlink(&victim, partial_of(&target)).unwrap();
        let bytes = b"fresh";
        let sha = format!("{:x}", Sha256::digest(bytes));
        write(&chunk(&target, 0, bytes, bytes.len() as u64, Some(&sha))).unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"private");
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
    }

    // EA2-45 review: another user can replace the staging name after the last chunk was written
    // and before it is hashed, permissioned and installed.
    #[test]
    fn a_staging_name_swapped_after_the_last_write_changes_nothing() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        for overwrite in [true, false] {
            let dir = scratch();
            let victim = dir.join("victim.txt");
            std::fs::write(&victim, b"private").unwrap();
            std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).unwrap();
            let target = dir.join("upload.bin");
            if overwrite {
                std::fs::write(&target, b"old").unwrap();
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
            let partial = partial_of(&target);
            let mut file = open_staging(&partial, &target, 0).unwrap();
            file.write_all(b"fresh").unwrap();
            std::fs::remove_file(&partial).unwrap();
            std::os::unix::fs::symlink(&victim, &partial).unwrap();
            let error = publish(&file, &partial, &target, overwrite, None).unwrap_err();
            assert_eq!(code(error), "file_staging_untrusted");
            assert_eq!(mode(&victim), 0o600);
            assert_eq!(std::fs::read(&victim).unwrap(), b"private");
            assert_eq!(target.exists(), overwrite);
            if overwrite {
                assert_eq!(std::fs::read(&target).unwrap(), b"old");
            }
        }
    }

    // Review (r12 f12 G1): a swap that wins the race between the name check and the rename must
    // not cost the file being replaced.
    #[test]
    fn a_swap_that_wins_the_race_to_replace_puts_the_original_back() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, b"private").unwrap();
        let target = dir.join("upload.bin");
        std::fs::write(&target, b"old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        let partial = partial_of(&target);
        let mut file = open_staging(&partial, &target, 0).unwrap();
        file.write_all(b"fresh").unwrap();
        std::fs::remove_file(&partial).unwrap();
        std::os::unix::fs::symlink(&victim, &partial).unwrap();
        let error = replace(&file, &partial, &target).unwrap_err();
        assert_eq!(code(error), "file_staging_untrusted");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(!std::fs::symlink_metadata(&target).unwrap().is_symlink());
        assert_eq!(std::fs::read(&victim).unwrap(), b"private");
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains(".chariox-previous")
            })
            .count();
        assert_eq!(leftovers, 0);
        // The unraced path installs the new bytes and leaves no backup behind.
        let partial = partial_of(&target);
        std::fs::remove_file(&partial).ok();
        let mut file = open_staging(&partial, &target, 0).unwrap();
        file.write_all(b"fresh").unwrap();
        replace(&file, &partial, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"fresh");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    }

    #[test]
    fn a_continued_upload_refuses_a_staging_file_that_is_not_the_private_one_it_made() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let target = dir.join("upload.bin");
        write(&chunk(&target, 0, b"abc", 6, None)).unwrap();
        let partial = partial_of(&target);
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = write(&chunk(&target, 3, b"def", 6, None)).unwrap_err();
        assert!(!error.to_string().contains(PARTIAL_SUFFIX));
        assert_eq!(code(error), "file_staging_untrusted");
        std::fs::remove_file(&partial).unwrap();
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, b"abc").unwrap();
        std::os::unix::fs::symlink(&victim, &partial).unwrap();
        let error = write(&chunk(&target, 3, b"def", 6, None)).unwrap_err();
        assert!(!error.to_string().contains(PARTIAL_SUFFIX));
        assert_eq!(code(error), "file_staging_untrusted");
        assert_eq!(std::fs::read(&victim).unwrap(), b"abc");
    }

    #[test]
    fn stat_hashes_on_request_and_a_listing_is_sorted() {
        let dir = scratch();
        std::fs::write(dir.join("b.txt"), b"abc").unwrap();
        std::fs::create_dir(dir.join("a-dir")).unwrap();
        let LocalDaemonResponse::KernelFileStat { stat } = stat(&StatKernelFileRequest {
            path: dir.join("b.txt").display().to_string(),
            with_sha256: true,
        })
        .unwrap() else {
            panic!()
        };
        assert_eq!(
            stat.sha256.as_deref(),
            Some(format!("{:x}", Sha256::digest(b"abc")).as_str())
        );
        let LocalDaemonResponse::KernelDirectoryListed {
            entries, truncated, ..
        } = list(&ListKernelDirectoryRequest {
            path: dir.display().to_string(),
            offset: 0,
        })
        .unwrap()
        else {
            panic!()
        };
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!((names, truncated), (vec!["a-dir", "b.txt"], false));
        assert_eq!(entries[0].kind, KernelFileKind::Directory);
    }

    // The window opens a link as what it points to; a transfer still sees the link (kind) and skips it.
    #[test]
    fn a_link_lists_as_a_link_that_says_what_it_points_to() {
        let dir = scratch();
        std::fs::write(dir.join("file.txt"), b"abc").unwrap();
        std::fs::create_dir(dir.join("folder")).unwrap();
        std::os::unix::fs::symlink(dir.join("folder"), dir.join("to-folder")).unwrap();
        std::os::unix::fs::symlink(dir.join("file.txt"), dir.join("to-file")).unwrap();
        std::os::unix::fs::symlink(dir.join("nothing"), dir.join("to-nothing")).unwrap();
        let LocalDaemonResponse::KernelDirectoryListed { entries, .. } =
            list(&ListKernelDirectoryRequest {
                path: dir.display().to_string(),
                offset: 0,
            })
            .unwrap()
        else {
            panic!()
        };
        let shown: Vec<_> = entries
            .iter()
            .map(|entry| {
                let json = serde_json::to_value(entry).unwrap();
                (
                    entry.name.clone(),
                    json["kind"].as_str().unwrap().to_owned(),
                    json["target_kind"].as_str().map(str::to_owned),
                )
            })
            .collect();
        let row = |name: &str, kind: &str, target: Option<&str>| {
            (name.to_owned(), kind.to_owned(), target.map(str::to_owned))
        };
        assert_eq!(
            shown,
            vec![
                row("file.txt", "file", None),
                row("folder", "directory", None),
                row("to-file", "symlink", Some("file")),
                row("to-folder", "symlink", Some("directory")),
                row("to-nothing", "symlink", None),
            ]
        );
    }
}
