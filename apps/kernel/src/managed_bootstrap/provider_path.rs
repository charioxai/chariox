use std::ffi::OsString;
use std::path::Path;

#[cfg(unix)]
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::io::{self, Read};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::process::Child;
#[cfg(unix)]
use std::time::{Duration, Instant};

const BOOTSTRAP_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const PATH_PROBE_COMMAND: &str = "printf '\\000CHARIOX_PROVIDER_PATH_V1\\000%s\\000' \"${PATH-}\"";
const PATH_PROBE_MARKER: &[u8] = b"\0CHARIOX_PROVIDER_PATH_V1\0";
const MAX_PROVIDER_PATH_BYTES: usize = 4096;
#[cfg(unix)]
const MAX_PROBE_OUTPUT_BYTES: usize = 16 * 1024;
#[cfg(unix)]
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve the user's login PATH in a short-lived, isolated Bash process.
/// Only the validated PATH string crosses back into the bootstrap process.
pub(super) fn resolve_login_path(home: &Path) -> OsString {
    resolve_login_path_with_fallback(home, false)
}

/// A disposable worker keeps its local provider tools available if its login
/// profile cannot be probed, while still excluding every profile-supplied path.
pub(super) fn resolve_worker_login_path(home: &Path) -> OsString {
    resolve_login_path_with_fallback(home, true)
}

fn resolve_login_path_with_fallback(home: &Path, include_home_local_bin: bool) -> OsString {
    match probe_login_path(home) {
        Ok(path) => path,
        Err(reason) => {
            let fallback_path = fallback_path(home, include_home_local_bin);
            crate::logging::warn_with_fields(
                "managed_bootstrap.provider_path_probe_failed",
                "managed provider login PATH probe failed; using a safe fallback PATH",
                serde_json::json!({
                    "reason": reason,
                    "fallback_path": fallback_path.as_str(),
                }),
            );
            OsString::from(fallback_path)
        }
    }
}

fn fallback_path(home: &Path, include_home_local_bin: bool) -> String {
    if include_home_local_bin {
        let local_bin = home.join(".local/bin");
        if let Some(local_bin) = local_bin
            .to_str()
            .filter(|component| {
                is_safe_path_component(component)
                    && component.len() + 1 + BOOTSTRAP_PATH.len() <= MAX_PROVIDER_PATH_BYTES
            })
        {
            return format!("{local_bin}:{BOOTSTRAP_PATH}");
        }
    }
    BOOTSTRAP_PATH.to_string()
}

#[cfg(unix)]
fn probe_login_path(home: &Path) -> Result<OsString, &'static str> {
    let mut command = Command::new("/bin/bash");
    command
        .arg("--login")
        .arg("-c")
        .arg(PATH_PROBE_COMMAND)
        .current_dir("/")
        .env_clear()
        .env("HOME", home)
        .env("USER", "chariox")
        .env("LOGNAME", "chariox")
        .env("SHELL", "/bin/bash")
        .env("PATH", BOOTSTRAP_PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command.process_group(0);

    let mut child = command
        .spawn()
        .map_err(|_| "could not start the login PATH probe")?;
    let child_pid = child.id();
    let mut stdout = child
        .stdout
        .take()
        .ok_or("login PATH probe stdout was unavailable")?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let mut status = None;
    let mut stdout_closed = false;
    let mut output = Vec::with_capacity(1024);

    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(observed) => status = observed,
                Err(_) => {
                    stop_probe(&mut child, child_pid);
                    return Err("could not wait for the login PATH probe");
                }
            }
        }
        if status.is_some() && stdout_closed {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            stop_probe(&mut child, child_pid);
            return Err("login PATH probe timed out");
        }

        let mut descriptor = libc::pollfd {
            fd: stdout.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        };
        let remaining_millis = deadline
            .saturating_duration_since(now)
            .as_millis()
            .clamp(1, 50) as i32;
        let poll_result = unsafe { libc::poll(&mut descriptor, 1, remaining_millis) };
        if poll_result < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            stop_probe(&mut child, child_pid);
            return Err("could not read the login PATH probe output");
        }
        if poll_result == 0 {
            continue;
        }
        if descriptor.revents
            & (libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)
            == 0
        {
            continue;
        }

        let remaining = MAX_PROBE_OUTPUT_BYTES + 1 - output.len();
        let mut chunk = vec![0; remaining.min(1024)];
        match stdout.read(&mut chunk) {
            Ok(0) => stdout_closed = true,
            Ok(read) => {
                output.extend_from_slice(&chunk[..read]);
                if output.len() > MAX_PROBE_OUTPUT_BYTES {
                    stop_probe(&mut child, child_pid);
                    return Err("login PATH probe output exceeded its limit");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => {
                stop_probe(&mut child, child_pid);
                return Err("could not read the login PATH probe output");
            }
        }
    }

    stop_process_group(child_pid);
    if !status.is_some_and(|status| status.success()) {
        return Err("login PATH probe failed");
    }
    parse_login_path(&output).map(OsString::from)
}

#[cfg(not(unix))]
fn probe_login_path(_home: &Path) -> Result<OsString, &'static str> {
    Err("login PATH probing requires a Unix platform")
}

fn parse_login_path(output: &[u8]) -> Result<String, &'static str> {
    let marker_start = output
        .windows(PATH_PROBE_MARKER.len())
        .rposition(|window| window == PATH_PROBE_MARKER)
        .ok_or("login profile did not produce the PATH probe marker")?;
    let framed = output[marker_start + PATH_PROBE_MARKER.len()..]
        .strip_suffix(b"\0")
        .ok_or("login profile emitted unexpected PATH probe output")?;
    if framed.is_empty()
        || framed.len() > MAX_PROVIDER_PATH_BYTES
        || framed.contains(&0)
    {
        return Err("login profile returned an invalid PATH");
    }
    let path = std::str::from_utf8(framed).map_err(|_| "login profile PATH is not UTF-8")?;
    let safe_components = path
        .split(':')
        .filter(|component| is_safe_path_component(component))
        .collect::<Vec<_>>();
    if safe_components.is_empty() {
        return Err("login profile returned no safe PATH entries");
    }
    Ok(safe_components.join(":"))
}

fn is_safe_path_component(component: &str) -> bool {
    !component.is_empty()
        && !component.contains(':')
        && Path::new(component).is_absolute()
        && !component.bytes().any(|byte| byte.is_ascii_control())
}

#[cfg(unix)]
fn stop_probe(child: &mut Child, pid: u32) {
    stop_process_group(pid);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn stop_process_group(pid: u32) {
    if let Ok(pid) = i32::try_from(pid) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestHome(PathBuf);

    impl TestHome {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "chariox-provider-path-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("test home should be created");
            Self(path)
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn login_profile_only_contributes_a_validated_provider_path() {
        let home = TestHome::new();
        let local_bin = home.0.join(".local/bin");
        fs::create_dir_all(&local_bin).expect("provider bin should be created");
        let expected_path = format!("{}:{BOOTSTRAP_PATH}", local_bin.display());
        fs::write(
            home.0.join(".profile"),
            format!(
                "test -z \"${{CHARIOX_MANAGED_RELEASE_PUBLIC_KEY-}}\" || exit 11\n\
                 test -z \"${{CHARIOX_MANAGED_KERNEL_BINARY-}}\" || exit 12\n\
                 test -z \"${{CHARIOX_MANAGED_BOOTSTRAP_PATH-}}\" || exit 13\n\
                 test -z \"${{CHARIOX_MANAGED_PROVIDER_TOPOLOGY-}}\" || exit 14\n\
                 test -z \"${{CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY-}}\" || exit 15\n\
                 test -z \"${{LD_PRELOAD-}}\" || exit 16\n\
                 export CHARIOX_MANAGED_RELEASE_PUBLIC_KEY=/profile/release-key\n\
                 export CHARIOX_MANAGED_KERNEL_BINARY=/profile/kernel\n\
                 export CHARIOX_MANAGED_BOOTSTRAP_PATH=/profile/bootstrap.json\n\
                 export CHARIOX_MANAGED_PROVIDER_TOPOLOGY=shared_host\n\
                 export CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY=/profile/builder-key\n\
                 export LD_PRELOAD=/profile/hostile.so\n\
                 export PATH='{}'\n",
                expected_path
            ),
        )
        .expect("login profile should be written");
        let tool = local_bin.join("chariox-provider-path-probe");
        fs::write(&tool, "#!/bin/sh\nprintf 'ordinary-provider-tool\\n'\n")
            .expect("provider probe should be written");
        let mut permissions = fs::metadata(&tool)
            .expect("provider probe should exist")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&tool, permissions).expect("provider probe should be executable");

        let supervisor_env = [
            "CHARIOX_MANAGED_RELEASE_PUBLIC_KEY",
            "CHARIOX_MANAGED_KERNEL_BINARY",
            "CHARIOX_MANAGED_BOOTSTRAP_PATH",
            "CHARIOX_MANAGED_PROVIDER_TOPOLOGY",
            "CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY",
            "LD_PRELOAD",
        ]
        .map(|name| (name, std::env::var_os(name)));
        let path = resolve_login_path(&home.0);
        assert_eq!(path, OsString::from(&expected_path));
        for (name, expected) in supervisor_env {
            assert_eq!(std::env::var_os(name), expected, "supervisor env {name} changed");
        }

        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg("chariox-provider-path-probe")
            .env_clear()
            .env("HOME", &home.0)
            .env("PATH", &path)
            .output()
            .expect("provider tool should resolve from the captured PATH");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"ordinary-provider-tool\n");
    }

    #[test]
    fn worker_login_path_probe_timeout_uses_home_bin_and_kills_background_stdout_writer() {
        let home = TestHome::new();
        fs::write(
            home.0.join(".profile"),
            concat!(
                "/bin/sh -c 'i=0; while [ \"$i\" -lt 1000 ]; do ",
                "printf x || :; printf x >> \"$HOME/writer-heartbeat\"; ",
                "/bin/sleep 0.02; i=$((i + 1)); done' &\n",
                "export PATH='/profile/never'\n",
            ),
        )
        .expect("login profile should be written");

        let expected_path = format!("{}:{BOOTSTRAP_PATH}", home.0.join(".local/bin").display());
        let started = Instant::now();
        let path = resolve_worker_login_path(&home.0);
        assert_eq!(path, OsString::from(&expected_path));
        let elapsed = started.elapsed();
        assert!(elapsed >= PROBE_TIMEOUT);
        assert!(elapsed < PROBE_TIMEOUT + Duration::from_secs(3));

        let heartbeat = home.0.join("writer-heartbeat");
        let before = fs::read(&heartbeat).expect("background writer should have started");
        assert!(!before.is_empty());
        std::thread::sleep(Duration::from_millis(200));
        let after = fs::read(&heartbeat)
            .expect("background writer heartbeat should remain readable");
        assert_eq!(after, before, "background writer survived probe cleanup");
    }

    #[test]
    fn login_path_probe_uses_bootstrap_path_for_oversized_output() {
        let home = TestHome::new();
        fs::write(
            home.0.join(".profile"),
            format!(
                "head -c {} /dev/zero | tr '\\000' x\n",
                MAX_PROBE_OUTPUT_BYTES + 1
            ),
        )
        .expect("login profile should be written");

        let path = resolve_login_path(&home.0);
        assert_eq!(path, OsString::from(BOOTSTRAP_PATH));
    }

    #[test]
    fn worker_fallback_only_adds_an_absolute_safe_home_bin() {
        let home = Path::new("/srv/worker home");
        assert_eq!(
            fallback_path(home, true),
            format!("{}:{BOOTSTRAP_PATH}", home.join(".local/bin").display())
        );
        assert_eq!(fallback_path(Path::new("relative/home"), true), BOOTSTRAP_PATH);
        assert_eq!(
            fallback_path(Path::new("/srv/worker:home"), true),
            BOOTSTRAP_PATH
        );
        assert_eq!(
            fallback_path(Path::new("/srv/worker\nhome"), true),
            BOOTSTRAP_PATH
        );
        assert_eq!(fallback_path(home, false), BOOTSTRAP_PATH);
    }

    #[test]
    fn login_path_probe_uses_bootstrap_path_when_profile_exits() {
        let home = TestHome::new();
        fs::write(home.0.join(".profile"), "exit 23\n")
            .expect("failing login profile should be written");

        assert_eq!(
            resolve_login_path(&home.0),
            OsString::from(BOOTSTRAP_PATH)
        );
    }

    #[test]
    fn login_path_accepts_profile_output_before_the_probe_frame() {
        assert_eq!(
            parse_login_path(b"\0CHARIOX_PROVIDER_PATH_V1\0relative:/bin::/usr/bin:\0"),
            Ok("/bin:/usr/bin".to_string())
        );
        assert!(parse_login_path(b"\0CHARIOX_PROVIDER_PATH_V1\0relative::\0").is_err());
        assert_eq!(
            parse_login_path(b"\0CHARIOX_PROVIDER_PATH_V1\0/bin:/bad\nentry:/usr/bin\0"),
            Ok("/bin:/usr/bin".to_string())
        );
        assert_eq!(
            parse_login_path(b"profile output\n\0CHARIOX_PROVIDER_PATH_V1\0/bin\0"),
            Ok("/bin".to_string())
        );
        assert!(parse_login_path(b"profile output without a probe frame").is_err());
    }
}
