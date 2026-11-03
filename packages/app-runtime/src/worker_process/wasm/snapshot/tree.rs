//! Quiesced migration copies, separate from bounded owner-export snapshots.
//! Walk held descriptors, never follow links, and stream file bytes. WASI can
//! create raw Unix names, symlinks and hard links; copy hard links independently
//! so a restored tree never shares writable inodes with its checkpoint.
use super::failed;
use crate::{
    private_fs::{self, Dir},
    worker_process::WorkerError,
};
use sha2::{Digest, Sha256};
use std::{
    ffi::{CStr, CString, OsStr},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
};

type Result<T> = std::result::Result<T, WorkerError>;

fn name(value: &OsStr) -> Result<CString> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.contains(&b'/') || bytes == b"." || bytes == b".." {
        return Err(failed());
    }
    CString::new(bytes).map_err(|_| failed())
}
fn check(value: libc::c_int) -> Result<()> {
    private_fs::check(value).map_err(|_| failed())
}
fn child(parent: &Dir, name: &CStr) -> Result<Dir> {
    private_fs::file(unsafe {
        libc::openat(
            parent.0.as_raw_fd(),
            name.as_ptr(),
            private_fs::directory_flags(),
        )
    })
    .map(Dir)
    .map_err(|_| failed())
}
fn metadata(parent: &Dir, name: &CStr) -> Result<libc::stat> {
    let mut stat = std::mem::MaybeUninit::uninit();
    check(unsafe {
        libc::fstatat(
            parent.0.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })?;
    Ok(unsafe { stat.assume_init() })
}
fn link(parent: &Dir, name: &CStr) -> Result<Vec<u8>> {
    let mut bytes = vec![0; 256];
    loop {
        let n = unsafe {
            libc::readlinkat(
                parent.0.as_raw_fd(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if n < 0 {
            return Err(failed());
        }
        if (n as usize) < bytes.len() {
            bytes.truncate(n as usize);
            return Ok(bytes);
        }
        bytes.resize(bytes.len().checked_mul(2).ok_or_else(failed)?, 0);
    }
}
fn hash_bytes(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

/// Includes names, entry types, empty directories, link targets and every file
/// byte. Validate before clearing the destination, and compare after replay.
pub(super) fn copy(from: &Dir, to: Option<&Dir>) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    walk(
        from,
        to,
        from.0.metadata().map_err(|_| failed())?.dev(),
        &mut hash,
    )?;
    Ok(hash.finalize().into())
}
struct Entries {
    name: Option<CString>,
    names: std::vec::IntoIter<std::ffi::OsString>,
}
fn entries(dir: &Dir, name: Option<CString>, hash: Option<&mut Sha256>) -> Result<Entries> {
    let mut names = dir.entries(usize::MAX).map_err(|_| failed())?;
    names.sort();
    if let Some(hash) = hash {
        hash.update((names.len() as u64).to_le_bytes());
    }
    Ok(Entries {
        name,
        names: names.into_iter(),
    })
}
fn clone(dir: &Dir) -> Result<Dir> {
    dir.0.try_clone().map(Dir).map_err(|_| failed())
}
fn parent(dir: &Dir) -> Result<Dir> {
    // Only traversal frames introduce '..'; guest-provided names reject it.
    child(dir, c"..")
}
fn walk(from: &Dir, to: Option<&Dir>, device: u64, hash: &mut Sha256) -> Result<()> {
    let mut from = clone(from)?;
    let mut to = to.map(clone).transpose()?;
    let mut stack = vec![entries(&from, None, Some(hash))?];
    while let Some(frame) = stack.last_mut() {
        let Some(raw) = frame.names.next() else {
            if let Some(to) = &to {
                to.sync().map_err(|_| failed())?;
            }
            stack.pop();
            if !stack.is_empty() {
                from = parent(&from)?;
                to = to.as_ref().map(parent).transpose()?;
            }
            continue;
        };
        let name = name(&raw)?;
        let stat = metadata(&from, &name)?;
        if stat.st_dev as u64 != device {
            return Err(failed());
        }
        hash_bytes(hash, raw.as_bytes());
        let kind = stat.st_mode & libc::S_IFMT;
        hash.update((kind as u32).to_le_bytes());
        match kind {
            libc::S_IFDIR => {
                let source = child(&from, &name)?;
                let target = if let Some(to) = &to {
                    check(unsafe { libc::mkdirat(to.0.as_raw_fd(), name.as_ptr(), 0o700) })?;
                    Some(child(to, &name)?)
                } else {
                    None
                };
                stack.push(entries(&source, Some(name), Some(hash))?);
                from = source;
                to = target;
            }
            libc::S_IFLNK => {
                let bytes = link(&from, &name)?;
                hash_bytes(hash, &bytes);
                if let Some(to) = &to {
                    let target = CString::new(bytes).map_err(|_| failed())?;
                    check(unsafe {
                        libc::symlinkat(target.as_ptr(), to.0.as_raw_fd(), name.as_ptr())
                    })?;
                }
            }
            libc::S_IFREG => {
                let mut input = private_fs::file(unsafe {
                    libc::openat(
                        from.0.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                    )
                })
                .map_err(|_| failed())?;
                let actual = input.metadata().map_err(|_| failed())?;
                if !actual.is_file() || actual.dev() != device || actual.ino() != stat.st_ino as u64
                {
                    return Err(failed());
                }
                hash.update(actual.len().to_le_bytes());
                let mut output = if let Some(to) = &to {
                    Some(
                        private_fs::file(unsafe {
                            libc::openat(
                                to.0.as_raw_fd(),
                                name.as_ptr(),
                                libc::O_WRONLY
                                    | libc::O_CREAT
                                    | libc::O_EXCL
                                    | libc::O_NOFOLLOW
                                    | libc::O_CLOEXEC,
                                0o600,
                            )
                        })
                        .map_err(|_| failed())?,
                    )
                } else {
                    None
                };
                let mut buffer = vec![0; 64 * 1024];
                let mut bytes = 0u64;
                loop {
                    let n = input.read(&mut buffer).map_err(|_| failed())?;
                    if n == 0 {
                        break;
                    }
                    bytes = bytes.checked_add(n as u64).ok_or_else(failed)?;
                    hash.update(&buffer[..n]);
                    if let Some(output) = &mut output {
                        output.write_all(&buffer[..n]).map_err(|_| failed())?;
                    }
                }
                if bytes != actual.len() {
                    return Err(failed());
                }
                if let Some(output) = output {
                    output.sync_all().map_err(|_| failed())?;
                }
            }
            _ => return Err(failed()),
        }
    }
    Ok(())
}

/// Guest data has no entry/depth quota yet. Cleanup must not impose a smaller
/// limit than copying, including partial publication and interrupted replay.
pub(in crate::worker_process::wasm) fn clear(dir: &Dir) -> Result<()> {
    let device = dir.0.metadata().map_err(|_| failed())?.dev();
    clear_on_device(dir, device)
}
fn clear_on_device(dir: &Dir, device: u64) -> Result<()> {
    let mut dir = clone(dir)?;
    let mut stack = vec![entries(&dir, None, None)?];
    while let Some(frame) = stack.last_mut() {
        let Some(raw) = frame.names.next() else {
            dir.sync().map_err(|_| failed())?;
            let frame = stack.pop().ok_or_else(failed)?;
            if let Some(name) = frame.name {
                let up = parent(&dir)?;
                let expected = dir.0.metadata().map_err(|_| failed())?;
                let actual = metadata(&up, &name)?;
                if actual.st_dev as u64 != expected.dev() || actual.st_ino as u64 != expected.ino()
                {
                    return Err(failed());
                }
                check(unsafe {
                    libc::unlinkat(up.0.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR)
                })?;
                dir = up;
            }
            continue;
        };
        let name = name(&raw)?;
        let stat = metadata(&dir, &name)?;
        if stat.st_dev as u64 != device {
            return Err(failed());
        }
        if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
            let next = child(&dir, &name)?;
            stack.push(entries(&next, Some(name), None)?);
            dir = next;
        } else {
            check(unsafe { libc::unlinkat(dir.0.as_raw_fd(), name.as_ptr(), 0) })?;
        }
    }
    Ok(())
}
