//! Agents never see the kernel's private state. Every provider run and every
//! command the kernel starts for an agent runs where the kernel's secrets are
//! out of reach: its state folder (database, relay credential, every provider
//! profile), Vault, credentials, machine identity, relay pairing and history.
//! What is the agent's own stays: the paths its launch names (working folder,
//! provider profile, scratch folder, program) are mounted back, and the
//! chariox CLI keeps the kernel registry and local sign-in token it reads to
//! reach this kernel, read-only.
//!
//! Linux hides them in a mount namespace made by Bubblewrap, which unprivileged
//! kernels may use where distributions restrict user namespaces. macOS denies
//! them with a Seatbelt profile. A kernel that cannot confine refuses to start
//! the process; nothing runs unconfined.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::ProviderLaunchResult;
use crate::error::DaemonError;

/// Entries of the kernel home that hold its secrets.
const SECRET_HOME_ENTRIES: &[&str] = &[
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

static CONFIGURED_SECRETS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// Adds the secret locations the kernel's configuration moved out of its home.
pub(crate) fn configure(config: &crate::config::DaemonConfig) {
    let vault = &config.user_config.credential_vault.path;
    let vault = match vault.strip_prefix("~/") {
        Some(suffix) => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(suffix))
            .unwrap_or_else(|| PathBuf::from(vault)),
        None => PathBuf::from(vault),
    };
    let mut secrets = vec![
        vault,
        config.private_runtime_state_root(),
        config.session_history_root(),
    ];
    secrets.extend(
        config
            .operational_history_path()
            .parent()
            .map(Path::to_path_buf),
    );
    let _ = CONFIGURED_SECRETS.set(secrets);
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
    let home = canonical_location(&crate::config::default_config_dir()).ok_or_else(|| {
        confinement_error("the kernel home has no absolute location to hide its secrets in")
    })?;
    let secrets = secret_paths(&home);
    own.extend(
        OWN_PATH_ENVIRONMENT
            .iter()
            .filter_map(|name| environment.get(*name))
            .map(PathBuf::from),
    );
    own.extend(Path::new(&program).parent().map(Path::to_path_buf));
    let own = existing_canonical(own);
    if let Some(path) = own.iter().find(|path| secrets.contains(path)) {
        return Err(confinement_error(format!(
            "{} is the kernel's private folder, so no agent process may work in it; use a folder outside it",
            path.display()
        )));
    }
    let shared = existing_canonical(cli_paths());
    platform::wrap(program, args, &secrets, &home, &own, &shared)
}

/// The length of a confinement prefix at the start of `args`, through the program.
pub(crate) fn confinement_prefix_len(args: &[String]) -> Option<usize> {
    platform::prefix_len(args)
}

/// Every secret location, canonical, outermost first, none inside another.
fn secret_paths(home: &Path) -> Vec<PathBuf> {
    let mut secrets = SECRET_HOME_ENTRIES
        .iter()
        .map(|entry| home.join(entry))
        .collect::<Vec<_>>();
    secrets.push(crate::config::default_state_dir());
    secrets.extend(CONFIGURED_SECRETS.get().into_iter().flatten().cloned());
    let mut secrets = secrets
        .into_iter()
        .filter_map(|path| canonical_location(&path))
        .collect::<Vec<_>>();
    secrets.sort_by_key(|path| path.components().count());
    let mut outermost: Vec<PathBuf> = Vec::new();
    for path in secrets {
        if !outermost.iter().any(|secret| path.starts_with(secret)) {
            outermost.push(path);
        }
    }
    outermost
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

/// The canonical form of a location that may not exist yet: its existing
/// ancestor resolved, the rest appended.
fn canonical_location(path: &Path) -> Option<PathBuf> {
    let path = std::path::absolute(path).ok()?;
    let mut missing = Vec::new();
    let mut existing = path.as_path();
    loop {
        if let Ok(canonical) = existing.canonicalize() {
            return Some(
                missing
                    .iter()
                    .rev()
                    .fold(canonical, |path, name| path.join(name)),
            );
        }
        missing.push(existing.file_name()?.to_os_string());
        existing = existing.parent()?;
    }
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use super::{confinement_error, STATE_CONFINEMENT_MARKER_ENV};
    use crate::error::DaemonError;
    use crate::runtime::bounded_process::BoundedCommandExt as _;

    static WORKING_BUBBLEWRAP: OnceLock<PathBuf> = OnceLock::new();

    /// A secret folder outside the kernel home is covered by an empty one.
    /// Every other secret is hidden by an empty folder over its parent with the
    /// parent's other entries mounted back, so a file vanishes as a folder does
    /// and a secret created later never appears; entries starting with a
    /// secret's name (Vault key envelopes) go with it. The kernel home is always
    /// hidden this way. Outer folders come first, so an inner cover hides what
    /// an outer one mounted back.
    pub(super) fn wrap(
        program: String,
        args: Vec<String>,
        secrets: &[PathBuf],
        home: &Path,
        own: &[PathBuf],
        shared: &[PathBuf],
    ) -> Result<(String, Vec<String>), DaemonError> {
        let bubblewrap = working_bubblewrap()?;
        let mut covers: BTreeMap<PathBuf, BTreeSet<OsString>> = BTreeMap::new();
        for secret in secrets {
            let parent = secret.parent().filter(|parent| *parent != Path::new("/"));
            match (parent, secret.file_name()) {
                (Some(parent), _) if secret.is_dir() && parent != home => {
                    covers.entry(secret.clone()).or_default();
                }
                (Some(parent), Some(name)) => {
                    covers
                        .entry(parent.to_path_buf())
                        .or_default()
                        .insert(name.to_os_string());
                }
                _ => {
                    return Err(confinement_error(format!(
                        "{} sits directly in /, so it cannot be hidden from agents; move it into a folder",
                        secret.display()
                    )))
                }
            }
        }
        let mut folders = covers.keys().cloned().collect::<Vec<_>>();
        folders.sort_by_key(|path| path.components().count());
        let mut wrapped = vec!["--dev-bind".to_string(), "/".to_string(), "/".to_string()];
        // A folder that does not exist holds nothing to read.
        for folder in folders.into_iter().filter(|folder| folder.is_dir()) {
            wrapped.extend(["--tmpfs".to_string(), path_arg(&folder)]);
            let names = &covers[&folder];
            if names.is_empty() {
                continue;
            }
            let entries = std::fs::read_dir(&folder).map_err(|error| {
                confinement_error(format!(
                    "could not list {} to hide the kernel's secrets in it: {error}",
                    folder.display()
                ))
            })?;
            for entry in entries {
                let entry = entry.map_err(|error| {
                    confinement_error(format!("could not list {}: {error}", folder.display()))
                })?;
                let name = entry.file_name();
                if names.iter().any(|secret| {
                    name.as_encoded_bytes()
                        .starts_with(secret.as_encoded_bytes())
                }) {
                    continue;
                }
                let path = entry.path();
                match std::fs::read_link(&path) {
                    Ok(target) => wrapped.extend([
                        "--symlink".to_string(),
                        path_arg(&target),
                        path_arg(&path),
                    ]),
                    Err(_) => {
                        wrapped.extend(["--dev-bind".to_string(), path_arg(&path), path_arg(&path)])
                    }
                }
            }
        }
        for (paths, bind) in [(shared, "--ro-bind"), (own, "--bind")] {
            for path in paths {
                if secrets.iter().any(|secret| path.starts_with(secret)) {
                    wrapped.extend([bind.to_string(), path_arg(path), path_arg(path)]);
                }
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

    use super::confinement_error;
    use crate::error::DaemonError;

    const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
    const PROFILE_HEADER: &str = ";; chariox kernel state confinement\n";

    /// Seatbelt denies the secrets by path, a secret's siblings that start with
    /// its name (Vault key envelopes) with it. The last matching rule wins, so
    /// the agent's own and the CLI's paths are allowed back after the denials.
    pub(super) fn wrap(
        program: String,
        args: Vec<String>,
        secrets: &[PathBuf],
        _home: &Path,
        own: &[PathBuf],
        shared: &[PathBuf],
    ) -> Result<(String, Vec<String>), DaemonError> {
        if !Path::new(SANDBOX_EXEC).exists() {
            return Err(confinement_error(format!(
                "agents run inside Seatbelt so they cannot read the kernel's secrets, and {SANDBOX_EXEC} is missing"
            )));
        }
        let mut profile = format!("{PROFILE_HEADER}(version 1)\n(allow default)\n");
        for secret in secrets {
            profile.push_str(&format!(
                "(deny file-read* file-write* (regex #\"^{}\"))\n",
                escaped(secret)
            ));
        }
        for (paths, operations) in [(own, "file-read* file-write*"), (shared, "file-read*")] {
            for path in paths {
                if secrets.iter().any(|secret| path.starts_with(secret)) {
                    profile.push_str(&format!(
                        "(allow {operations} (regex #\"^{}(/|$)\"))\n",
                        escaped(path)
                    ));
                }
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

    use super::confinement_error;
    use crate::error::DaemonError;

    pub(super) fn wrap(
        _program: String,
        _args: Vec<String>,
        _secrets: &[PathBuf],
        _home: &std::path::Path,
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
