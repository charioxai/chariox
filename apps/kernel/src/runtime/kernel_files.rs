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
    let mut file = std::fs::File::open(path).map_err(|e| io_failure(path, &e))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| io_failure(path, &e))?;
        if read == 0 {
            return Ok(format!("{:x}", hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
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
    let mut file = if request.offset == 0 {
        std::fs::File::create(&partial).map_err(|e| io_failure(&partial, &e))?
    } else {
        let have = std::fs::metadata(&partial).map_or(0, |m| m.len());
        if have != request.offset {
            return Err(fail(
                "file_offset_mismatch",
                format!(
                    "{} has {have} bytes of this upload; continue at offset {have}.",
                    path.display()
                ),
            ));
        }
        std::fs::OpenOptions::new()
            .append(true)
            .open(&partial)
            .map_err(|e| io_failure(&partial, &e))?
    };
    file.write_all(&data)
        .map_err(|e| io_failure(&partial, &e))?;
    file.flush().map_err(|e| io_failure(&partial, &e))?;
    drop(file);
    if end < request.total_size {
        return Ok(LocalDaemonResponse::KernelFileWritten {
            path: path.display().to_string(),
            received: end,
            complete: false,
            sha256: None,
        });
    }
    let sha256 = sha256_of(&partial)?;
    if let Some(expected) = request.sha256.as_deref() {
        if !expected.eq_ignore_ascii_case(&sha256) {
            let _ = std::fs::remove_file(&partial);
            return Err(fail(
                "file_hash_mismatch",
                format!("The received file hashes to {sha256}, not {expected}; nothing was kept. Upload it again."),
            ));
        }
    }
    if request.overwrite {
        // Replacing a file keeps its permissions (a 0600 credentials file stays private).
        if let Ok(existing) = std::fs::metadata(&path) {
            std::fs::set_permissions(&partial, existing.permissions())
                .map_err(|e| io_failure(&partial, &e))?;
        }
        std::fs::rename(&partial, &path).map_err(|e| io_failure(&path, &e))?;
    } else {
        // A hard link fails when something appeared at `path` meanwhile; a rename would replace it.
        std::fs::hard_link(&partial, &path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                fail(
                    "file_exists",
                    format!(
                        "{} already exists; send overwrite to replace it.",
                        path.display()
                    ),
                )
            } else {
                io_failure(&path, &error)
            }
        })?;
        std::fs::remove_file(&partial).map_err(|e| io_failure(&partial, &e))?;
    }
    Ok(LocalDaemonResponse::KernelFileWritten {
        path: path.display().to_string(),
        received: end,
        complete: true,
        sha256: Some(sha256),
    })
}

#[cfg(test)]
mod tests {
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
