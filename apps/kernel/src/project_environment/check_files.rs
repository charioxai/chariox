//! MP-08 / MP-10 / MP-11: mounted Files I/O runs under an owned process deadline.
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// Only selection metadata crosses this private helper boundary, never file contents.
const METADATA_LIMIT: u64 = 64 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    current: ProjectEnvironment,
    draft: EnvironmentRevisionDraft,
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        for name in ["request", "result"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}
fn private_file(path: &Path) -> Result<std::fs::File, DaemonError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|_| environment_error("Files Check metadata unavailable"))
}
fn read_metadata<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, DaemonError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| environment_error("Files Check metadata unavailable"))?;
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() <= METADATA_LIMIT)
    {
        return Err(environment_error("Files Check metadata exceeds bounds"));
    }
    let mut bytes = Vec::new();
    file.take(METADATA_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| environment_error("Files Check metadata unavailable"))?;
    if bytes.len() as u64 > METADATA_LIMIT {
        return Err(environment_error("Files Check metadata exceeds bounds"));
    }
    serde_json::from_slice(&bytes).map_err(|_| environment_error("Files Check metadata invalid"))
}

pub(super) fn measure(
    config: &crate::config::DaemonConfig,
    current: &ProjectEnvironment,
    draft: &mut EnvironmentRevisionDraft,
    deadline: Instant,
) -> Result<(), DaemonError> {
    let root = config.private_runtime_state_root();
    std::fs::create_dir_all(&root)
        .map_err(|_| environment_error("Files Check state unavailable"))?;
    let path = root.join(format!("files-check-{:032x}", rand::random::<u128>()));
    let mut directory = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory
        .create(&path)
        .map_err(|_| environment_error("Files Check state unavailable"))?;
    let scratch = Scratch(path);
    let request = scratch.0.join("request");
    let result = scratch.0.join("result");
    let bytes = serde_json::to_vec(&Request {
        current: current.clone(),
        draft: draft.clone(),
    })
    .map_err(|_| environment_error("Files Check metadata invalid"))?;
    if bytes.len() as u64 > METADATA_LIMIT {
        return Err(environment_error("Files Check metadata exceeds bounds"));
    }
    private_file(&request)?
        .write_all(&bytes)
        .map_err(|_| environment_error("Files Check metadata unavailable"))?;
    let left = super::check_inventory::remaining(deadline, Duration::from_secs(60))
        .map_err(|_| environment_error("Files Check budget exceeded"))?;
    let mut command = std::process::Command::new(
        std::env::current_exe().map_err(|_| environment_error("Files Check worker unavailable"))?,
    );
    command.current_dir("/").env_clear();
    #[cfg(not(test))]
    command
        .arg("--measure-environment-files")
        .arg(&request)
        .arg(&result)
        .arg(left.as_millis().max(1).to_string());
    #[cfg(test)]
    command
        .args([
            "project_environment::check_files::tests::measurement_worker",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("CHARIOX_FILES_CHECK_REQUEST", &request)
        .env("CHARIOX_FILES_CHECK_RESULT", &result)
        .env(
            "CHARIOX_FILES_CHECK_BUDGET_MS",
            left.as_millis().max(1).to_string(),
        );
    #[cfg(test)]
    if let Some(marker) = std::env::var_os("CHARIOX_FILES_CHECK_IO_STALL_MARKER") {
        command.env("CHARIOX_FILES_CHECK_IO_STALL_MARKER", marker);
    }
    let output = crate::local::provider_requests::capture_inventory_probe(command, left)
        .map_err(|_| environment_error("Files Check worker did not settle within its budget"))?;
    if !output.status.success() {
        return Err(environment_error("Files Check measurement unavailable"));
    }
    *draft = read_metadata(&result)?;
    Ok(())
}

/// Internal self-exec entry; runs before kernel credentials, logging or runtime initialization.
#[doc(hidden)]
pub fn run_files_measurement_worker(
    request: &Path,
    result: &Path,
    budget: Duration,
) -> Result<(), DaemonError> {
    if !crate::io::control_memory::make_process_nondumpable() {
        return Err(environment_error(
            "Files Check worker memory protection unavailable",
        ));
    }
    let deadline = Instant::now() + budget;
    let Request { current, mut draft } = read_metadata(request)?;
    super::manual_files::verify_check_files(&current, &mut draft, deadline)?;
    let bytes = serde_json::to_vec(&draft)
        .map_err(|_| environment_error("Files Check metadata invalid"))?;
    if bytes.len() as u64 > METADATA_LIMIT {
        return Err(environment_error("Files Check metadata exceeds bounds"));
    }
    private_file(result)?
        .write_all(&bytes)
        .map_err(|_| environment_error("Files Check metadata unavailable"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "owned subprocess entry, exercised by Files Check runner tests"]
    fn measurement_worker() {
        let Some(request) = std::env::var_os("CHARIOX_FILES_CHECK_REQUEST") else {
            return;
        };
        let result = std::env::var_os("CHARIOX_FILES_CHECK_RESULT").unwrap();
        let budget = std::env::var("CHARIOX_FILES_CHECK_BUDGET_MS")
            .unwrap()
            .parse::<u64>()
            .unwrap();
        std::process::exit(
            if run_files_measurement_worker(
                Path::new(&request),
                Path::new(&result),
                Duration::from_millis(budget),
            )
            .is_ok()
            {
                0
            } else {
                1
            },
        );
    }
}
