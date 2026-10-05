//! Unix websocket endpoint. Admission comes from OS peer credentials; no tokens.
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::PathBuf;
use tokio::net::UnixListener;

pub(crate) struct LocalIpcListener {
    pub(crate) listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
}

impl LocalIpcListener {
    pub(crate) fn bind(path: PathBuf) -> io::Result<Self> {
        // Fail before creating/chmodding directories or removing stale sockets.
        crate::config::validate_local_socket_path(&path)?;
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("socket has no directory"))?;
        if let Some(root) = parent.parent() {
            fs::create_dir_all(root)?;
        }
        match fs::DirBuilder::new().mode(0o700).create(parent) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let metadata = fs::symlink_metadata(parent)?;
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::other(
                "socket directory must be owned by the kernel user and not a symlink",
            ));
        }
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() }
                {
                    return Err(io::Error::other(
                        "refusing to remove an unowned or non-socket IPC path",
                    ));
                }
                match std::os::unix::net::UnixStream::connect(&path) {
                    Ok(_) => return Err(io::Error::other("local socket is already in use")),
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                        ) => {}
                    Err(e) => return Err(e),
                }
                fs::remove_file(&path)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let metadata = fs::symlink_metadata(&path)?;
        Ok(Self {
            listener,
            path,
            identity: (metadata.dev(), metadata.ino()),
        })
    }
}

impl Drop for LocalIpcListener {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests;
