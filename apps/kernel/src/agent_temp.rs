//! Each agent's scratch folder: one short, kernel-owned path per Room and agent
//! inside the kernel's home (`<home>/t/<room>/<agent>`), set as TMPDIR, TMP and
//! TEMP of everything the kernel starts for the agent. The folder is cleared by
//! inactivity, item by item, and never while a process has the item open or
//! works inside it. The kernel only deletes here, never in a workspace.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long nothing may have written an item before it is cleared.
const IDLE_SECONDS_ENV: &str = "CHARIOX_TEMP_IDLE_SECONDS";
const DEFAULT_IDLE: Duration = Duration::from_secs(3600);
/// Entries looked at to decide an item is idle; a bigger tree counts as busy.
const SCAN_LIMIT: usize = 100_000;
const ID_LENGTH: usize = 8;

pub(crate) fn idle_window() -> Duration {
    std::env::var(IDLE_SECONDS_ENV)
        .ok()
        .and_then(|value| value.parse().ok())
        .map_or(DEFAULT_IDLE, Duration::from_secs)
}

pub(crate) fn root() -> Option<PathBuf> {
    crate::credential::chariox_home().map(|home| home.join("t"))
}

/// A short stable name for an id: the characters of its unique part, so a
/// socket inside the folder stays well under the 107-byte path limit and the
/// path survives a restart without a table.
fn short(id: &str) -> String {
    id.trim_start_matches("agent-")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(ID_LENGTH)
        .collect()
}

pub(crate) fn room_dir(root: &Path, session_id: &str) -> PathBuf {
    // Timestamp Room IDs vary in their low bits; a prefix merges nearby Rooms.
    root.join(
        session_id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>(),
    )
}

pub(crate) fn dir(session_id: &str, agent_id: &str) -> Option<PathBuf> {
    Some(dir_in(&root()?, session_id, agent_id))
}

fn dir_in(root: &Path, session_id: &str, agent_id: &str) -> PathBuf {
    room_dir(root, session_id).join(short(agent_id))
}

/// TMPDIR, TMP and TEMP for the agent, with its folder created (private).
/// Without a home to put it in, the agent keeps the system temp folder.
pub(crate) fn environment(session_id: &str, agent_id: &str) -> Vec<(&'static str, String)> {
    root().map_or_else(Vec::new, |root| environment_in(&root, session_id, agent_id))
}

fn environment_in(root: &Path, session_id: &str, agent_id: &str) -> Vec<(&'static str, String)> {
    let dir = dir_in(root, session_id, agent_id);
    if let Err(error) = create_private(&dir) {
        crate::logging::warn_with_fields(
            "agent_temp.create",
            "the agent's temp folder could not be created; it keeps the system temp folder",
            serde_json::json!({"path": dir.display().to_string(), "error": error.to_string()}),
        );
        return Vec::new();
    }
    let path = dir.display().to_string();
    ["TMPDIR", "TMP", "TEMP"]
        .into_iter()
        .map(|name| (name, path.clone()))
        .collect()
}

fn create_private(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for level in dir.ancestors().take(2) {
            std::fs::set_permissions(level, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// A Room that ended or was archived leaves nothing in its temp folders.
pub(crate) fn clear_room(session_id: &str) {
    if let Some(root) = root() {
        clear_room_in(&root, session_id);
    }
}

fn clear_room_in(root: &Path, session_id: &str) {
    let room = room_dir(root, session_id);
    if room == root || !room.starts_with(root) {
        return;
    }
    if let Err(error) = std::fs::remove_dir_all(&room) {
        if error.kind() != std::io::ErrorKind::NotFound {
            crate::logging::warn_with_fields(
                "agent_temp.clear_room",
                "the ended Room's temp folder was not cleared",
                serde_json::json!({"path": room.display().to_string(), "error": error.to_string()}),
            );
        }
    }
}

/// What one trim removed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Trimmed {
    pub items: usize,
    pub bytes: u64,
}

/// Paths some process has open, works in or maps. An item with one inside it
/// is in use. `None` where the platform cannot tell: nothing is then removed.
#[cfg(target_os = "linux")]
pub(crate) fn held_paths() -> Option<BTreeSet<PathBuf>> {
    let mut held = BTreeSet::new();
    let mut tables = BTreeMap::<PathBuf, BTreeMap<u64, PathBuf>>::new();
    for process in std::fs::read_dir("/proc").ok()?.flatten() {
        let name = process.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let base = process.path();
        if let Ok(cwd) = std::fs::read_link(base.join("cwd")) {
            held.insert(cwd);
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
                        if !tables.contains_key(&network) {
                            match std::fs::read_to_string(base.join("net/unix")) {
                                Ok(table) => {
                                    held.extend(bound_socket_paths(&table).filter_map(
                                        |(_, path)| path.is_absolute().then_some(path),
                                    ));
                                    tables.insert(network.clone(), relative_socket_names(&table));
                                }
                                Err(_) if !base.exists() => {}
                                Err(_) => return None,
                            }
                        }
                        // A name bound relatively is kept in the table as
                        // given: it lives where the holder was when it bound.
                        if let (Some(names), Some(cwd)) = (
                            tables.get(&network),
                            std::fs::read_link(base.join("cwd")).ok(),
                        ) {
                            for target in &targets {
                                if let Some(name) = socket_inode(target).and_then(|i| names.get(&i))
                                {
                                    held.insert(cwd.join(name));
                                }
                            }
                        }
                    }
                    Err(_) if base.exists() => return None,
                    Err(_) => {}
                }
            }
            held.extend(targets);
        }
        if let Ok(maps) = std::fs::read_to_string(base.join("maps")) {
            held.extend(
                maps.lines()
                    .filter_map(|line| line.split_once('/'))
                    .map(|(_, path)| PathBuf::from(format!("/{path}"))),
            );
        }
    }
    Some(held)
}

/// The inode and file-system name of each bound socket in a `/proc/net/unix`
/// table.
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
        // An abstract name (`@...`) is not in the file system.
        if path.is_empty() || path.starts_with('@') {
            return None;
        }
        Some((inode?, PathBuf::from(path)))
    })
}

/// The names sockets were bound with that do not start at the root, by inode.
#[cfg(target_os = "linux")]
fn relative_socket_names(table: &str) -> BTreeMap<u64, PathBuf> {
    bound_socket_paths(table)
        .filter(|(_, path)| path.is_relative())
        .collect()
}

#[cfg(target_os = "linux")]
fn socket_inode(target: &Path) -> Option<u64> {
    target
        .to_str()?
        .strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn held_paths() -> Option<BTreeSet<PathBuf>> {
    None
}

/// The newest write below `path` (its own time for a file), or `None` when
/// the tree is too big to look through: such an item is treated as busy.
fn last_written(path: &Path, budget: &mut usize) -> Option<SystemTime> {
    *budget = budget.checked_sub(1)?;
    let meta = std::fs::symlink_metadata(path).ok()?;
    let mut newest = meta.modified().ok()?;
    if meta.is_dir() {
        for child in std::fs::read_dir(path).ok()?.flatten() {
            newest = newest.max(last_written(&child.path(), budget)?);
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
pub(crate) fn trim(
    root: &Path,
    now: SystemTime,
    idle: Duration,
    held: &BTreeSet<PathBuf>,
) -> Trimmed {
    let mut trimmed = Trimmed::default();
    let directories = |path: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect()
    };
    for room in directories(root) {
        for agent in directories(&room) {
            let Ok(items) = std::fs::read_dir(&agent) else {
                continue;
            };
            for item in items.flatten().map(|item| item.path()) {
                let mut budget = SCAN_LIMIT;
                let Some(written) = last_written(&item, &mut budget) else {
                    continue;
                };
                let idle_for = now.duration_since(written).unwrap_or_default();
                if idle_for < idle || in_use(&item, held) {
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
