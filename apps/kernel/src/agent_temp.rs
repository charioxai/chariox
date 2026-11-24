//! Each agent has a short private scratch path, scoped to its kernel and Room.
//! Its contents are trimmed when idle. The agent's process namespace exposes
//! its own folder, and ending the agent or Room removes the folder.
use crate::error::DaemonError;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long nothing may have written an item before it is cleared.
const IDLE_SECONDS_ENV: &str = "CHARIOX_TEMP_IDLE_SECONDS";
const DEFAULT_IDLE: Duration = Duration::from_secs(3600);
/// Entries looked at to decide an item is idle; a bigger tree counts as busy.
const SCAN_LIMIT: usize = 100_000;
const ID_LENGTH: usize = 12;
const ROOT_ID_LENGTH: usize = 16;
const TEMP_VARIABLES: [&str; 3] = ["TMPDIR", "TMP", "TEMP"];

pub(crate) fn idle_window() -> Duration {
    std::env::var(IDLE_SECONDS_ENV)
        .ok()
        .and_then(|value| value.parse().ok())
        .map_or(DEFAULT_IDLE, Duration::from_secs)
}

pub(crate) fn root() -> Option<PathBuf> {
    let home = crate::credential::chariox_home()?;
    let home = home.canonicalize().unwrap_or(home);
    let (host, port) = crate::config::kernel_listen_address();
    let parent = temp_parent(&home);
    Some(short_root(&parent, &home, &host, port))
}

#[cfg(test)]
fn temp_parent(home: &Path) -> PathBuf {
    // Unit-test homes own their scratch and remove it with the test fixture.
    home.to_path_buf()
}

#[cfg(not(test))]
fn temp_parent(_home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    return PathBuf::from("/private/tmp");
    #[cfg(all(unix, not(target_os = "macos")))]
    return PathBuf::from("/tmp");
    #[cfg(not(unix))]
    std::env::temp_dir()
}

fn short_root(parent: &Path, home: &Path, host: &str, port: u16) -> PathBuf {
    let key = Sha256::new()
        .chain_update(home.as_os_str().as_encoded_bytes())
        .chain_update([0])
        .chain_update(host)
        .chain_update(port.to_be_bytes())
        .finalize();
    parent.join(format!("cx-{}", &format!("{key:x}")[..ROOT_ID_LENGTH]))
}

pub(crate) fn room_dir(root: &Path, session_id: &str) -> PathBuf {
    root.join(short_name(session_id))
}

fn short_name(id: &str) -> String {
    format!("{:x}", Sha256::digest(id.as_bytes()))[..ID_LENGTH].to_owned()
}

pub(crate) fn dir(session_id: &str, agent_id: &str) -> Option<PathBuf> {
    Some(dir_in(&root()?, session_id, agent_id))
}

fn dir_in(root: &Path, session_id: &str, agent_id: &str) -> PathBuf {
    room_dir(root, session_id).join(short_name(agent_id))
}

/// Every directory leading to the agent path is private when it first exists.
/// Reused paths must be owned directories, so a precreated link is never followed.
fn private_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            check_private_directory(path)
        }
        Err(error) => Err(error),
    }
}

fn check_private_directory(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    #[cfg(unix)]
    let private = {
        use std::os::unix::fs::MetadataExt as _;
        metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o777 == 0o700
    };
    #[cfg(not(unix))]
    let private = true;
    if private && metadata.is_dir() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "expected an owned private directory",
        ))
    }
}

pub(crate) fn private_root() -> Option<PathBuf> {
    root().filter(|path| check_private_directory(path).is_ok())
}

#[cfg(test)]
pub(crate) fn environment(session_id: &str, agent_id: &str) -> Vec<(&'static str, String)> {
    let root = root().unwrap();
    let directory = create_agent_directory(&root, session_id, agent_id).unwrap();
    TEMP_VARIABLES
        .into_iter()
        .map(|name| (name, directory.display().to_string()))
        .collect()
}

/// Add the default scratch path to this launch, and return the directory that
/// a managed launch needs to mount after it covers the shared temp directory.
pub(crate) fn attach_environment(
    session_id: &str,
    agent_id: Option<&str>,
    environment: &mut BTreeMap<String, String>,
) -> Result<Option<PathBuf>, DaemonError> {
    let Some(agent_id) = agent_id else {
        return Ok(None);
    };
    if TEMP_VARIABLES
        .iter()
        .all(|name| environment.contains_key(*name))
    {
        return Ok(None);
    }
    let Some(root) = root() else {
        return Ok(None);
    };
    let directory = create_agent_directory(&root, session_id, agent_id)?;
    let path = directory.display().to_string();
    for name in TEMP_VARIABLES {
        environment
            .entry(name.to_string())
            .or_insert_with(|| path.clone());
    }
    crate::logging::info_with_fields(
        "agent_temp.create",
        "agent temporary directory ready",
        serde_json::json!({"session_id": session_id, "agent_id": agent_id, "path": path, "path_bytes": path.len()}),
    );
    Ok(Some(directory))
}

fn create_agent_directory(
    root: &Path,
    session_id: &str,
    agent_id: &str,
) -> Result<PathBuf, DaemonError> {
    let directory = dir_in(root, session_id, agent_id);
    for path in [
        root,
        directory.parent().expect("agent room directory"),
        &directory,
    ] {
        private_directory(path).map_err(|error| DaemonError::LocalTransport {
            operation: "create agent temporary directory",
            message: format!("Could not use {} ({error}). Restore the kernel's private temporary directory and retry.", path.display()),
        })?;
    }
    Ok(directory)
}

pub(crate) fn clear_after_restart() {
    // A successful in-place replacement adopts the old provider processes and
    // their mounts. Only a cold start owns the previous process's scratch.
    #[cfg(unix)]
    if crate::hot_reload::has_received_handover() {
        return;
    }
    if let Some(root) = private_root() {
        clear_directory(&root);
    }
}

pub(crate) fn clear_agent(session_id: &str, agent_id: &str) {
    if let Some(root) = private_root() {
        let room = room_dir(&root, session_id);
        if check_private_directory(&room).is_ok() {
            clear_directory(&dir_in(&root, session_id, agent_id));
        }
    }
}

/// A Room that ended or was archived leaves nothing in its temp folders.
pub(crate) fn clear_room(session_id: &str) {
    if let Some(root) = private_root() {
        clear_room_in(&root, session_id);
    }
}

fn clear_room_in(root: &Path, session_id: &str) {
    clear_directory(&room_dir(root, session_id));
}

fn clear_directory(path: &Path) {
    let Err(error) = std::fs::remove_dir_all(path) else {
        return;
    };
    if error.kind() != std::io::ErrorKind::NotFound {
        crate::logging::warn_with_fields(
            "agent_temp.clear",
            "the temporary directory was not cleared",
            serde_json::json!({"path": path.display().to_string(), "error": error.to_string()}),
        );
    }
}

/// What one trim removed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Trimmed {
    pub items: usize,
    pub bytes: u64,
}

/// Paths some process has open, works in or maps, and socket names whose bind
/// directory is unknown. Matching endpoints remain in use wherever they live.
#[derive(Default)]
pub(crate) struct HeldPaths {
    paths: BTreeSet<PathBuf>,
    relative_socket_names: BTreeSet<OsString>,
}

/// `None` where the platform cannot tell: nothing is then removed.
#[cfg(target_os = "linux")]
pub(crate) fn held_paths() -> Option<HeldPaths> {
    let mut held = HeldPaths::default();
    let mut tables = BTreeSet::new();
    for process in std::fs::read_dir("/proc").ok()?.flatten() {
        let name = process.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let base = process.path();
        if let Ok(cwd) = std::fs::read_link(base.join("cwd")) {
            held.paths.insert(cwd);
        }
        if let Ok(fds) = std::fs::read_dir(base.join("fd")) {
            let targets = fds
                .flatten()
                .filter_map(|fd| std::fs::read_link(fd.path()).ok())
                .collect::<Vec<_>>();
            if targets
                .iter()
                .any(|target| target.to_string_lossy().starts_with("socket:["))
            {
                match std::fs::read_link(base.join("ns/net")) {
                    Ok(network) => {
                        if !tables.contains(&network) {
                            match std::fs::read_to_string(base.join("net/unix")) {
                                Ok(table) => {
                                    for (_, path) in bound_socket_paths(&table) {
                                        if path.is_absolute() {
                                            held.paths.insert(path);
                                        } else if let Some(name) = path.file_name() {
                                            // The holder can change cwd or inherit the
                                            // descriptor, so its cwd cannot locate this.
                                            held.relative_socket_names.insert(name.to_owned());
                                        }
                                    }
                                    tables.insert(network);
                                }
                                Err(_) if !base.exists() => {}
                                Err(_) => return None,
                            }
                        }
                    }
                    Err(_) if base.exists() => return None,
                    Err(_) => {}
                }
            }
            held.paths
                .extend(targets.into_iter().filter(|path| path.is_absolute()));
        }
        if let Ok(maps) = std::fs::read_to_string(base.join("maps")) {
            held.paths.extend(
                maps.lines()
                    .filter_map(|line| line.split_once('/'))
                    .map(|(_, path)| PathBuf::from(format!("/{path}"))),
            );
        }
    }
    Some(held)
}

/// The inode and reported name of each bound socket in a `/proc/net/unix` table.
#[cfg(target_os = "linux")]
fn bound_socket_paths(table: &str) -> impl Iterator<Item = (u64, PathBuf)> + '_ {
    table.lines().filter_map(|line| {
        let mut rest = line;
        let mut inode = None;
        // Seven fixed fields precede the path (the seventh is the inode);
        // preserve spaces inside the path.
        for field in 0..7 {
            rest = rest.trim_start();
            let end = rest.find(char::is_whitespace)?;
            if field == 6 {
                inode = rest[..end].parse().ok();
            }
            rest = &rest[end..];
        }
        let path = rest.trim_start();
        // `@...` can name an abstract socket or a relative filesystem socket.
        // Preserve matching filesystem endpoints when the table is ambiguous.
        if path.is_empty() {
            return None;
        }
        Some((inode?, PathBuf::from(path)))
    })
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn held_paths() -> Option<HeldPaths> {
    None
}

/// The newest write below `path` (its own time for a file), or `None` when
/// the tree is too big to look through or contains a possibly live socket:
/// such an item is treated as busy.
fn last_written(path: &Path, budget: &mut usize, held: &HeldPaths) -> Option<SystemTime> {
    *budget = budget.checked_sub(1)?;
    let meta = std::fs::symlink_metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if meta.file_type().is_socket()
            && path
                .file_name()
                .is_some_and(|name| held.relative_socket_names.contains(name))
        {
            // Without the bind directory, a matching stale socket is
            // indistinguishable from the live endpoint. Keep both until closed.
            return None;
        }
    }
    let mut newest = meta.modified().ok()?;
    if meta.is_dir() {
        for child in std::fs::read_dir(path).ok()?.flatten() {
            newest = newest.max(last_written(&child.path(), budget, held)?);
        }
    }
    Some(newest)
}

fn in_use(item: &Path, held: &BTreeSet<PathBuf>) -> bool {
    held.range(item.to_path_buf()..)
        .take_while(|path| path.starts_with(item))
        .next()
        .is_some()
}

/// Clears the items of every agent folder under `root` that nothing wrote for
/// `idle` and no process in `held` has open or works in. Roots stay until the
/// Room ends: a process may first use its inherited TMPDIR much later.
pub(crate) fn trim(root: &Path, now: SystemTime, idle: Duration, held: &HeldPaths) -> Trimmed {
    let mut trimmed = Trimmed::default();
    let Ok(root) = root.canonicalize() else {
        return trimmed;
    };
    // /proc resolves cwd and fd paths, while socket names can retain aliases.
    // Resolve both sides, but never resolve individual scratch entries: a
    // symlink entry is itself disposable, independently of its target.
    let held_paths = held
        .paths
        .iter()
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
        .collect();
    let directories = |path: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect()
    };
    for room in directories(&root) {
        for agent in directories(&room) {
            let Ok(items) = std::fs::read_dir(&agent) else {
                continue;
            };
            for item in items.flatten().map(|item| item.path()) {
                let mut budget = SCAN_LIMIT;
                let Some(written) = last_written(&item, &mut budget, held) else {
                    continue;
                };
                let idle_for = now.duration_since(written).unwrap_or_default();
                if idle_for < idle || in_use(&item, &held_paths) {
                    continue;
                }
                let mut sizing = SCAN_LIMIT;
                let bytes = crate::disk_scan::size_of(&item, &mut sizing);
                let removed = match std::fs::symlink_metadata(&item) {
                    Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&item),
                    Ok(_) => std::fs::remove_file(&item),
                    Err(error) => Err(error),
                };
                if removed.is_ok() {
                    trimmed.items += 1;
                    trimmed.bytes += bytes;
                }
            }
        }
    }
    trimmed
}

#[cfg(test)]
mod tests;
