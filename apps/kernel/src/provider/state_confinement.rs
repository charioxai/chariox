//! Agents never see the kernel's private state. Every provider run and every
//! command the kernel starts for an agent runs where the kernel's secrets are
//! out of reach: its state folder (database, relay credential, every provider
//! profile), Vault, credentials, machine identity, relay pairing and history.
//! What is the agent's own stays: the paths its launch names (working folder,
//! provider profile, scratch folder, program) are mounted back, and the
//! chariox CLI keeps the kernel registry and local sign-in token it reads to
//! reach this kernel, read-only.
//!
//! Only the secrets themselves are covered, never the folder around them, so
//! everything an agent writes next to a secret reaches the disk. Linux covers
//! them in a mount namespace made by Bubblewrap, which unprivileged kernels may
//! use where distributions restrict user namespaces. macOS denies them with a
//! Seatbelt profile. A kernel that cannot confine refuses to start the process;
//! nothing runs unconfined.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::ProviderLaunchResult;
use crate::error::DaemonError;

/// Folders of the kernel home that hold its secrets.
const SECRET_HOME_FOLDERS: &[&str] = &[
    "state",
    "credentials",
    "machine",
    "daemon",
    "history",
    "sessions",
    "kernels",
    "vault",
];

/// Launch environment that names the agent's own folders.
const OWN_PATH_ENVIRONMENT: &[&str] = &[
    "HOME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
    "OPENCODE_CONFIG_DIR",
    "GH_CONFIG_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
];

/// Set inside every confined Linux process; it also marks the wrapper's arguments.
#[cfg(target_os = "linux")]
pub(crate) const STATE_CONFINEMENT_MARKER_ENV: &str = "CHARIOX_KERNEL_STATE_CONFINED";

/// Secret files the kernel's configuration may place anywhere.
static CONFIGURED_FILES: OnceLock<Vec<PathBuf>> = OnceLock::new();
/// Secret folders the kernel's configuration may place anywhere.
static CONFIGURED_FOLDERS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// The kernel's secrets as they are on disk now.
struct Secrets {
    /// Covered whole; outermost first, none inside another.
    folders: Vec<PathBuf>,
    /// Covered one by one; none inside a covered folder.
    files: Vec<PathBuf>,
}

impl Secrets {
    fn cover(&self, path: &Path) -> bool {
        self.folders.iter().any(|folder| path.starts_with(folder))
    }
}

/// Records where the configuration keeps secrets, and creates the kernel home's
/// secret folders, so one the kernel fills later is covered from the start.
pub(crate) fn configure(config: &crate::config::DaemonConfig) {
    let home = crate::config::default_config_dir();
    for folder in SECRET_HOME_FOLDERS {
        let folder = home.join(folder);
        if !folder.exists() {
            if let Err(error) = private_folder(&folder) {
                crate::logging::warn_with_fields(
                    "provider.state_confinement",
                    "could not create a kernel secret folder; agents started before it exists can read it",
                    serde_json::json!({"folder": folder.display().to_string(), "error": error.to_string()}),
                );
            }
        }
    }
    let vault = &config.user_config.credential_vault.path;
    let vault = match vault.strip_prefix("~/") {
        Some(suffix) => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(suffix))
            .unwrap_or_else(|| PathBuf::from(vault)),
        None => PathBuf::from(vault),
    };
    let _ = CONFIGURED_FILES.set(vec![
        vault,
        config.durable_state_path(),
        config.account_profile_registry_path(),
        config.operational_history_path(),
    ]);
    let _ = CONFIGURED_FOLDERS.set(vec![config.session_history_root()]);
}

fn private_folder(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// The launch, run so that it and everything it starts cannot read the kernel's
/// secrets. An endpoint the kernel does not start runs no process here.
pub(crate) fn confine_launch(
    mut launch: ProviderLaunchResult,
) -> Result<ProviderLaunchResult, DaemonError> {
    let Some(program) = launch.pty_program.take() else {
        return Ok(launch);
    };
    let (program, args) = confine_command(
        program,
        std::mem::take(&mut launch.pty_args),
        &launch.pty_env,
        launch.working_directory.iter().cloned().collect(),
    )?;
    launch.pty_program = Some(program);
    launch.pty_args = args;
    Ok(launch)
}

/// `program` with `args`, wrapped so it cannot read the kernel's secrets. The
/// folders `environment` names as the agent's own and `own` (its working
/// folder, files the kernel hands it) stay reachable.
pub(crate) fn confine_command(
    program: String,
    args: Vec<String>,
    environment: &BTreeMap<String, String>,
    mut own: Vec<PathBuf>,
) -> Result<(String, Vec<String>), DaemonError> {
    let secrets = secrets();
    own.extend(
        OWN_PATH_ENVIRONMENT
            .iter()
            .filter_map(|name| environment.get(*name))
            .map(PathBuf::from),
    );
    own.extend(Path::new(&program).parent().map(Path::to_path_buf));
    let own = existing_canonical(own);
    if let Some(path) = own.iter().find(|path| secrets.folders.contains(path)) {
        return Err(confinement_error(format!(
            "{} is the kernel's private folder, so no agent process may work in it; use a folder outside it",
            path.display()
        )));
    }
    let shared = existing_canonical(cli_paths());
    platform::wrap(program, args, &secrets, &own, &shared)
}

/// The length of a confinement prefix at the start of `args`, through the program.
pub(crate) fn confinement_prefix_len(args: &[String]) -> Option<usize> {
    platform::prefix_len(args)
}

/// Every secret that exists now. A secret's siblings that start with its name
/// (Vault key envelopes, database journals) are secrets too.
fn secrets() -> Secrets {
    let home = crate::config::default_config_dir();
    let mut candidates = SECRET_HOME_FOLDERS
        .iter()
        .map(|folder| home.join(folder))
        .collect::<Vec<_>>();
    candidates.push(crate::config::default_state_dir());
    candidates.extend(CONFIGURED_FOLDERS.get().into_iter().flatten().cloned());
    candidates.extend(CONFIGURED_FILES.get().into_iter().flatten().cloned());
    let mut folders = Vec::new();
    let mut files = Vec::new();
    for path in candidates
        .iter()
        .flat_map(|candidate| named_like(candidate))
    {
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if canonical.is_dir() {
            folders.push(canonical);
        } else {
            files.push(canonical);
        }
    }
    folders.sort_by_key(|path| path.components().count());
    let mut outermost: Vec<PathBuf> = Vec::new();
    for folder in folders {
        if !outermost.iter().any(|outer| folder.starts_with(outer)) {
            outermost.push(folder);
        }
    }
    files.sort();
    files.dedup();
    files.retain(|file| !outermost.iter().any(|folder| file.starts_with(folder)));
    Secrets {
        folders: outermost,
        files,
    }
}

/// `path` and its siblings whose names start with its name, as they exist now.
fn named_like(path: &Path) -> Vec<PathBuf> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .as_encoded_bytes()
                .starts_with(name.as_encoded_bytes())
        })
        .map(|entry| entry.path())
        .collect()
}

/// What the chariox CLI reads to reach this kernel.
fn cli_paths() -> Vec<PathBuf> {
    vec![
        crate::config::default_state_dir().join("kernel-local-auth"),
        crate::config::default_config_dir()
            .join("kernels")
            .join("active"),
    ]
}

fn existing_canonical(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut paths = paths
        .into_iter()
        .filter_map(|path| path.canonicalize().ok())
        .collect::<Vec<_>>();
    paths.sort_by_key(|path| path.components().count());
    paths.dedup();
    paths
}

fn confinement_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "kernel_state_confinement",
        message: message.into(),
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use super::{confinement_error, Secrets, STATE_CONFINEMENT_MARKER_ENV};
    use crate::error::DaemonError;
    use crate::runtime::bounded_process::BoundedCommandExt as _;

    static WORKING_BUBBLEWRAP: OnceLock<PathBuf> = OnceLock::new();

    /// Each secret folder is covered by an empty one and each secret file by a
    /// socket nobody listens on, so reading it fails instead of looking empty.
    /// The agent's own paths inside a covered folder are mounted back.
    pub(super) fn wrap(
        program: String,
        args: Vec<String>,
        secrets: &Secrets,
        own: &[PathBuf],
        shared: &[PathBuf],
    ) -> Result<(String, Vec<String>), DaemonError> {
        let bubblewrap = working_bubblewrap()?;
        let mut wrapped = vec!["--dev-bind".to_string(), "/".to_string(), "/".to_string()];
        for folder in &secrets.folders {
            wrapped.extend(["--tmpfs".to_string(), path_arg(folder)]);
        }
        if !secrets.files.is_empty() {
            let stand_in = unreadable_stand_in()?;
            for file in &secrets.files {
                wrapped.extend(["--ro-bind".to_string(), path_arg(&stand_in), path_arg(file)]);
            }
        }
        for (paths, bind) in [(shared, "--ro-bind"), (own, "--bind")] {
            for path in paths.iter().filter(|path| secrets.cover(path)) {
                wrapped.extend([bind.to_string(), path_arg(path), path_arg(path)]);
            }
        }
        wrapped.extend([
            "--setenv".to_string(),
            STATE_CONFINEMENT_MARKER_ENV.to_string(),
            "1".to_string(),
            "--".to_string(),
            program,
        ]);
        wrapped.extend(args);
        Ok((bubblewrap.display().to_string(), wrapped))
    }

    pub(super) fn prefix_len(args: &[String]) -> Option<usize> {
        let separator = args.iter().position(|arg| arg == "--")?;
        args[..separator]
            .windows(2)
            .any(|window| window == ["--setenv", STATE_CONFINEMENT_MARKER_ENV])
            .then_some(separator + 2)
            .filter(|end| *end <= args.len())
    }

    fn path_arg(path: &Path) -> String {
        path.display().to_string()
    }

    /// A socket in the kernel's state folder that nobody listens on: opening it
    /// fails with "No such device or address".
    fn unreadable_stand_in() -> Result<PathBuf, DaemonError> {
        use std::os::unix::fs::FileTypeExt as _;
        let state = crate::config::default_state_dir();
        let path = state.join("hidden-secret.sock");
        let is_socket = |path: &Path| {
            std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
        };
        if is_socket(&path) {
            return Ok(path);
        }
        let made = super::private_folder(&state).and_then(|()| {
            let _ = std::fs::remove_file(&path);
            // Through the folder's descriptor the socket path stays short
            // whatever the depth of the kernel home.
            let folder = std::fs::File::open(&state)?;
            let short = format!(
                "/proc/self/fd/{}/hidden-secret.sock",
                std::os::fd::AsRawFd::as_raw_fd(&folder)
            );
            std::os::unix::net::UnixListener::bind(short).map(drop)
        });
        match made {
            Ok(()) => Ok(path),
            // Another launch made it first.
            Err(_) if is_socket(&path) => Ok(path),
            Err(error) => Err(confinement_error(format!(
                "could not make {} to stand in for the kernel's secret files: {error}",
                path.display()
            ))),
        }
    }

    /// Bubblewrap from PATH, once it has made a namespace on this machine.
    fn working_bubblewrap() -> Result<&'static PathBuf, DaemonError> {
        if let Some(path) = WORKING_BUBBLEWRAP.get() {
            return Ok(path);
        }
        let path = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .map(|directory| directory.join("bwrap"))
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| {
                confinement_error(
                    "agents run inside Bubblewrap so they cannot read the kernel's secrets, and bwrap is not installed; install bubblewrap (apt install bubblewrap, dnf install bubblewrap) and retry",
                )
            })?;
        let output = crate::process_spawn::command(&path)
            .args(["--dev-bind", "/", "/", "--"])
            .arg(&path)
            .arg("--version")
            .bounded_output()
            .map_err(|error| {
                confinement_error(format!("could not run {}: {error}", path.display()))
            })?;
        if !output.status.success() {
            return Err(confinement_error(format!(
                "agents run inside Bubblewrap so they cannot read the kernel's secrets, and {} cannot make a mount namespace here ({}); allow unprivileged user namespaces for bwrap or run the kernel as root, then retry",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(WORKING_BUBBLEWRAP.get_or_init(|| path))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::{Path, PathBuf};

    use super::{confinement_error, Secrets};
    use crate::error::DaemonError;

    const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
    const PROFILE_HEADER: &str = ";; chariox kernel state confinement\n";

    /// Seatbelt denies the secret folders and files by path. The last matching
    /// rule wins, so the agent's own and the CLI's paths are allowed back after
    /// the denials.
    pub(super) fn wrap(
        program: String,
        args: Vec<String>,
        secrets: &Secrets,
        own: &[PathBuf],
        shared: &[PathBuf],
    ) -> Result<(String, Vec<String>), DaemonError> {
        if !Path::new(SANDBOX_EXEC).exists() {
            return Err(confinement_error(format!(
                "agents run inside Seatbelt so they cannot read the kernel's secrets, and {SANDBOX_EXEC} is missing"
            )));
        }
        let mut profile = format!("{PROFILE_HEADER}(version 1)\n(allow default)\n");
        for folder in &secrets.folders {
            profile.push_str(&format!(
                "(deny file-read* file-write* (regex #\"^{}(/|$)\"))\n",
                escaped(folder)
            ));
        }
        for file in &secrets.files {
            profile.push_str(&format!(
                "(deny file-read* file-write* (regex #\"^{}$\"))\n",
                escaped(file)
            ));
        }
        for (paths, operations) in [(own, "file-read* file-write*"), (shared, "file-read*")] {
            for path in paths.iter().filter(|path| secrets.cover(path)) {
                profile.push_str(&format!(
                    "(allow {operations} (regex #\"^{}(/|$)\"))\n",
                    escaped(path)
                ));
            }
        }
        let mut wrapped = vec!["-p".to_string(), profile, program];
        wrapped.extend(args);
        Ok((SANDBOX_EXEC.to_string(), wrapped))
    }

    pub(super) fn prefix_len(args: &[String]) -> Option<usize> {
        (args.len() >= 3 && args[0] == "-p" && args[1].starts_with(PROFILE_HEADER)).then_some(3)
    }

    fn escaped(path: &Path) -> String {
        let mut escaped = String::new();
        for character in path.display().to_string().chars() {
            if "\\.+*?()|[]{}^$\"".contains(character) {
                escaped.push('\\');
            }
            escaped.push(character);
        }
        escaped
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    use std::path::PathBuf;

    use super::{confinement_error, Secrets};
    use crate::error::DaemonError;

    pub(super) fn wrap(
        _program: String,
        _args: Vec<String>,
        _secrets: &Secrets,
        _own: &[PathBuf],
        _shared: &[PathBuf],
    ) -> Result<(String, Vec<String>), DaemonError> {
        Err(confinement_error(
            "this operating system cannot keep agents away from the kernel's secrets, so the kernel starts no agent processes on it",
        ))
    }

    pub(super) fn prefix_len(_args: &[String]) -> Option<usize> {
        None
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "state_confinement_tests.rs"]
pub(crate) mod tests;
