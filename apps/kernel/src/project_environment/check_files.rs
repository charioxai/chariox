//! MP-08 / MP-10 / MP-11: mounted Files and workspace metadata share an owned I/O deadline.
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
enum Request {
    Machine {
        root: Option<PathBuf>,
    },
    Files {
        current: ProjectEnvironment,
        draft: EnvironmentRevisionDraft,
    },
    Workspace {
        folders: Vec<(String, PathBuf)>,
    },
    #[cfg(test)]
    PublicationBinding {
        root: Option<PathBuf>,
        locator: ProjectEnvironmentLocator,
    },
    Binding {
        root: Option<PathBuf>,
        locator: ProjectEnvironmentLocator,
    },
    Npm {
        root: Option<PathBuf>,
        name: String,
    },
}
#[derive(Serialize, Deserialize)]
enum Response {
    Machine(EnvironmentMeasuredFacts),
    Files(EnvironmentRevisionDraft),
    Workspace(super::check_workspace::WorkspacePreflight),
    Binding(super::check_binding::PublicBinding),
    Npm(Result<Option<String>, String>),
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

pub(super) fn machine(
    config: &crate::config::DaemonConfig,
    root: Option<&Path>,
    deadline: Instant,
) -> Result<EnvironmentMeasuredFacts, DaemonError> {
    match run(
        config,
        Request::Machine {
            root: root.map(Path::to_path_buf),
        },
        deadline,
    )? {
        Response::Machine(facts) => Ok(facts),
        _ => Err(environment_error("Machine Check worker result invalid")),
    }
}

pub(super) fn measure(
    config: &crate::config::DaemonConfig,
    current: &ProjectEnvironment,
    draft: &mut EnvironmentRevisionDraft,
    deadline: Instant,
) -> Result<(), DaemonError> {
    match run(
        config,
        Request::Files {
            current: current.clone(),
            draft: draft.clone(),
        },
        deadline,
    )? {
        Response::Files(measured) => {
            *draft = measured;
            Ok(())
        }
        _ => Err(environment_error("Files Check worker result invalid")),
    }
}

pub(super) fn workspace(
    config: &crate::config::DaemonConfig,
    current: &ProjectEnvironment,
    deadline: Instant,
) -> Result<super::check_workspace::WorkspacePreflight, DaemonError> {
    match run(
        config,
        Request::Workspace {
            folders: current
                .folders
                .iter()
                .filter(|f| !f.local_workspace_binding.is_empty())
                .map(|f| {
                    (
                        f.folder_id.clone(),
                        PathBuf::from(&f.local_workspace_binding),
                    )
                })
                .collect(),
        },
        deadline,
    )? {
        Response::Workspace(measured) => Ok(measured),
        _ => Err(environment_error("Workspace Check worker result invalid")),
    }
}

pub(super) fn binding(
    config: &crate::config::DaemonConfig,
    root: Option<&Path>,
    locator: &ProjectEnvironmentLocator,
    deadline: Instant,
) -> Result<super::check_binding::PublicBinding, DaemonError> {
    match run(
        config,
        Request::Binding {
            root: root.map(Path::to_path_buf),
            locator: locator.clone(),
        },
        deadline,
    )? {
        Response::Binding(binding) => Ok(binding),
        _ => Err(environment_error("Binding Check worker result invalid")),
    }
}
pub(super) fn npm(
    config: &crate::config::DaemonConfig,
    root: Option<&Path>,
    name: &str,
    deadline: Instant,
) -> Result<Option<String>, &'static str> {
    match run(
        config,
        Request::Npm {
            root: root.map(Path::to_path_buf),
            name: name.into(),
        },
        deadline,
    )
    .map_err(|_| "package_unproven")?
    {
        Response::Npm(result) => result.map_err(|_| "package_unproven"),
        _ => Err("package_unproven"),
    }
}

pub(super) fn publication_binding(
    config: &crate::config::DaemonConfig,
    root: &Path,
    locator: &ProjectEnvironmentLocator,
    deadline: Instant,
    cancelled: impl FnMut() -> bool,
) -> Result<super::check_binding::PublicBinding, DaemonError> {
    #[cfg(test)]
    let input = Request::PublicationBinding {
        root: Some(root.to_path_buf()),
        locator: locator.clone(),
    };
    #[cfg(not(test))]
    let input = Request::Binding {
        root: Some(root.to_path_buf()),
        locator: locator.clone(),
    };
    match run_with_cancel(config, input, deadline, cancelled)? {
        Response::Binding(facts) => Ok(facts),
        _ => Err(environment_error("Binding reader result invalid")),
    }
}
fn run(
    config: &crate::config::DaemonConfig,
    input: Request,
    deadline: Instant,
) -> Result<Response, DaemonError> {
    run_with_cancel(config, input, deadline, || false)
}
fn run_with_cancel(
    config: &crate::config::DaemonConfig,
    input: Request,
    deadline: Instant,
    cancelled: impl FnMut() -> bool,
) -> Result<Response, DaemonError> {
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
    let bytes = serde_json::to_vec(&input)
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
    #[cfg(test)]
    if let Some(marker) = std::env::var_os("CHARIOX_WORKSPACE_CHECK_STALL_MARKER") {
        command.env("CHARIOX_WORKSPACE_CHECK_STALL_MARKER", marker);
    }
    #[cfg(test)]
    for key in [
        "CHARIOX_WORKSPACE_CHECK_COUNT",
        "CHARIOX_MACHINE_CAPACITY_STALL_MARKER",
        "CHARIOX_TYPED_CHECK_STALL_MARKER",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    #[cfg(test)]
    if matches!(&input, Request::PublicationBinding { .. }) {
        if let Some(marker) = std::env::var_os("CHARIOX_APPLY_BINDING_STALL_MARKER") {
            command.env("CHARIOX_APPLY_BINDING_STALL_MARKER", marker);
        }
    }
    let output = crate::local::provider_requests::capture_inventory_probe_cancellable(
        command, left, cancelled,
    )
    .map_err(|_| environment_error("Files Check worker did not settle within its budget"))?;
    if !output.status.success() {
        return Err(environment_error("Files Check measurement unavailable"));
    }
    read_metadata(&result)
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
    let response = match read_metadata::<Request>(request)? {
        Request::Machine { root } => {
            Response::Machine(super::check_inventory::machine(root.as_deref()))
        }
        Request::Files { current, mut draft } => {
            super::manual_files::verify_check_files(&current, &mut draft, deadline)?;
            Response::Files(draft)
        }
        Request::Workspace { folders } => {
            Response::Workspace(super::check_workspace::inspect(&folders))
        }
        #[cfg(test)]
        Request::PublicationBinding { root, locator } => {
            super::apply_bindings::publication_probe_test_gate();
            Response::Binding(super::check_binding::inspect(root.as_deref(), &locator))
        }
        Request::Binding { root, locator } => {
            Response::Binding(super::check_binding::inspect(root.as_deref(), &locator))
        }
        Request::Npm { root, name } => Response::Npm(
            super::check_inventory::software_version_until(
                &name,
                Some("npm package"),
                root.as_deref(),
                &[],
                deadline,
            )
            .map_err(str::to_owned),
        ),
    };
    let bytes = serde_json::to_vec(&response)
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
