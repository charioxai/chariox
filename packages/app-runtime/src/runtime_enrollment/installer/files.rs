//! Installer-owned descriptor operations. Caller supplies only fixed internal
//! names; signed payload paths have already passed the shared exact graph check.
use super::*;
use crate::private_fs::{self, Dir};
use std::{
    ffi::OsStr,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::AsRawFd,
    os::unix::fs::MetadataExt,
    path::Component,
};

pub(super) fn dir(path: &Path, uid: u32) -> Result<Dir> {
    Ok(Dir(filesystem::Directory::open(path, uid)?.file))
}
pub(super) fn child(parent: &Dir, name: &str, uid: u32) -> Result<Dir> {
    let child = parent.child(OsStr::new(name)).map_err(fs_error)?;
    require_directory(&child, uid)?;
    Ok(child)
}
fn require_directory(dir: &Dir, uid: u32) -> Result<()> {
    let metadata = dir.0.metadata()?;
    if metadata.uid() != uid || metadata.mode() & 0o7022 != 0 {
        return Err(EnrollmentError::Identity);
    }
    Ok(())
}
pub(super) fn create_or_open(parent: &Dir, name: &str, uid: u32) -> Result<Dir> {
    let created = match parent.create_child(OsStr::new(name)) {
        Ok(child) => {
            mode(&child.0, 0o755)?;
            child
        }
        Err(private_fs::FsError::Io(error))
            if error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            child(parent, name, uid)?
        }
        Err(error) => return Err(fs_error(error)),
    };
    require_directory(&created, uid)?;
    created.sync().map_err(fs_error)?;
    parent.sync().map_err(fs_error)?;
    Ok(created)
}
pub(super) fn mode(file: &File, mode: libc::mode_t) -> Result<()> {
    private_fs::check(unsafe { libc::fchmod(file.as_raw_fd(), mode) }).map_err(fs_error)
}
pub(super) fn lock(dir: &Dir, uid: u32) -> Result<File> {
    let name = OsStr::new(".runtime-installer.lock");
    let file = match dir.create_private_file(name) {
        Ok(file) => file,
        Err(private_fs::FsError::Io(error))
            if error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            dir.open_private_file(name).map_err(fs_error)?
        }
        Err(error) => return Err(fs_error(error)),
    };
    if file.metadata()?.uid() != uid || file.metadata()?.len() != 0 {
        return Err(EnrollmentError::Identity);
    }
    if !private_fs::try_lock_file(&file).map_err(fs_error)? {
        return Err(EnrollmentError::Busy);
    }
    file.sync_all()?;
    dir.sync().map_err(fs_error)?;
    Ok(file)
}
pub(super) fn exclusive(file: &File) -> Result<()> {
    if file.metadata()?.len() != 0 {
        return Err(EnrollmentError::Identity);
    }
    if private_fs::try_lock_file(file).map_err(fs_error)? {
        Ok(())
    } else {
        Err(EnrollmentError::Busy)
    }
}
pub(super) fn output(root: &Dir, path: &str, uid: u32) -> Result<File> {
    let mut directory = Dir(root.0.try_clone()?);
    let parts: Vec<_> = Path::new(path).components().collect();
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            return Err(EnrollmentError::Contract);
        };
        if index + 1 == parts.len() {
            return directory.create_private_file(name).map_err(fs_error);
        }
        let name = name.to_str().ok_or(EnrollmentError::Contract)?;
        directory = create_or_open(&directory, name, uid)?;
    }
    Err(EnrollmentError::Contract)
}
pub(super) fn copy(
    input: &mut File,
    output: &mut File,
    size: u64,
    digest: &str,
    executable: bool,
) -> Result<()> {
    if input.metadata()?.len() != size {
        return Err(EnrollmentError::Identity);
    }
    input.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0; 65536];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .ok_or(EnrollmentError::Limit)?;
        if copied > size {
            return Err(EnrollmentError::Limit);
        }
        output.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
    }
    if copied != size || format!("{:x}", hash.finalize()) != digest {
        return Err(EnrollmentError::Identity);
    }
    mode(output, if executable { 0o555 } else { 0o444 })?;
    output.sync_all()?;
    Ok(())
}
pub(super) fn write(root: &Dir, path: &str, bytes: &[u8], uid: u32) -> Result<()> {
    let mut output = output(root, path, uid)?;
    output.write_all(bytes)?;
    mode(&output, 0o444)?;
    output.sync_all()?;
    Ok(())
}
pub(super) fn rename(parent: &Dir, old: &str, new: &str, replace: bool) -> Result<()> {
    if !replace {
        return private_fs::publish(parent, OsStr::new(old), OsStr::new(new)).map_err(Into::into);
    }
    let old = private_fs::cstring(OsStr::new(old)).map_err(fs_error)?;
    let new = private_fs::cstring(OsStr::new(new)).map_err(fs_error)?;
    private_fs::check(unsafe {
        libc::renameat(
            parent.0.as_raw_fd(),
            old.as_ptr(),
            parent.0.as_raw_fd(),
            new.as_ptr(),
        )
    })
    .map_err(fs_error)
}
pub(super) fn remove_file(root: &Dir, name: &str, uid: u32) -> Result<()> {
    match filesystem::Directory::from_file(root.0.try_clone()?, uid).file(name, None) {
        Ok(_) => root.remove_file(OsStr::new(name)).map_err(fs_error),
        Err(EnrollmentError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
pub(super) fn fs_error(error: private_fs::FsError) -> EnrollmentError {
    match error {
        private_fs::FsError::Io(error) => EnrollmentError::Io(error),
        private_fs::FsError::UnsafeEntry => EnrollmentError::Identity,
        private_fs::FsError::EntryLimit => EnrollmentError::Limit,
    }
}
