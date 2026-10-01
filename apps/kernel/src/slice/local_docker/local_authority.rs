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
    let launcher = PathBuf::from(format!("/usr/libexec/chariox-local-docker-broker-{uid}"));
    Some(root_controlled(&launcher).and_then(|()| launch(&launcher, uid)))
}

// The root-installed launcher pins its immutable public Node runtime.
// Do not select an ambient PATH runtime or duplicate the enrollment schema.
#[cfg(target_os = "linux")]
fn launch(launcher: &Path, uid: u32) -> std::io::Result<PathBuf> {
    let mut child = Command::new(launcher)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(std::io::Error::other("broker transport unavailable"));
    };
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
    // A refusal names its reason on stderr; drain the rest so the launcher
    // never blocks on it.
    let (refusal_send, refusal_receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut refusal = Vec::new();
        let _ = (&mut stderr).take(1024).read_to_end(&mut refusal);
        let _ = refusal_send.send(refusal);
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
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
            let refusal = refusal_receive
                .recv_timeout(Duration::from_secs(1))
                .ok()
                .and_then(|refusal| refusal_reason(&refusal));
            Err(std::io::Error::other(refusal.unwrap_or_else(|| {
                "verified local DEV broker startup refused".to_string()
            })))
        }
    }
}

#[cfg(target_os = "linux")]
fn refusal_reason(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(
        line.chars()
            .filter(|character| !character.is_control())
            .take(512)
            .collect(),
    )
}

#[cfg(not(target_os = "linux"))]
pub(super) fn start() -> Option<std::io::Result<std::path::PathBuf>> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn run_launcher(name: &str, script: &str) -> std::io::Result<PathBuf> {
        let directory = std::env::temp_dir().join(format!(
            "chariox-local-broker-launch-{}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("fixture directory should exist");
        let launcher = directory.join("launcher");
        fs::write(&launcher, script).expect("fixture launcher should exist");
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755))
            .expect("fixture launcher should be executable");
        // Another test may fork while the fresh script is still open for writing.
        let mut result = launch(&launcher, 4242);
        for _ in 0..50 {
            if result.as_ref().err().and_then(std::io::Error::raw_os_error) != Some(libc::ETXTBSY) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
            result = launch(&launcher, 4242);
        }
        let _ = fs::remove_dir_all(directory);
        result
    }

    #[test]
    fn a_launcher_refusal_reaches_the_kernel_with_its_reason() {
        let reason = "Verified local Docker DEV broker startup refused: transport refused: socket owner 0 mode 0755 after 30000 ms";
        let error = run_launcher(
            "refused",
            &format!("#!/bin/sh\necho '{reason}' >&2\nexit 1\n"),
        )
        .expect_err("a refused launch must fail");
        assert_eq!(error.to_string(), reason);
    }

    #[test]
    fn a_silent_launcher_failure_keeps_the_generic_refusal() {
        let error =
            run_launcher("silent", "#!/bin/sh\nexit 1\n").expect_err("a failed launch must fail");
        assert_eq!(
            error.to_string(),
            "verified local DEV broker startup refused"
        );
    }

    #[test]
    fn a_published_transport_path_is_returned() {
        let path = run_launcher(
            "published",
            "#!/bin/sh\necho /tmp/chariox-local-broker-4242-abc/control.sock\n",
        )
        .expect("a published transport should be returned");
        assert_eq!(
            path,
            PathBuf::from("/tmp/chariox-local-broker-4242-abc/control.sock")
        );
    }
}
