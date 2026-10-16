//! MP-08 / MP-10 / MP-11: one contained, private path builder for catalog and execution caches.
use super::catalog::failure;
use crate::error::DaemonError;
use std::{
    fs,
    path::{Path, PathBuf},
};
pub(super) fn owned_directory(home: &Path, relative: &Path) -> Result<PathBuf, DaemonError> {
    contained_directory(home, relative, true)
}

/// MP-08 / MP-10 / MP-11: cache probes must not publish an empty immutable
/// digest directory. A miss is read-only and remains eligible for retransmit.
pub(super) fn existing_owned_directory(
    home: &Path,
    relative: &Path,
) -> Result<PathBuf, DaemonError> {
    contained_directory(home, relative, false)
}

fn contained_directory(home: &Path, relative: &Path, create: bool) -> Result<PathBuf, DaemonError> {
    if !home.is_absolute() {
        return Err(failure("Skills require a resolved absolute Chariox home"));
    }
    let mut path = fs::canonicalize(home).map_err(|_| failure("cannot resolve Chariox home"))?;
    for part in relative.components() {
        let std::path::Component::Normal(part) = part else {
            return Err(failure("invalid catalog path"));
        };
        path.push(part);
        if create {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(failure("cannot create catalog directory")),
            }
        }
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| failure("cannot inspect catalog directory"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(failure(
                "catalog directory must be owned state, not a symlink",
            ));
        }
    }
    Ok(path)
}
