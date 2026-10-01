//! Optional root-installed Linux local DEV broker. Absence preserves legacy
//! local Docker behavior; present but invalid enrollment fails closed.
#[cfg(target_os = "linux")]
use std::{
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

#[cfg(target_os = "linux")]
fn root_controlled(path: &Path) -> std::io::Result<()> {
    for entry in path.ancestors() {
        let metadata = fs::symlink_metadata(entry)?;
        if metadata.file_type().is_symlink() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0
        {
            return Err(std::io::Error::other(
                "local DEV broker public authority is invalid",
            ));
        }
        if entry == path
            && (!metadata.is_file() || metadata.nlink() != 1 || metadata.mode() & 0o111 == 0)
        {
            return Err(std::io::Error::other(
                "local DEV broker executable is invalid",
            ));
        }
        if entry != path && !metadata.is_dir() {
            return Err(std::io::Error::other(
                "local DEV broker ancestor is invalid",
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn start() -> Option<std::io::Result<PathBuf>> {
    let uid = unsafe { libc::geteuid() };
    let enrollment = PathBuf::from(format!("/etc/chariox/slice-local-dev/{uid}.json"));
    match fs::symlink_metadata(&enrollment) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => return Some(Err(error)),
        Ok(_) => {}
    }
    Some((|| {
        let launcher = PathBuf::from(format!("/usr/libexec/chariox-local-docker-broker-{uid}"));
        root_controlled(&launcher)?;
        // The root-installed launcher pins its immutable public Node runtime.
        // Do not select an ambient PATH runtime or duplicate the enrollment schema.
        let mut child = Command::new(launcher)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("broker transport unavailable"))?;
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = Vec::new();
            let result = (|| {
                for _ in 0..4096 {
                    let mut byte = [0];
                    stdout.read_exact(&mut byte)?;
                    if byte[0] == b'\n' {
                        return Ok(line);
                    }
                    line.push(byte[0]);
                }
                Err(std::io::Error::other("broker transport path exceeds limit"))
            })();
            let _ = send.send(result);
        });
        let result = receive
            .recv_timeout(Duration::from_secs(35))
            .map_err(|_| std::io::Error::other("local DEV broker startup deadline exceeded"))
            .and_then(|result| result)
            .and_then(|line| {
                String::from_utf8(line)
                    .map_err(|_| std::io::Error::other("broker transport path is invalid"))
            });
        match result {
            Ok(path)
                if path.starts_with(&format!("/tmp/chariox-local-broker-{uid}-"))
                    && path.ends_with("/control.sock")
                    && !path.contains("/../")
                    && !path.contains('\0') =>
            {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                Ok(PathBuf::from(path))
            }
            _ => {
                unsafe {
                    libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
                }
                use wait_timeout::ChildExt;
                if child
                    .wait_timeout(Duration::from_secs(5))
                    .ok()
                    .flatten()
                    .is_none()
                {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                Err(std::io::Error::other(
                    "verified local DEV broker startup refused",
                ))
            }
        }
    })())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn start() -> Option<std::io::Result<std::path::PathBuf>> {
    None
}
