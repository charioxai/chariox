//! Build inputs are source files, never a caller-selected shell command.
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    os::unix::{fs::DirBuilderExt, process::CommandExt},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use chariox_app_package::{developer, Limits};
use sha2::{Digest, Sha256};

pub(super) struct Build {
    pub app_id: String,
    pub bytes: Vec<u8>,
    pub digest: String,
}

pub(super) fn build(
    directory: &Path,
    scratch_parent: &Path,
    stopped: &AtomicBool,
) -> Result<Build, super::Failure> {
    let root = fs::canonicalize(directory).map_err(|_| super::Failure { code: crate::local::AppDevPushFailureCode::SourceUnavailable, message: "App source directory is unavailable on this kernel. Restore this workspace path or use files put to place source there. A remote TUI can use /app dev on its client-side folder through package upload.".to_owned() })?;
    let limits = Limits::default();
    let manifest_path = root.join("app.json");
    let metadata = fs::symlink_metadata(&manifest_path)
        .map_err(|_| "App source needs app.json.".to_owned())?;
    if !metadata.is_file() || metadata.len() > limits.max_metadata_bytes as u64 {
        return Err("app.json must be a regular file within the package metadata limit.".into());
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|_| "Could not read app.json.")?)
            .map_err(|_| "app.json is invalid JSON.")?;
    let app_id = manifest["appId"]
        .as_str()
        .ok_or("app.json needs appId.")?
        .to_owned();
    let mut files =
        developer::read_bundle(&root.join("bundle"), &limits).map_err(|error| error.message)?;
    if manifest["schema"] == "chariox.app.v2" {
        let scratch = Scratch::new(scratch_parent)?;
        let output = scratch.0.join("app.wasm");
        let tools = std::env::var_os("CHARIOX_APP_SDK_TOOLS").map(PathBuf::from)
            .ok_or("Set CHARIOX_APP_SDK_TOOLS to the installed pinned App SDK tools directory, then retry.")?;
        let mut command = match manifest["build"]["language"].as_str() {
            Some("typescript" | "javascript") => {
                let mut command = Command::new("node");
                let entry = if manifest["build"]["language"] == "typescript" {
                    "app.ts"
                } else {
                    "app.js"
                };
                command
                    .arg(tools.join("build-js.mjs"))
                    .arg(root.join(entry))
                    .arg(&output);
                command
            }
            Some("rust") => {
                let mut command = if let Some(runner) = std::env::var_os("CHARIOX_APP_CARGO_RUNNER")
                {
                    let mut command = Command::new(runner);
                    command.arg("python3");
                    command
                } else {
                    Command::new("python3")
                };
                command
                    .arg(tools.join("build-rust.py"))
                    .arg(root.join("Cargo.toml"))
                    .arg(&output);
                command
            }
            _ => {
                return Err(
                    "Unsupported App build language; use the pinned JS/TS or Rust SDK.".into(),
                )
            }
        };
        command
            .current_dir(&root)
            .env("TMPDIR", &scratch.0)
            .stdin(Stdio::null());
        run(&mut command, &scratch.0, stopped)?;
        let name = manifest["runtime"]["component"]
            .as_str()
            .ok_or("app.json needs runtime.component.")?;
        if Path::new(name)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
            || name.is_empty()
        {
            return Err("runtime.component must be a relative package path.".into());
        }
        let mut component = File::open(output).map_err(|_| "Build did not produce a component.")?;
        let mut bytes = Vec::new();
        component
            .by_ref()
            .take(limits.max_file_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read built component.")?;
        if bytes.len() > limits.max_file_bytes {
            return Err("Built component exceeds the package file limit.".into());
        }
        files.insert(name.to_owned(), bytes);
    }
    pack(manifest, files, app_id, &limits).map_err(Into::into)
}

fn pack(
    manifest: serde_json::Value,
    files: BTreeMap<String, Vec<u8>>,
    app_id: String,
    limits: &Limits,
) -> Result<Build, String> {
    let (_, bytes) =
        developer::pack_dev_files(manifest, &files, limits).map_err(|error| error.message)?;
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    Ok(Build {
        app_id,
        bytes,
        digest,
    })
}

struct Scratch(PathBuf);
impl Scratch {
    fn new(parent: &Path) -> Result<Self, String> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|_| "Could not create kernel App build storage.")?;
        let path = parent.join(format!("build-{:016x}", rand::random::<u64>()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| "Could not create private App build directory.")?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(command: &mut Command, scratch: &Path, stopped: &AtomicBool) -> Result<(), String> {
    let log_path = scratch.join("build.log");
    let log = File::create(&log_path).map_err(|_| "Could not create build diagnostics.")?;
    command
        .stdout(
            log.try_clone()
                .map_err(|_| "Could not capture build diagnostics.")?,
        )
        .stderr(log)
        .process_group(0);
    let mut child = BuildChild { child: command.spawn().map_err(|_| "Could not start the App compiler. Install the pinned SDK toolchain and its Node/Python prerequisites.")?, wait_authority: true };
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if stopped.load(Ordering::Acquire) {
            return Err(
                "App build cancelled because the kernel is shutting down; retry after restart."
                    .into(),
            );
        }
        if Instant::now() >= deadline {
            return Err("App build exceeded five minutes; inspect the source and pinned toolchain, then retry.".into());
        }
        if fs::metadata(&log_path).is_ok_and(|metadata| metadata.len() > 1024 * 1024) {
            return Err(
                "App build exceeded its diagnostic output limit; reduce compiler output and retry."
                    .into(),
            );
        }
        if let Some(success) = child
            .exited()
            .map_err(|_| "Could not collect App compiler status.")?
        {
            if success {
                return Ok(());
            }
            let mut log = File::open(&log_path)
                .map_err(|_| "App compiler failed; diagnostics are unavailable.")?;
            let _ = log.seek(SeekFrom::End(-4096));
            let mut bytes = Vec::new();
            log.take(4096)
                .read_to_end(&mut bytes)
                .map_err(|_| "Could not read build diagnostics.")?;
            return Err(format!(
                "App build failed. Fix the source or pinned toolchain and save again.\n{}",
                String::from_utf8_lossy(&bytes)
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct BuildChild {
    child: Child,
    wait_authority: bool,
}
impl BuildChild {
    fn exited(&mut self) -> std::io::Result<Option<bool>> {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id() as libc::id_t,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                self.wait_authority = false;
            }
            if error.kind() == std::io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(error);
        }
        let info = unsafe { info.assume_init() };
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        Ok(Some(
            info.si_code == libc::CLD_EXITED && unsafe { info.si_status() } == 0,
        ))
    }
}
impl Drop for BuildChild {
    fn drop(&mut self) {
        // process_group(0) created this group solely for our compiler and its
        // descendants. Keep the leader unreaped until after group cleanup.
        if !self.wait_authority {
            return;
        }
        if let Ok(pid) = i32::try_from(self.child.id()) {
            if pid > 1 {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        }
        let _ = self.child.wait();
    }
}
