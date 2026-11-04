//! Each agent's scratch folder: one short, kernel-owned path per Room and agent
//! inside the kernel's home (`<home>/t/<room>/<agent>`), set as TMPDIR, TMP and
//! TEMP of everything the kernel starts for the agent. The folder is cleared by
//! inactivity, item by item, and never while a process has the item open or
//! works inside it. The kernel only deletes here, never in a workspace.
use std::collections::BTreeSet;
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
    root.join(short(session_id))
}

pub(crate) fn dir(session_id: &str, agent_id: &str) -> Option<PathBuf> {
    Some(room_dir(&root()?, session_id).join(short(agent_id)))
}

/// TMPDIR, TMP and TEMP for the agent, with its folder created (private).
/// Without a home to put it in, the agent keeps the system temp folder.
pub(crate) fn environment(session_id: &str, agent_id: &str) -> Vec<(&'static str, String)> {
    let Some(dir) = dir(session_id, agent_id) else {
        return Vec::new();
    };
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
    let Some(root) = root() else {
        return;
    };
    let room = room_dir(&root, session_id);
    if short(session_id).is_empty() || !room.starts_with(&root) {
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
            held.extend(
                fds.flatten()
                    .filter_map(|fd| std::fs::read_link(fd.path()).ok()),
            );
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
/// `idle` and no process in `held` has open or works in. A folder left empty
/// goes with its last item.
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
            // remove_dir only removes an empty folder.
            if !in_use(&agent, held) && std::fs::remove_dir(&agent).is_ok() {
                let _ = std::fs::remove_dir(&room);
            }
        }
    }
    trimmed
}

#[cfg(test)]
mod tests;
