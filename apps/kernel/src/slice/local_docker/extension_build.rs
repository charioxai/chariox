use std::fs::File;
use std::process::Command;

use super::{super::model::SliceRecord, LocalDockerSliceOptions};
use crate::error::DaemonError;

pub(super) fn prepare(
    command: &mut Command,
    record: &SliceRecord,
    action: &str,
    options: &LocalDockerSliceOptions,
    stdout: &File,
    stderr: &File,
) -> Result<(), DaemonError> {
    if !super::broker::configured() {
        return Ok(());
    }
    // Recovery and saved-state restore do not build from an extension context.
    // A private user path must never enter the broker's shared-path request.
    command.env_remove("CHARIOX_SLICE_EXTENSION_DOCKERFILE");
    if action != "provision" || options.saved_home_archive.is_some() {
        return Ok(());
    }
    let Some(path) = options.extension_dockerfile.as_deref() else {
        return Ok(());
    };
    #[cfg(target_os = "linux")]
    {
        linux::prepare(command, record, options, path, stdout, stderr)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (record, options, path, stdout, stderr);
        Err(DaemonError::LocalTransport {
            operation: "slice.extension.build",
            message: "the Path-1 extension build helper requires Linux".into(),
        })
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use serde::Serialize;
    use std::collections::BTreeMap;
    use std::ffi::{CString, OsStr, OsString};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Path;
    use std::process::{ExitStatus, Stdio};

    const HELPER: &str = "/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-extension-build.py";

    // Linux native exec upper bound is 6MiB for argv+environment.
    // Base64/JSON needs at most fivefold space for tiny pairs, plus metadata.
    const MAX_NATIVE_ENV_BYTES: usize = 6 * 1024 * 1024;
    const MAX_REQUEST_BYTES: usize = 5 * MAX_NATIVE_ENV_BYTES + 4096;

    #[derive(Serialize)]
    struct DescriptorIdentity {
        fd: i32,
        device: u64,
        inode: u64,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Request {
        phase: &'static str,
        caller_pid: u32,
        image: String,
        policy: &'static str,
        environment: BTreeMap<String, String>,
        context: Option<DescriptorIdentity>,
        dockerfile: Option<DescriptorIdentity>,
        basename: Option<String>,
    }

    fn failure(message: impl Into<String>) -> DaemonError {
        DaemonError::LocalTransport {
            operation: "slice.extension.build",
            message: message.into(),
        }
    }

    fn identity(file: &File) -> Result<DescriptorIdentity, DaemonError> {
        let metadata = file.metadata().map_err(|error| {
            failure(format!("failed to inspect pinned extension input: {error}"))
        })?;
        Ok(DescriptorIdentity {
            fd: file.as_raw_fd(),
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn open_dockerfile(context: &File, basename: &OsStr) -> Result<File, DaemonError> {
        let name = CString::new(basename.as_bytes())
            .map_err(|_| failure("extension Dockerfile basename contains NUL"))?;
        let descriptor = unsafe {
            libc::openat(
                context.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if descriptor < 0 {
            return Err(failure(format!(
                "failed to pin extension Dockerfile: {}",
                std::io::Error::last_os_error()
            )));
        }
        let file = unsafe { File::from_raw_fd(descriptor) };
        if !file
            .metadata()
            .map_err(|error| failure(format!("failed to inspect extension Dockerfile: {error}")))?
            .is_file()
        {
            return Err(failure(
                "extension Dockerfile must resolve to a regular file",
            ));
        }
        Ok(file)
    }

    fn client_environment() -> Result<BTreeMap<String, String>, DaemonError> {
        capture_client_environment(std::env::vars_os())
    }

    fn capture_client_environment(
        variables: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<BTreeMap<String, String>, DaemonError> {
        let mut environment = BTreeMap::new();
        let mut used = 0;
        let mut home = false;
        let mut path = false;
        let pagesize = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if pagesize <= 0 {
            return Err(failure("failed to inspect native environment limits"));
        }
        for (name, value) in variables {
            let name = name.as_bytes();
            let value = value.as_bytes();
            let size = name.len() + value.len() + 2;
            used += size;
            if size > 32 * pagesize as usize || used > MAX_NATIVE_ENV_BYTES {
                return Err(failure(
                    "Docker client environment exceeds Linux native exec limits",
                ));
            }
            if name == b"HOME" {
                home = value.starts_with(b"/");
            }
            if name == b"PATH" {
                path = !value.is_empty();
            }
            environment.insert(STANDARD.encode(name), STANDARD.encode(value));
        }
        if !home || !path {
            return Err(failure(
                "extension builds require the caller's absolute HOME and PATH",
            ));
        }
        Ok(environment)
    }

    fn helper_command(helper: &Path) -> Command {
        let mut command = Command::new("/usr/bin/sudo");
        command
            .args(["-n", "--", "/usr/bin/python3", "-I", "-S", "-B"])
            .arg(helper)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin");
        command
    }

    fn run_helper(
        helper: &Path,
        request: &Request,
        stdout: &File,
        stderr: &File,
    ) -> Result<ExitStatus, DaemonError> {
        let payload = serde_json::to_vec(request).map_err(|error| {
            failure(format!("failed to encode extension build request: {error}"))
        })?;
        if payload.len() > MAX_REQUEST_BYTES {
            return Err(failure("extension build request exceeded its limit"));
        }
        // Isolated Python startup is mandatory while privileged. Only the
        // caller Docker client environment is restored after dropping UID.
        let mut child = helper_command(helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::from(stdout.try_clone().map_err(|error| {
                failure(format!("failed to attach extension build log: {error}"))
            })?))
            .stderr(Stdio::from(stderr.try_clone().map_err(|error| {
                failure(format!("failed to attach extension build log: {error}"))
            })?))
            .spawn()
            .map_err(|error| {
                failure(format!(
                    "failed to launch signed extension build helper: {error}"
                ))
            })?;
        let written = child
            .stdin
            .take()
            .expect("helper stdin pipe")
            .write_all(&payload);
        if let Err(error) = written {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failure(format!(
                "failed to submit extension build request: {error}"
            )));
        }
        // Ordinary extension builds have no global deadline. The helper binds
        // its namespace to this caller and the sudo monitor, bounds retained
        // output, and settles all descendants on cancellation/caller death.
        match child.wait() {
            Ok(status) => Ok(status),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(failure(format!(
                    "failed to wait for extension build helper: {error}"
                )))
            }
        }
    }

    pub(super) fn prepare(
        command: &mut Command,
        record: &SliceRecord,
        options: &LocalDockerSliceOptions,
        path: &Path,
        stdout: &File,
        stderr: &File,
    ) -> Result<(), DaemonError> {
        // Canonicalize the signed facade once, so activation cannot redirect
        // the helper to another release between cache check and build.
        let helper = std::fs::canonicalize(HELPER).map_err(|error| {
            failure(format!(
                "signed Path-1 extension build helper is unavailable: {error}"
            ))
        })?;
        let mut request = Request {
            phase: "check",
            caller_pid: std::process::id(),
            image: super::super::image::selected_image(record, options),
            policy: options.build_image.as_env_value(),
            environment: client_environment()?,
            context: None,
            dockerfile: None,
            basename: None,
        };
        let status = run_helper(&helper, &request, stdout, stderr)?;
        if !status.success() && status.code() != Some(42) {
            return Err(failure(format!(
                "extension image cache preflight failed with {status}; see the slice action log"
            )));
        }
        if status.code() == Some(42) {
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let context = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_PATH | libc::O_DIRECTORY)
                .open(parent)
                .map_err(|error| failure(format!("failed to pin extension context: {error}")))?;
            // Ordinary Docker accepts a selected Dockerfile symlink. Opening
            // before sudo preserves the caller's access check and pins its target.
            let basename = path
                .file_name()
                .ok_or_else(|| failure("extension Dockerfile basename is invalid"))?;
            let dockerfile = open_dockerfile(&context, basename)?;
            request.phase = "build";
            request.context = Some(identity(&context)?);
            request.dockerfile = Some(identity(&dockerfile)?);
            request.basename = Some(
                basename
                    .to_str()
                    .ok_or_else(|| failure("extension Dockerfile basename is invalid"))?
                    .to_string(),
            );
            let status = run_helper(&helper, &request, stdout, stderr)?;
            if !status.success() {
                return Err(failure(format!(
                    "extension image build failed with {status}; see the slice action log"
                )));
            }
            // Source File handles remain alive until the helper and its owned
            // namespace have settled, including all failure paths.
        }
        // The broker consumes the prepared image using its existing compatibility,
        // mount and quota admission. It receives no user context path.
        command.env("CHARIOX_SLICE_BUILD_IMAGE", "never");
        Ok(())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Read;
        use std::os::unix::fs::symlink;

        struct Scratch(std::path::PathBuf);
        impl Scratch {
            fn new() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "chariox-extension-sender-{}-{}",
                    std::process::id(),
                    rand::random::<u64>()
                ));
                std::fs::create_dir(&path).unwrap();
                Self(path)
            }
        }
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        #[test]
        fn dockerfile_open_uses_pinned_directory_after_parent_replacement() {
            let scratch = Scratch::new();
            let original = scratch.0.join("context");
            std::fs::create_dir(&original).unwrap();
            std::fs::write(original.join("Customfile"), "original instructions").unwrap();
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_PATH | libc::O_DIRECTORY)
                .open(&original)
                .unwrap();
            std::fs::rename(&original, scratch.0.join("retained")).unwrap();
            std::fs::create_dir(&original).unwrap();
            std::fs::write(original.join("Customfile"), "foreign instructions").unwrap();
            let mut file = open_dockerfile(&directory, OsStr::new("Customfile")).unwrap();
            let mut text = String::new();
            file.read_to_string(&mut text).unwrap();
            assert_eq!(text, "original instructions");
        }

        #[test]
        fn selected_dockerfile_symlink_target_is_pinned_by_caller() {
            let scratch = Scratch::new();
            std::fs::write(scratch.0.join("instructions"), "original instructions").unwrap();
            symlink("instructions", scratch.0.join("Customfile")).unwrap();
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_PATH | libc::O_DIRECTORY)
                .open(&scratch.0)
                .unwrap();
            let mut file = open_dockerfile(&directory, OsStr::new("Customfile")).unwrap();
            std::fs::rename(scratch.0.join("instructions"), scratch.0.join("retained")).unwrap();
            std::fs::write(scratch.0.join("instructions"), "replacement").unwrap();
            let mut text = String::new();
            file.read_to_string(&mut text).unwrap();
            assert_eq!(text, "original instructions");
        }

        #[test]
        fn docker_credential_helpers_keep_arbitrary_caller_settings() {
            let environment = capture_client_environment([
                ("HOME".into(), "/synthetic".into()),
                ("PATH".into(), "/synthetic/bin".into()),
                ("AWS_PROFILE".into(), "synthetic-profile".into()),
                (
                    "ARBITRARY_HELPER_SETTING".into(),
                    "synthetic-setting".into(),
                ),
            ])
            .unwrap();
            assert_eq!(environment.get("AWS_PROFILE").unwrap(), "synthetic-profile");
            assert_eq!(
                environment
                    .get(&STANDARD.encode(b"ARBITRARY_HELPER_SETTING"))
                    .unwrap(),
                &STANDARD.encode(b"synthetic-setting")
            );
        }

        #[test]
        fn native_byte_and_large_environment_survive_wire_without_utf8_loss() {
            use std::os::unix::ffi::OsStringExt;
            let mut values = vec![
                ("HOME".into(), "/synthetic".into()),
                ("PATH".into(), "/usr/bin:/bin".into()),
                (
                    OsString::from_vec(b"BYTE_SETTING_\xff".to_vec()),
                    OsString::from_vec(vec![255]),
                ),
            ];
            for index in 0..8 {
                values.push((
                    format!("LARGE_SETTING_{index}").into(),
                    "x".repeat(40960).into(),
                ));
            }
            let environment = capture_client_environment(values).unwrap();
            assert_eq!(
                STANDARD
                    .decode(
                        environment
                            .get(&STANDARD.encode(b"BYTE_SETTING_\xff"))
                            .unwrap()
                    )
                    .unwrap(),
                vec![255]
            );
            let wire = serde_json::to_vec(&environment).unwrap();
            assert!(wire.len() > 256 * 1024 && wire.len() < MAX_REQUEST_BYTES);
        }

        #[test]
        fn helper_starts_isolated_python_with_closed_privileged_environment() {
            let command = helper_command(Path::new("/signed/release/helper.py"));
            assert_eq!(command.get_program(), "/usr/bin/sudo");
            assert_eq!(
                command
                    .get_args()
                    .map(|a| a.to_str().unwrap())
                    .collect::<Vec<_>>(),
                vec![
                    "-n",
                    "--",
                    "/usr/bin/python3",
                    "-I",
                    "-S",
                    "-B",
                    "/signed/release/helper.py"
                ]
            );
            assert_eq!(
                command.get_envs().collect::<Vec<_>>(),
                vec![(
                    OsStr::new("PATH"),
                    Some(OsStr::new("/usr/sbin:/usr/bin:/sbin:/bin"))
                )]
            );
        }
    }
}
