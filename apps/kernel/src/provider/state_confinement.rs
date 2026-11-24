//! Agents never see the kernel's private state. Every provider run and every
//! command the kernel starts for an agent runs where the kernel's secrets are
//! out of reach: its state folder (database, relay credential, every provider
//! profile), Vault, credentials, machine identity, relay pairing and history.
//! What is the agent's own stays: the paths its launch names (working folder,
//! provider profile, scratch folder, program) are mounted back, and the
//! chariox CLI keeps the kernel registry read-only. The provider receives its
//! own CLI credential in its launch environment.
//!
//! Secrets live in folders of their own, and exactly those folders are
//! covered, matched by path component: a folder that only shares a name prefix
//! with one, or a file beside one, is never covered. A covered folder is empty
//! and read-only, so a write into it fails instead of vanishing, and a secret
//! the kernel replaces or creates later stays covered. A configured secret file
//! outside the kernel home must sit in a folder of its own, or the kernel
//! refuses to start. Linux covers the folders in a mount namespace made by
//! Bubblewrap, which unprivileged kernels may use where distributions restrict
//! user namespaces. macOS denies them with a Seatbelt profile. A kernel that
//! cannot confine refuses to start the process; nothing runs unconfined.
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

/// Secret folders the kernel's configuration may place anywhere.
static CONFIGURED_FOLDERS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// A configured secret file, known with the companions its writer keeps beside it.
enum SecretFile {
    /// Transferred key envelopes and writes in progress.
    Vault(PathBuf),
    /// SQLite's journal and shared-memory files.
    Database(PathBuf),
}

const DATABASE_COMPANION_SUFFIXES: &[&str] = &["-wal", "-shm", "-journal"];

impl SecretFile {
    fn path(&self) -> &Path {
        match self {
            Self::Vault(path) | Self::Database(path) => path,
        }
    }

    fn setting(&self) -> &'static str {
        match self {
            Self::Vault(_) => "credential_vault.path",
            Self::Database(_) => "history.operational.path",
        }
    }

    /// Whether `name` in the file's folder belongs to it.
    fn owns(&self, name: &str) -> bool {
        let Some(own) = self.path().file_name().and_then(|own| own.to_str()) else {
            return false;
        };
        name == own
            || match self {
                Self::Vault(_) => crate::secret::is_vault_companion(own, name),
                Self::Database(_) => name
                    .strip_prefix(own)
                    .is_some_and(|suffix| DATABASE_COMPANION_SUFFIXES.contains(&suffix)),
            }
    }

    /// The file's folder, once it is sure to hold nothing but the file and its
    /// companions.
    fn own_folder(&self) -> Result<PathBuf, DaemonError> {
        let path = self.path();
        let folder = path.parent().unwrap_or(Path::new("/")).to_path_buf();
        let others = std::fs::read_dir(&folder)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| !self.owns(name))
            .take(3)
            .collect::<Vec<_>>();
        if others.is_empty() {
            return Ok(folder);
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        Err(confinement_error(format!(
            "{} ({}) shares {} with other files ({}), so agents could not be kept away from it without hiding them; move it into a folder of its own, e.g. {}, and set {} to the new path",
            path.display(),
            self.setting(),
            folder.display(),
            others.join(", "),
            folder.join("chariox-secret").join(&*name).display(),
            self.setting(),
        )))
    }
}

/// The kernel's secret folders as they are on disk now: outermost first, none
/// inside another.
struct Secrets {
    folders: Vec<PathBuf>,
}

impl Secrets {
    fn cover(&self, path: &Path) -> bool {
        self.folders.iter().any(|folder| path.starts_with(folder))
    }
}

/// Records the secret folders the configuration names, and creates every
/// secret folder, so a secret the kernel writes later is covered from the
/// start. Refuses a configured secret file that does not sit in a folder of
/// its own, and a secret folder that holds the user's home.
pub(crate) fn configure(config: &crate::config::DaemonConfig) -> Result<(), DaemonError> {
    let home_folders = home_secret_folders();
    // The durable state's folder also holds the relay credential, provider
    // accounts and other private runtime state.
    let mut folders = [
        Some(config.private_runtime_state_root()),
        Some(config.session_history_root()),
        config.publication_control_state_root.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    for file in [
        SecretFile::Vault(crate::secret::normalize_vault_path(PathBuf::from(
            &config.user_config.credential_vault.path,
        ))),
        SecretFile::Database(config.operational_history_path()),
    ] {
        let covered = home_folders
            .iter()
            .chain(&folders)
            .any(|folder| file.path().starts_with(folder));
        if !covered {
            folders.push(file.own_folder()?);
        }
    }
    if let Some(folder) = folders.iter().find(|folder| is_shared_folder(folder)) {
        return Err(confinement_error(format!(
            "{} holds the kernel's private state and the user's own files, so agents could not be kept away from one without hiding the other; give the kernel's state a folder of its own",
            folder.display()
        )));
    }
    for folder in home_folders.iter().chain(&folders) {
        if !folder.exists() {
            private_folder(folder).map_err(|error| {
                confinement_error(format!(
                    "could not create the kernel's secret folder {}: {error}",
                    folder.display()
                ))
            })?;
        }
    }
    // A running provider can receive a workflow turn later. Keep this mount
    // present from launch so it sees new files without restarting the provider.
    let instructions = crate::config::DaemonConfig::workflow_instruction_mount();
    private_folder(&instructions).map_err(|error| {
        confinement_error(format!(
            "could not create workflow instruction directory {}: {error}",
            instructions.display()
        ))
    })?;
    let _ = CONFIGURED_FOLDERS.set(folders);
    Ok(())
}

/// The secret folders of the kernel home and its default state folder.
fn home_secret_folders() -> Vec<PathBuf> {
    let home = crate::config::default_config_dir();
    SECRET_HOME_FOLDERS
        .iter()
        .map(|folder| home.join(folder))
        .chain([crate::config::default_state_dir()])
        .collect()
}

/// The filesystem root, the user's home or a folder above it.
fn is_shared_folder(folder: &Path) -> bool {
    folder.parent().is_none()
        || std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .is_some_and(|home| Path::new(&home).starts_with(folder))
}

fn private_folder(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// This store is mounted back even when its parent is private. Private state
/// must never be configured inside the mount.
pub(crate) fn validate_skill_package_store(
    config: &crate::config::DaemonConfig,
    store: &Path,
) -> Result<(), DaemonError> {
    let vault = crate::secret::normalize_vault_path(PathBuf::from(
        &config.user_config.credential_vault.path,
    ));
    for path in [
        Some(config.private_runtime_state_root()),
        Some(config.session_history_root()),
        config.publication_control_state_root.clone(),
        Some(config.operational_history_path()),
        Some(vault),
    ]
    .into_iter()
    .flatten()
    {
        if path.ancestors().any(|ancestor| {
            ancestor
                .canonicalize()
                .is_ok_and(|ancestor| ancestor == store)
        }) {
            return Err(confinement_error(format!(
                "{} is inside the skill package store; move the kernel's private state outside {}",
                path.display(),
                store.display(),
            )));
        }
    }
    Ok(())
}

/// The launch, run so that it and everything it starts cannot read the kernel's
/// secrets. An endpoint the kernel does not start runs no process here.
pub(crate) fn confine_launch(
    mut launch: ProviderLaunchResult,
    read_only: &[PathBuf],
) -> Result<ProviderLaunchResult, DaemonError> {
    let Some(program) = launch.pty_program.take() else {
        return Ok(launch);
    };
    let (program, args) = confine_command_with_read_only(
        program,
        std::mem::take(&mut launch.pty_args),
        &launch.pty_env,
        launch.working_directory.iter().cloned().collect(),
        read_only,
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
    own: Vec<PathBuf>,
) -> Result<(String, Vec<String>), DaemonError> {
    confine_command_with_read_only(program, args, environment, own, &[])
}

fn confine_command_with_read_only(
    program: String,
    args: Vec<String>,
    environment: &BTreeMap<String, String>,
    own: Vec<PathBuf>,
    read_only: &[PathBuf],
) -> Result<(String, Vec<String>), DaemonError> {
    let secrets = secrets();
    let named = own
        .into_iter()
        .map(|path| {
            (
                "its working folder or a file handed to it".to_string(),
                path,
            )
        })
        .chain(OWN_PATH_ENVIRONMENT.iter().filter_map(|name| {
            environment
                .get(*name)
                .map(|path| (format!("its {name}"), PathBuf::from(path)))
        }))
        .chain(
            Path::new(&program)
                .parent()
                .map(|folder| (format!("the folder of {program}"), folder.to_path_buf())),
        )
        .collect::<Vec<_>>();
    if let Some((name, path)) = named.iter().find(|(_, path)| {
        path.canonicalize()
            .is_ok_and(|path| secrets.folders.contains(&path))
    }) {
        return Err(confinement_error(format!(
            "{} ({name}) is the kernel's private folder, so no agent process may work in it; use a folder outside it",
            path.display()
        )));
    }
    let own = existing_canonical(named.into_iter().map(|(_, path)| path).collect());
    let mut shared = existing_canonical(cli_paths());
    shared.retain(|path| secrets.cover(path));
    for path in read_only {
        shared.push(path.canonicalize().map_err(|error| {
            confinement_error(format!(
                "could not make {} readable: {error}",
                path.display()
            ))
        })?);
    }
    platform::wrap(program, args, &secrets, &own, &shared)
}

/// The length of a confinement prefix at the start of `args`, through the program.
pub(crate) fn confinement_prefix_len(args: &[String]) -> Option<usize> {
    platform::prefix_len(args)
}

/// The process that runs the program of a confined launch led by `leader`,
/// once it has started.
pub(crate) fn confined_program(leader: u32) -> Option<u32> {
    platform::confined_program(leader)
}

/// The signal that ended the program of a confined launch, read from its
/// leader's exit status.
#[cfg(unix)]
pub(crate) fn confined_program_signal(status: &std::process::ExitStatus) -> Option<i32> {
    platform::confined_program_signal(status)
}

/// Exactly the declared secret folders that exist now, by their real paths.
fn secrets() -> Secrets {
    let mut folders = home_secret_folders()
        .into_iter()
        .chain(CONFIGURED_FOLDERS.get().into_iter().flatten().cloned())
        .chain(crate::agent_temp::private_root())
        .filter_map(|folder| folder.canonicalize().ok())
        .filter(|folder| folder.is_dir())
        .collect::<Vec<_>>();
    folders.sort_by_key(|path| path.components().count());
    let mut outermost: Vec<PathBuf> = Vec::new();
    for folder in folders {
        if !outermost.iter().any(|outer| folder.starts_with(outer)) {
            outermost.push(folder);
        }
    }
    Secrets { folders: outermost }
}

/// What the chariox CLI reads to reach this kernel.
fn cli_paths() -> Vec<PathBuf> {
    vec![crate::config::default_config_dir()
        .join("kernels")
        .join("active")]
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

    /// Bubblewrap, the setpriv that ends the program with it, and the env that
    /// keeps the stop signals for the program.
    struct Sandbox {
        bubblewrap: PathBuf,
        setpriv: PathBuf,
        env: PathBuf,
    }

    static WORKING_SANDBOX: OnceLock<Sandbox> = OnceLock::new();

    /// Bubblewrap loses its --die-with-parent for the program it starts, so
    /// setpriv sets the program's parent-death signal itself. The kernel stops
    /// a confined process by stopping its sandbox; the program must not
    /// outlive it.
    const PROGRAM_DEATH: [&str; 3] = ["--pdeathsig", "KILL", "--"];

    /// Bubblewrap shares its program's session and process group, and it dies
    /// of the signals that ask a program to stop. A Stop's Ctrl-C, a hang-up
    /// or a SIGTERM to the session would end the sandbox, and with it the
    /// program at once, instead of letting the program handle them. Bubblewrap
    /// ignores them; the program gets them back. A SIGKILL ends both.
    const SANDBOX_SIGNALS: [&str; 2] = ["--ignore-signal=HUP,INT,QUIT,TERM", "--"];
    const PROGRAM_SIGNALS: [&str; 2] = ["--default-signal=HUP,INT,QUIT,TERM", "--"];

    /// Each secret folder is covered by an empty read-only one. The agent's
    /// own paths inside a covered folder are mounted back.
    pub(super) fn wrap(
        program: String,
        args: Vec<String>,
        secrets: &Secrets,
        own: &[PathBuf],
        shared: &[PathBuf],
    ) -> Result<(String, Vec<String>), DaemonError> {
        let sandbox = working_sandbox()?;
        let mut wrapped = SANDBOX_SIGNALS.map(str::to_string).to_vec();
        wrapped.extend([
            path_arg(&sandbox.bubblewrap),
            "--dev-bind".to_string(),
            "/".to_string(),
            "/".to_string(),
        ]);
        for folder in &secrets.folders {
            wrapped.extend(["--tmpfs".to_string(), path_arg(folder)]);
        }
        for path in own.iter().filter(|path| secrets.cover(path)) {
            wrapped.extend(["--bind".to_string(), path_arg(path), path_arg(path)]);
        }
        for path in shared {
            wrapped.extend(["--ro-bind".to_string(), path_arg(path), path_arg(path)]);
        }
        // A write into a covered folder fails instead of vanishing with the
        // process; the paths mounted back keep their own mode.
        for folder in &secrets.folders {
            wrapped.extend(["--remount-ro".to_string(), path_arg(folder)]);
        }
        wrapped.extend([
            "--setenv".to_string(),
            STATE_CONFINEMENT_MARKER_ENV.to_string(),
            "1".to_string(),
            "--".to_string(),
            path_arg(&sandbox.setpriv),
        ]);
        wrapped.extend(PROGRAM_DEATH.map(str::to_string));
        wrapped.push(path_arg(&sandbox.env));
        wrapped.extend(PROGRAM_SIGNALS.map(str::to_string));
        wrapped.push(program);
        wrapped.extend(args);
        Ok((path_arg(&sandbox.env), wrapped))
    }

    pub(super) fn prefix_len(args: &[String]) -> Option<usize> {
        let separator = args
            .windows(4)
            .position(|window| window == ["--setenv", STATE_CONFINEMENT_MARKER_ENV, "1", "--"])?
            + 3;
        let death = separator + 2;
        let signals = death + PROGRAM_DEATH.len() + 1;
        let program = signals + PROGRAM_SIGNALS.len();
        (args.get(death..death + PROGRAM_DEATH.len())? == PROGRAM_DEATH
            && args.get(signals..program)? == PROGRAM_SIGNALS
            && program < args.len())
        .then_some(program + 1)
    }

    /// Bubblewrap stays the leader and runs the program as its only child.
    pub(super) fn confined_program(leader: u32) -> Option<u32> {
        let children =
            std::fs::read_to_string(format!("/proc/{leader}/task/{leader}/children")).ok()?;
        let mut children = children.split_whitespace();
        match (children.next(), children.next()) {
            (Some(program), None) => program.parse().ok(),
            _ => None,
        }
    }

    pub(super) fn confined_program_signal(status: &std::process::ExitStatus) -> Option<i32> {
        use std::os::unix::process::ExitStatusExt as _;
        // Bubblewrap cannot re-raise its program's signal n and exits 128 + n
        // instead, so a program that exits 129..=192 itself is named as that
        // signal too: the convention a shell's $? follows.
        status.signal().or_else(|| {
            status
                .code()
                .map(|code| code - 128)
                .filter(|signal| (1..=libc::SIGRTMAX()).contains(signal))
        })
    }

    fn path_arg(path: &Path) -> String {
        path.display().to_string()
    }

    fn on_path(name: &str) -> Option<PathBuf> {
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    }

    fn in_bubblewrap_profile(profile: &str) -> bool {
        profile.split_whitespace().next().is_some_and(|name| {
            name.split("//&")
                .any(|part| matches!(part, "bwrap" | "unpriv_bwrap"))
        })
    }

    fn namespace_probe_refusal(stderr: &str, nested: bool) -> Option<&'static str> {
        let denied = stderr.contains("bwrap:")
            && stderr.contains("namespace")
            && (stderr.contains("Operation not permitted")
                || stderr.contains("Permission denied")
                || stderr.contains("No permissions to create"));
        if !denied {
            return None;
        }
        Some(if nested {
            "provider confinement could not start: the kernel is already inside Bubblewrap (nested confinement) and cannot create its own mount namespace. For a drill, ask the host operator to start a dedicated drill kernel as a host service with its own Chariox home, or use a dedicated lab machine"
        } else {
            "provider confinement could not start: Linux denied Bubblewrap a mount namespace. Ask the host operator to check the AppArmor policy for unprivileged user namespaces and the kernel service's namespace restrictions and capabilities, then retry from an authorized host service or dedicated lab machine"
        })
    }

    /// Bubblewrap, setpriv and env from PATH, once they have run a program in
    /// a namespace on this machine.
    fn working_sandbox() -> Result<&'static Sandbox, DaemonError> {
        if let Some(sandbox) = WORKING_SANDBOX.get() {
            return Ok(sandbox);
        }
        let bubblewrap = on_path("bwrap").ok_or_else(|| {
            confinement_error(
                "agents run inside Bubblewrap so they cannot read the kernel's secrets, and bwrap is not installed; install bubblewrap (apt install bubblewrap, dnf install bubblewrap) and retry",
            )
        })?;
        let setpriv = on_path("setpriv").ok_or_else(|| {
            confinement_error(
                "agents run inside Bubblewrap and end with it through setpriv, which is not installed; install util-linux (apt install util-linux, dnf install util-linux-core) and retry",
            )
        })?;
        let env = on_path("env").ok_or_else(|| {
            confinement_error(
                "agents run inside Bubblewrap and get their stop signals through env, which is not installed; install coreutils 8.31 or later and retry",
            )
        })?;
        let output = crate::process_spawn::command(&env)
            .args(SANDBOX_SIGNALS)
            .arg(&bubblewrap)
            .args(["--dev-bind", "/", "/", "--"])
            .arg(&setpriv)
            .args(PROGRAM_DEATH)
            .arg(&env)
            .args(PROGRAM_SIGNALS)
            .arg(&bubblewrap)
            .arg("--version")
            .bounded_output()
            .map_err(|error| {
                confinement_error(format!("could not run {}: {error}", bubblewrap.display()))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let profile = std::fs::read_to_string("/proc/self/attr/current").unwrap_or_default();
            let nested = std::env::var_os(STATE_CONFINEMENT_MARKER_ENV).is_some()
                || in_bubblewrap_profile(&profile);
            if let Some(reason) = namespace_probe_refusal(&stderr, nested) {
                return Err(confinement_error(reason));
            }
            return Err(confinement_error(format!(
                "{} with {} and {} could not run the provider confinement probe ({}); check the Bubblewrap and util-linux installation and that env is from coreutils 8.31 or later",
                bubblewrap.display(),
                setpriv.display(),
                env.display(),
                stderr.trim()
            )));
        }
        Ok(WORKING_SANDBOX.get_or_init(|| Sandbox {
            bubblewrap,
            setpriv,
            env,
        }))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn namespace_probe_names_nested_confinement_and_the_supported_drill_path() {
            assert!(in_bubblewrap_profile("bwrap//&unpriv_bwrap (enforce)"));
            assert!(!in_bubblewrap_profile("unconfined"));
            let reason = namespace_probe_refusal(
                "bwrap: Creating new namespace failed: Operation not permitted",
                true,
            )
            .unwrap();
            assert!(reason.contains("nested confinement"));
            assert!(reason.contains("dedicated drill kernel as a host service"));
        }

        #[test]
        fn namespace_probe_distinguishes_host_policy_from_other_startup_failures() {
            let reason =
                namespace_probe_refusal("bwrap: No permissions to create a new namespace", false)
                    .unwrap();
            assert!(reason.contains("AppArmor") && reason.contains("capabilities"));
            assert!(namespace_probe_refusal("env: unrecognized option", false).is_none());
            assert!(
                namespace_probe_refusal("bwrap: Can't bind mount: Permission denied", true)
                    .is_none()
            );
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::{Path, PathBuf};

    use super::{confinement_error, Secrets};
    use crate::error::DaemonError;

    const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
    const PROFILE_HEADER: &str = ";; chariox kernel state confinement\n";

    /// Seatbelt denies the secret folders by path. The last matching
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
        for path in own.iter().filter(|path| secrets.cover(path)) {
            profile.push_str(&format!(
                "(allow file-read* file-write* (regex #\"^{}(/|$)\"))\n",
                escaped(path)
            ));
        }
        for path in shared {
            let path = escaped(path);
            profile.push_str(&format!(
                "(allow file-read* (regex #\"^{path}(/|$)\"))\n(deny file-write* (regex #\"^{path}(/|$)\"))\n",
            ));
        }
        let mut wrapped = vec!["-p".to_string(), profile, program];
        wrapped.extend(args);
        Ok((SANDBOX_EXEC.to_string(), wrapped))
    }

    pub(super) fn prefix_len(args: &[String]) -> Option<usize> {
        (args.len() >= 3 && args[0] == "-p" && args[1].starts_with(PROFILE_HEADER)).then_some(3)
    }

    /// sandbox-exec becomes the program it confines.
    pub(super) fn confined_program(leader: u32) -> Option<u32> {
        Some(leader)
    }

    pub(super) fn confined_program_signal(status: &std::process::ExitStatus) -> Option<i32> {
        use std::os::unix::process::ExitStatusExt as _;
        status.signal()
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

    pub(super) fn confined_program(_leader: u32) -> Option<u32> {
        None
    }

    #[cfg(unix)]
    pub(super) fn confined_program_signal(_status: &std::process::ExitStatus) -> Option<i32> {
        None
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "state_confinement_tests.rs"]
pub(crate) mod tests;
