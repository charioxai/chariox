use std::ffi::OsString;
use std::path::Path;

#[cfg(unix)]
use std::ffi::OsStr;
#[cfg(unix)]
use std::path::Component;

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
#[cfg(unix)]
const PATH_PROBE_MARKER: &[u8] = b"\0CHARIOX_PROVIDER_PATH_V1\0";
#[cfg(unix)]
const MAX_PROVIDER_PATH_BYTES: usize = 4096;
#[cfg(unix)]
const MAX_PROBE_OUTPUT_BYTES: usize = 16 * 1024;
#[cfg(unix)]
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve the user's login PATH in a short-lived, isolated Bash process.
/// Only its bounded, framed PATH value crosses back into the bootstrap process.
pub(super) fn resolve_login_path(home: &Path) -> OsString {
    resolve_login_path_with_fallback(home)
}

/// Keep a disposable worker's local provider tools available if its login
/// profile cannot be probed.
pub(super) fn resolve_worker_login_path(home: &Path) -> OsString {
    resolve_login_path_with_fallback(home)
}

fn resolve_login_path_with_fallback(home: &Path) -> OsString {
    match probe_login_path(home) {
        Ok(path) => path,
        Err(reason) => {
            let fallback_path = fallback_path(home);
            let fallback_path_for_log = fallback_path.to_string_lossy().into_owned();
            crate::logging::warn_with_fields(
                "managed_bootstrap.provider_path_probe_failed",
                "managed provider login PATH probe failed; using a safe fallback PATH",
                serde_json::json!({
                    "reason": reason,
                    "fallback_path": fallback_path_for_log,
                }),
            );
            fallback_path
        }
    }
}

fn fallback_path(home: &Path) -> OsString {
    #[cfg(unix)]
    {
        if !home.is_absolute()
            || home
                .components()
                .any(|component| component == Component::ParentDir)
        {
            return OsString::from(BOOTSTRAP_PATH);
        }

        let local_bin = home.join(".local").join("bin");
        let entries =
            std::iter::once(local_bin).chain(std::env::split_paths(OsStr::new(BOOTSTRAP_PATH)));
        match std::env::join_paths(entries) {
            Ok(path) if path.as_os_str().len() <= MAX_PROVIDER_PATH_BYTES => path,
            _ => OsString::from(BOOTSTRAP_PATH),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = home;
        OsString::from(BOOTSTRAP_PATH)
    }
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
        if descriptor.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) == 0
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
    parse_login_path(&output)
}

#[cfg(not(unix))]
fn probe_login_path(_home: &Path) -> Result<OsString, &'static str> {
    Err("login PATH probing requires a Unix platform")
}

#[cfg(unix)]
fn parse_login_path(output: &[u8]) -> Result<OsString, &'static str> {
    use std::os::unix::ffi::OsStringExt;

    let marker_start = output
        .windows(PATH_PROBE_MARKER.len())
        .rposition(|window| window == PATH_PROBE_MARKER)
        .ok_or("login profile did not produce the PATH probe marker")?;
    let framed = output[marker_start + PATH_PROBE_MARKER.len()..]
        .strip_suffix(b"\0")
        .ok_or("login profile emitted unexpected PATH probe output")?;
    if framed.len() > MAX_PROVIDER_PATH_BYTES || framed.contains(&0) {
        return Err("login profile returned an invalid PATH");
    }
    Ok(OsString::from_vec(framed.to_vec()))
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
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
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

    struct TestEnvVar {
        name: &'static str,
        previous: Option<OsString>,
    }

    impl TestEnvVar {
        fn set(name: &'static str, value: impl AsRef<OsStr>) -> Self {
            let previous = std::env::var_os(name);
            std::env::set_var(name, value);
            Self { name, previous }
        }

        fn unset(name: &'static str) -> Self {
            let previous = std::env::var_os(name);
            std::env::remove_var(name);
            Self { name, previous }
        }
    }

    impl Drop for TestEnvVar {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }

    fn write_executable(directory: &Path, name: &str, contents: &str) -> PathBuf {
        fs::create_dir_all(directory).expect("tool directory should be created");
        let tool = directory.join(name);
        fs::write(&tool, contents).expect("tool should be written");
        let mut permissions = fs::metadata(&tool)
            .expect("tool should exist")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&tool, permissions).expect("tool should be executable");
        tool
    }

    #[test]
    fn login_profile_is_reread_for_each_launch_and_preserves_provider_path() {
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
                 export PATH='{expected_path}'\n"
            ),
        )
        .expect("login profile should be written");
        let profile_only_controls = [
            ("CHARIOX_MANAGED_RELEASE_PUBLIC_KEY", "/profile/release-key"),
            ("CHARIOX_MANAGED_KERNEL_BINARY", "/profile/kernel"),
            ("CHARIOX_MANAGED_BOOTSTRAP_PATH", "/profile/bootstrap.json"),
            ("CHARIOX_MANAGED_PROVIDER_TOPOLOGY", "shared_host"),
            ("CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY", "/profile/builder-key"),
            ("LD_PRELOAD", "/profile/hostile.so"),
        ];
        let provider_probe_script = |message: &str| {
            let mut source = String::from("#!/bin/sh\n");
            for (name, value) in profile_only_controls {
                source.push_str("test \"$(printenv ");
                source.push_str(name);
                source.push_str(")\" != '");
                source.push_str(value);
                source.push_str("' || exit 31\n");
            }
            source.push_str(&format!("printf '{message}\\n'\n"));
            source
        };
        let tool = local_bin.join("chariox-provider-path-probe");
        fs::write(&tool, provider_probe_script("ordinary-provider-tool"))
            .expect("provider probe should be written");
        let mut permissions = fs::metadata(&tool)
            .expect("provider probe should exist")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&tool, permissions).expect("provider probe should be executable");

        let supervisor_env = profile_only_controls.map(|(name, _)| (name, std::env::var_os(name)));
        let path = resolve_login_path(&home.0);
        assert_eq!(path, OsString::from(&expected_path));
        for (name, expected) in &supervisor_env {
            assert_eq!(
                std::env::var_os(*name),
                expected.clone(),
                "supervisor env {name} changed"
            );
        }

        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg("chariox-provider-path-probe")
            .env("HOME", &home.0)
            .env("PATH", &path)
            .env_remove("LD_PRELOAD")
            .env_remove("BASH_ENV")
            .output()
            .expect("provider tool should resolve from the captured PATH");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"ordinary-provider-tool\n");

        let profile_bin_b = home.0.join("toolchain-b/bin");
        fs::create_dir_all(&profile_bin_b).expect("second profile provider bin should be created");
        let tool_b = profile_bin_b.join("chariox-provider-path-probe");
        fs::write(&tool_b, provider_probe_script("ordinary-provider-tool-b"))
            .expect("second provider probe should be written");
        let mut permissions = fs::metadata(&tool_b)
            .expect("second provider probe should exist")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&tool_b, permissions)
            .expect("second provider probe should be executable");

        let expected_path_b = format!("{}:{BOOTSTRAP_PATH}", profile_bin_b.display());
        let profile =
            fs::read_to_string(home.0.join(".profile")).expect("login profile should be readable");
        let updated_profile = profile.replace(&expected_path, &expected_path_b);
        assert_ne!(
            profile, updated_profile,
            "second launch profile should change its PATH"
        );
        fs::write(home.0.join(".profile"), updated_profile)
            .expect("updated login profile should be written");

        let restarted_path = resolve_login_path(&home.0);
        assert_eq!(restarted_path, OsString::from(&expected_path_b));
        for (name, expected) in &supervisor_env {
            assert_eq!(
                std::env::var_os(*name),
                expected.clone(),
                "supervisor env {name} changed after the profile update"
            );
        }
        let restarted = Command::new("/bin/sh")
            .arg("-c")
            .arg("chariox-provider-path-probe")
            .env("HOME", &home.0)
            .env("PATH", &restarted_path)
            .env_remove("LD_PRELOAD")
            .env_remove("BASH_ENV")
            .output()
            .expect("provider tool should resolve from the updated login PATH");
        assert!(restarted.status.success());
        assert_eq!(restarted.stdout, b"ordinary-provider-tool-b\n");
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
        let after =
            fs::read(&heartbeat).expect("background writer heartbeat should remain readable");
        assert_eq!(after, before, "background writer survived probe cleanup");
    }

    #[test]
    fn login_path_probe_uses_home_fallback_for_oversized_output() {
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
        assert_eq!(path, fallback_path(&home.0));
    }

    #[test]
    fn fallback_adds_home_bin_without_lossy_path_conversion() {
        let home = Path::new("/srv/worker home");
        assert_eq!(
            fallback_path(home),
            OsString::from(format!(
                "{}:{BOOTSTRAP_PATH}",
                home.join(".local/bin").display()
            ))
        );
        assert_eq!(
            fallback_path(Path::new("relative/home")),
            OsString::from(BOOTSTRAP_PATH)
        );
        assert_eq!(
            fallback_path(Path::new("/srv/worker/../home")),
            OsString::from(BOOTSTRAP_PATH)
        );
        assert_eq!(
            fallback_path(Path::new("/srv/worker:home")),
            OsString::from(BOOTSTRAP_PATH)
        );
        assert_eq!(
            fallback_path(Path::new("/srv/worker\nhome")),
            OsString::from(format!("/srv/worker\nhome/.local/bin:{BOOTSTRAP_PATH}"))
        );

        let mut home_bytes = b"/srv/worker-".to_vec();
        home_bytes.extend_from_slice(b"\xff\nhome");
        let non_utf8_home = PathBuf::from(OsString::from_vec(home_bytes));
        let non_utf8_fallback = fallback_path(&non_utf8_home);
        let expected = std::env::join_paths(
            std::iter::once(non_utf8_home.join(".local").join("bin"))
                .chain(std::env::split_paths(OsStr::new(BOOTSTRAP_PATH))),
        )
        .expect("non-UTF-8 home should be representable in PATH");
        assert_eq!(non_utf8_fallback, expected);

        let too_long_home = PathBuf::from(format!("/{}", "h".repeat(MAX_PROVIDER_PATH_BYTES)));
        assert_eq!(
            fallback_path(&too_long_home),
            OsString::from(BOOTSTRAP_PATH)
        );
    }

    #[test]
    fn login_path_probe_fallback_resolves_provider_from_home_local_bin() {
        let home = TestHome::new();
        fs::write(home.0.join(".profile"), "exit 23\n")
            .expect("failing login profile should be written");

        let local_bin = home.0.join(".local/bin");
        let tool = write_executable(&local_bin, "codex", "#!/bin/sh\nprintf local-codex\n");
        let path = resolve_login_path(&home.0);
        assert_eq!(path, fallback_path(&home.0));

        let _env_lock = crate::env_lock::lock();
        let _codex_override = TestEnvVar::unset("CHARIOX_CODEX_BIN");
        let _path = TestEnvVar::set("PATH", &path);
        let resolved = crate::provider::resolve_codex_executable()
            .expect("provider resolver should find the fallback home tool");
        assert_eq!(resolved, tool);
        let output = Command::new(&resolved)
            .output()
            .expect("fallback provider tool should run");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"local-codex");
    }

    #[test]
    fn login_path_tool_lookup_preserves_relative_and_empty_components_in_selected_cwd() {
        let home = TestHome::new();
        let selected_cwd = home.0.join("selected-cwd");
        let relative_tool = write_executable(
            &selected_cwd.join("bin"),
            "relative-provider-tool",
            "#!/bin/sh\nprintf relative-tool\n",
        );
        let cwd_tool = write_executable(
            &selected_cwd,
            "empty-path-provider-tool",
            "#!/bin/sh\nprintf cwd-tool\n",
        );

        fs::write(home.0.join(".profile"), "export PATH='bin:'\n")
            .expect("relative PATH profile should be written");
        let relative_path = resolve_login_path(&home.0);
        assert_eq!(relative_path, OsString::from("bin:"));
        let relative_output = Command::new("/bin/sh")
            .arg("-c")
            .arg("relative-provider-tool")
            .current_dir(&selected_cwd)
            .env("PATH", &relative_path)
            .env_remove("LD_PRELOAD")
            .env_remove("BASH_ENV")
            .output()
            .expect("relative PATH tool lookup should run");
        assert!(relative_output.status.success());
        assert_eq!(relative_output.stdout, b"relative-tool");
        assert!(relative_tool.is_file());

        fs::write(home.0.join(".profile"), "export PATH=''\n")
            .expect("empty PATH profile should be written");
        let empty_path = resolve_login_path(&home.0);
        assert!(empty_path.is_empty());
        let empty_output = Command::new("/bin/sh")
            .arg("-c")
            .arg("empty-path-provider-tool")
            .current_dir(&selected_cwd)
            .env("PATH", &empty_path)
            .env_remove("LD_PRELOAD")
            .env_remove("BASH_ENV")
            .output()
            .expect("empty PATH tool lookup should run");
        assert!(empty_output.status.success());
        assert_eq!(empty_output.stdout, b"cwd-tool");
        assert!(cwd_tool.is_file());
    }

    #[test]
    fn login_path_accepts_profile_output_before_the_probe_frame() {
        assert_eq!(
            parse_login_path(b"\0CHARIOX_PROVIDER_PATH_V1\0relative:/bin::/usr/bin:\0"),
            Ok(OsString::from("relative:/bin::/usr/bin:"))
        );
        assert_eq!(
            parse_login_path(b"\0CHARIOX_PROVIDER_PATH_V1\0/bin:/bad\nentry:/usr/bin\0"),
            Ok(OsString::from("/bin:/bad\nentry:/usr/bin"))
        );
        assert_eq!(
            parse_login_path(b"profile output\n\0CHARIOX_PROVIDER_PATH_V1\0/bin\0"),
            Ok(OsString::from("/bin"))
        );
        assert_eq!(
            parse_login_path(b"\0CHARIOX_PROVIDER_PATH_V1\0\0"),
            Ok(OsString::new())
        );
        assert!(parse_login_path(b"profile output without a probe frame").is_err());
        assert!(parse_login_path(b"\0CHARIOX_PROVIDER_PATH_V1\0/bin").is_err());
        assert!(parse_login_path(b"\0CHARIOX_PROVIDER_PATH_V1\0/bin\0other\0").is_err());
        let mut oversized = b"\0CHARIOX_PROVIDER_PATH_V1\0".to_vec();
        oversized.resize(oversized.len() + MAX_PROVIDER_PATH_BYTES + 1, b'x');
        oversized.push(0);
        assert!(parse_login_path(&oversized).is_err());
    }

    #[test]
    fn login_path_preserves_non_utf8_and_control_bytes() {
        let mut output = b"\0CHARIOX_PROVIDER_PATH_V1\0/bin:".to_vec();
        output.extend_from_slice(b"relative-\xff\nentry");
        output.push(0);
        let path = parse_login_path(&output).expect("non-NUL Unix PATH bytes should be accepted");
        assert_eq!(path.as_os_str().as_bytes(), b"/bin:relative-\xff\nentry");
    }
}
