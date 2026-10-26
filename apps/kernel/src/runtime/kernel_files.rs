//! File access on this machine for the kernel file requests (External Agents v2).
//!
//! The kernel runs as an OS user and can touch what that user can: these
//! requests add no power over a terminal on the kernel, only a transfer that
//! is chunked, resumable and checked by sha256. Paths are absolute or `~/`.
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::local::{
    KernelDirectoryEntry, KernelFileKind, KernelFileStat, ListKernelDirectoryRequest,
    LocalDaemonResponse, ReadKernelFileRequest, StatKernelFileRequest, WriteKernelFileRequest,
    KERNEL_FILE_MAX_BYTES, KERNEL_FILE_MAX_CHUNK_BYTES,
};

/// Directory listings stop here and say so.
const MAX_DIRECTORY_ENTRIES: usize = 2000;
const PARTIAL_SUFFIX: &str = ".chariox-partial";

/// Codes whose message this module writes itself, so the grant holder reads them in full.
pub(crate) fn is_public_code(code: &str) -> bool {
    matches!(
        code,
        "file_path_invalid"
            | "file_not_found"
            | "file_permission_denied"
            | "file_not_a_file"
            | "file_too_large"
            | "file_chunk_too_large"
            | "file_offset_mismatch"
            | "file_hash_mismatch"
            | "file_exists"
            | "file_io_failed"
            | "file_staging_untrusted"
    )
}

fn fail(code: &str, message: impl Into<String>) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.into(),
        message: message.into(),
    }
}

fn io_failure(path: &Path, error: &std::io::Error) -> DaemonError {
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

/// `~/x` becomes the kernel user's home; anything else must be absolute.
fn resolve(path: &str) -> Result<PathBuf, DaemonError> {
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
    if resolved.is_absolute() {
        Ok(resolved)
    } else {
        Err(fail(
            "file_path_invalid",
            format!("\"{path}\" is not an absolute path; start it with / or ~/."),
        ))
    }
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
fn sha256_of_open(file: &std::fs::File, path: &Path) -> Result<String, DaemonError> {
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

pub(crate) fn stat(request: &StatKernelFileRequest) -> Result<LocalDaemonResponse, DaemonError> {
    let path = resolve(&request.path)?;
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
            sha256: None,
            partial_size,
        },
        Err(error) => return Err(io_failure(&path, &error)),
    };
    Ok(LocalDaemonResponse::KernelFileStat { stat })
}

pub(crate) fn list(
    request: &ListKernelDirectoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let path = resolve(&request.path)?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in std::fs::read_dir(&path).map_err(|e| io_failure(&path, &e))? {
        let entry = entry.map_err(|e| io_failure(&path, &e))?;
        if entries.len() == MAX_DIRECTORY_ENTRIES {
            truncated = true;
            break;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        entries.push(KernelDirectoryEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            kind: kind_of(metadata.file_type()),
            size: metadata.len(),
            modified_ms: modified_ms(&metadata),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(LocalDaemonResponse::KernelDirectoryListed {
        path: path.display().to_string(),
        entries,
        truncated,
    })
}

fn too_large(size: u64) -> DaemonError {
    fail(
        "file_too_large",
        format!("A file may be at most {KERNEL_FILE_MAX_BYTES} bytes; this one is {size}."),
    )
}

pub(crate) fn read(request: &ReadKernelFileRequest) -> Result<LocalDaemonResponse, DaemonError> {
    if request.length > KERNEL_FILE_MAX_CHUNK_BYTES {
        return Err(fail(
            "file_chunk_too_large",
            format!("Read at most {KERNEL_FILE_MAX_CHUNK_BYTES} bytes per request."),
        ));
    }
    let path = resolve(&request.path)?;
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

/// Opens the upload's staging file without trusting what is already at the predictable name:
/// a first chunk replaces whatever is there, a later chunk continues only the private regular
/// file this user made (another user of a shared directory may have planted a link or a file).
fn open_staging(partial: &Path, path: &Path, offset: u64) -> Result<std::fs::File, DaemonError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let untrusted = || {
        fail(
            "file_staging_untrusted",
            format!(
                "{} is not the private file this upload made; remove it, then upload again from offset 0.",
                partial.display()
            ),
        )
    };
    if offset == 0 {
        match std::fs::remove_file(partial) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_failure(partial, &e)),
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
            .map_err(|e| io_failure(partial, &e));
    }
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(partial)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(offset_mismatch(path, 0));
        }
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Err(untrusted()),
        Err(e) => return Err(io_failure(partial, &e)),
    };
    let metadata = file.metadata().map_err(|e| io_failure(partial, &e))?;
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

fn offset_mismatch(path: &Path, have: u64) -> DaemonError {
    fail(
        "file_offset_mismatch",
        format!(
            "{} has {have} bytes of this upload; continue at offset {have}.",
            path.display()
        ),
    )
}

pub(crate) fn write(request: &WriteKernelFileRequest) -> Result<LocalDaemonResponse, DaemonError> {
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
    let path = resolve(&request.path)?;
    let partial = partial_of(&path);
    let existing = std::fs::symlink_metadata(&path);
    if existing.is_ok() && !request.overwrite {
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
    let end = request.offset.saturating_add(data.len() as u64);
    if end > request.total_size {
        return Err(fail(
            "file_offset_mismatch",
            format!(
                "The chunk ends at {end}, past total_size {}.",
                request.total_size
            ),
        ));
    }
    let mut file = open_staging(&partial, &path, request.offset)?;
    file.write_all(&data)
        .map_err(|e| io_failure(&partial, &e))?;
    file.flush().map_err(|e| io_failure(&partial, &e))?;
    if end < request.total_size {
        return Ok(LocalDaemonResponse::KernelFileWritten {
            path: path.display().to_string(),
            received: end,
            complete: false,
            sha256: None,
        });
    }
    let sha256 = publish(
        &file,
        &partial,
        &path,
        request.overwrite,
        request.sha256.as_deref(),
    )?;
    Ok(LocalDaemonResponse::KernelFileWritten {
        path: path.display().to_string(),
        received: end,
        complete: true,
        sha256: Some(sha256),
    })
}

fn staging_replaced(partial: &Path) -> DaemonError {
    fail(
        "file_staging_untrusted",
        format!(
            "{} was replaced while the upload finished; nothing was kept. Remove it, then upload again.",
            partial.display()
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
        return Err(io_failure(path, &error));
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
    Err(staging_replaced(partial))
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
    let sha256 = sha256_of_open(file, partial)?;
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
        return Err(staging_replaced(partial));
    }
    if overwrite {
        // Replacing a file keeps its permissions (a 0600 credentials file stays private).
        if let Ok(existing) = std::fs::metadata(path) {
            file.set_permissions(existing.permissions())
                .map_err(|e| io_failure(partial, &e))?;
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
                io_failure(path, &error)
            }
        })?;
        if !names_the_open_file(path, file) {
            let _ = std::fs::remove_file(path);
            return Err(staging_replaced(partial));
        }
        let _ = std::fs::remove_file(partial);
    }
    Ok(sha256)
}

#[cfg(test)]
mod tests {
    // EA2-45: the refusal names the staging file and what to do, so the agent must read it.
    #[test]
    fn every_code_this_module_raises_with_its_own_message_is_public() {
        for code in [
            "file_path_invalid",
            "file_not_found",
            "file_permission_denied",
            "file_not_a_file",
            "file_too_large",
            "file_chunk_too_large",
            "file_offset_mismatch",
            "file_hash_mismatch",
            "file_exists",
            "file_io_failed",
            "file_staging_untrusted",
        ] {
            assert!(is_public_code(code), "{code}");
        }
        assert!(!is_public_code("kernel_request_failed"));
    }

    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kernel-files-{:08x}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
        assert_eq!(code(error), "file_staging_untrusted");
        std::fs::remove_file(&partial).unwrap();
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, b"abc").unwrap();
        std::os::unix::fs::symlink(&victim, &partial).unwrap();
        let error = write(&chunk(&target, 3, b"def", 6, None)).unwrap_err();
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
        })
        .unwrap()
        else {
            panic!()
        };
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!((names, truncated), (vec!["a-dir", "b.txt"], false));
        assert_eq!(entries[0].kind, KernelFileKind::Directory);
    }
}
