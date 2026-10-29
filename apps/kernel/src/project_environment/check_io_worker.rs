//! MP-08 / MP-10 / MP-11: mounted Files and workspace metadata share an owned I/O deadline.
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// Request/result files contain metadata only. Setup values use a private output pipe.
const METADATA_LIMIT: u64 = 64 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Request {
    WorkspaceInspection(crate::runtime::workspace_inspection::Inspection),
    SetupRead {
        root: PathBuf,
        relative: String,
    },
    SetupSource {
        root: PathBuf,
        relative: String,
        expected: Option<String>,
    },
    SetupPaths {
        workspace: String,
        home: PathBuf,
        project: String,
        worker: String,
    },
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
    Software {
        root: Option<PathBuf>,
        name: String,
        source: Option<String>,
        roots: Vec<PathBuf>,
        private_root: PathBuf,
    },
    ManualFiles {
        current: ProjectEnvironment,
        draft: EnvironmentRevisionDraft,
    },
}
#[derive(Serialize, Deserialize)]
pub(super) enum Response {
    WorkspaceInspection(Result<crate::runtime::workspace_inspection::Metadata, ()>),
    #[serde(skip)]
    PublicRead(zeroize::Zeroizing<String>),
    SetupSource,
    SetupPaths {
        workspace: PathBuf,
        preparation: PathBuf,
    },
    Machine(EnvironmentMeasuredFacts),
    Files(EnvironmentRevisionDraft),
    Workspace(super::check_workspace::WorkspacePreflight),
    Binding(super::check_binding::PublicBinding),
    Software(Result<(Option<String>, bool), String>),
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

pub(super) fn run(
    config: &crate::config::DaemonConfig,
    input: Request,
    deadline: Instant,
) -> Result<Response, DaemonError> {
    run_with_cancel(config, input, deadline, || false)
}
pub(super) fn run_with_cancel(
    config: &crate::config::DaemonConfig,
    input: Request,
    deadline: Instant,
    cancelled: impl FnMut() -> bool,
) -> Result<Response, DaemonError> {
    run_at_root(
        &config.private_runtime_state_root(),
        input,
        deadline,
        cancelled,
    )
}
pub(super) fn run_at_root(
    root: &Path,
    input: Request,
    deadline: Instant,
    cancelled: impl FnMut() -> bool,
) -> Result<Response, DaemonError> {
    std::fs::create_dir_all(root)
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
    let private_read = matches!(&input, Request::SetupRead { .. });
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
    if matches!(
        &input,
        Request::Software { .. } | Request::ManualFiles { .. } | Request::WorkspaceInspection(_)
    ) {
        for key in [
            "PATH",
            "HOME",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "RUSTUP_TOOLCHAIN",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
    }
    if matches!(&input, Request::WorkspaceInspection(_)) {
        for key in crate::git_worktree_placement::working_directory_environment_names() {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
    }
    #[cfg(not(test))]
    command
        .arg("--measure-environment-files")
        .arg(&request)
        .arg(&result)
        .arg(left.as_millis().max(1).to_string());
    #[cfg(test)]
    command
        .args([
            "project_environment::check_io_worker::tests::measurement_worker",
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
        "CHARIOX_NAMED_EXECUTABLE_STALL_MARKER",
        "CHARIOX_MANUAL_ROOT_STALL_MARKER",
        "CHARIOX_SETUP_PUBLIC_STALL_MARKER",
        "CHARIOX_SETUP_SOURCE_STALL_MARKER",
        "CHARIOX_WORKSPACE_METADATA_STALL_MARKER",
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
    let stdout = zeroize::Zeroizing::new(output.stdout);
    let _stderr = zeroize::Zeroizing::new(output.stderr);
    if !output.status.success() {
        return Err(environment_error("Files Check measurement unavailable"));
    }
    if private_read {
        return super::setup_inputs::decode_public_file(&stdout).map(Response::PublicRead);
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
        Request::WorkspaceInspection(inspection) => Response::WorkspaceInspection(
            crate::runtime::workspace_inspection::measure(inspection).map_err(|_| ()),
        ),
        Request::SetupRead { root, relative } => {
            return super::setup_inputs::write_public_file(&root, &relative)
        }
        Request::SetupSource {
            root,
            relative,
            expected,
        } => {
            super::setup_inputs::verify_source(&root, &relative, expected.as_deref())?;
            Response::SetupSource
        }
        Request::SetupPaths {
            workspace,
            home,
            project,
            worker,
        } => {
            let (workspace, preparation) =
                crate::runtime::state::prepare_setup_paths(&workspace, &home, &project, &worker)?;
            Response::SetupPaths {
                workspace,
                preparation,
            }
        }
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
        Request::Software {
            root,
            name,
            source,
            roots,
            private_root,
        } => {
            let version = super::check_inventory::software_version_until(
                &name,
                source.as_deref(),
                root.as_deref(),
                &roots,
                deadline,
            );
            let measured = version
                .map(|version| {
                    let owned = source.as_deref() != Some("npm package")
                        && super::check_inventory::executable(&name)
                            .is_some_and(|path| path.starts_with(&private_root));
                    (version, owned)
                })
                .map_err(str::to_owned);
            Response::Software(measured)
        }
        Request::ManualFiles { current, mut draft } => {
            super::manual_files::verify_manual_files_until(&current, &mut draft, deadline)?;
            Response::Files(draft)
        }
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
