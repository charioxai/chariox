//! MP-08 / MP-09 / MP-10 / MP-11: occupancy samples for the ordinary process feed.
//! Walks run on a blocking worker, at most one at a time, every thirty seconds.
//! Readers only copy the last sample; incomplete walks remain unknown.
use super::process_resources::{SessionProcess, SessionProcessOwner};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(30);
const RETAIN: Duration = Duration::from_secs(120);
const ENTRIES: usize = 200_000;
const WALK_TIME: Duration = Duration::from_millis(500);

/// Shared directories and open files keep their identity across owners, so a
/// parent row can count them once. `path` is absent for an unlinked open file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiskUsageEntry {
    pub id: String,
    pub path: Option<String>,
    pub used_bytes: Option<u64>,
    pub sampled_at_ms: u64,
    pub owner: Option<SessionProcessOwner>,
    pub process_id: Option<String>,
}

pub(crate) struct Directory {
    pub path: PathBuf,
    pub owner: Option<SessionProcessOwner>,
}

#[derive(Clone, Default)]
pub(crate) struct DiskUsageStore(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    busy: bool,
    pruned: Option<Instant>,
    rooms: BTreeMap<String, Sample>,
}
struct Sample {
    requested: Instant,
    checked: Option<Instant>,
    entries: Vec<DiskUsageEntry>,
}

impl DiskUsageStore {
    pub(crate) fn read(
        &self,
        room: &str,
        collect: impl FnOnce(&str) -> (Vec<Directory>, Vec<SessionProcess>) + Send + 'static,
    ) -> Vec<DiskUsageEntry> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.pruned.is_none_or(|at| at.elapsed() >= INTERVAL) {
            state
                .rooms
                .retain(|_, sample| sample.requested.elapsed() < RETAIN);
            state.pruned = Some(Instant::now());
        }
        let cached = state
            .rooms
            .entry(room.to_string())
            .or_insert_with(|| Sample {
                requested: Instant::now(),
                checked: None,
                entries: vec![DiskUsageEntry {
                    id: "pending".into(),
                    path: None,
                    used_bytes: None,
                    sampled_at_ms: 0,
                    owner: None,
                    process_id: None,
                }],
            });
        cached.requested = Instant::now();
        let due = cached.checked.is_none_or(|at| at.elapsed() >= INTERVAL);
        let entries = cached.entries.clone();
        if due && !state.busy {
            state.busy = true;
            let store = self.clone();
            // Serve the oldest pending sample first when many Rooms are watched.
            let room = state
                .rooms
                .iter()
                .min_by_key(|(_, sample)| sample.checked)
                .map(|(room, _)| room.clone())
                .expect("the requested Room is present");
            tokio::task::spawn_blocking(move || {
                // A panic still releases admission for the next sampling pass.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let (directories, processes) = collect(&room);
                    sample(directories, &processes)
                }));
                let mut state = store.0.lock().unwrap_or_else(|e| e.into_inner());
                state.busy = false;
                if let Some(sample) = state.rooms.get_mut(&room) {
                    sample.checked = Some(Instant::now());
                    match result {
                        Ok(entries) => sample.entries = entries,
                        Err(_) => tracing::warn!(
                            "disk occupancy sampling failed; the next sample will retry"
                        ),
                    }
                }
            });
        }
        entries
    }
}

pub(crate) fn sample(
    directories: Vec<Directory>,
    processes: &[SessionProcess],
) -> Vec<DiskUsageEntry> {
    let mut entries = Vec::new();
    let mut measured = BTreeMap::new();
    for directory in directories {
        let path = std::fs::canonicalize(&directory.path).unwrap_or(directory.path);
        let (id, used_bytes) = measured
            .entry(path.clone())
            .or_insert_with(|| {
                let id = path.display().to_string();
                let bytes = allocated_tree(&path, ENTRIES, WALK_TIME);
                (id, bytes)
            })
            .clone();
        entries.push(DiskUsageEntry {
            id,
            path: Some(path.display().to_string()),
            used_bytes,
            sampled_at_ms: crate::session::unix_epoch_ms(),
            owner: directory.owner,
            process_id: None,
        });
    }
    for process in processes {
        open_files(process, &mut entries);
    }
    entries
}

/// Never report a budget-limited lower bound as the total. Links are not
/// followed, and hard links to the same allocation count once within a root.
fn allocated_tree(path: &Path, mut budget: usize, time: Duration) -> Option<u64> {
    let deadline = Instant::now() + time;
    let mut pending = vec![path.to_path_buf()];
    let mut seen = BTreeSet::new();
    let mut bytes: u64 = 0;
    while let Some(path) = pending.pop() {
        if budget == 0 || Instant::now() >= deadline {
            return None;
        }
        budget -= 1;
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        let (id, allocated) = allocation(&meta, &path);
        if !seen.insert(id) {
            continue;
        }
        bytes = bytes.saturating_add(allocated);
        if meta.is_dir() {
            let children = std::fs::read_dir(&path).ok()?;
            for child in children {
                if pending.len() >= budget || Instant::now() >= deadline {
                    return None;
                }
                pending.push(child.ok()?.path());
            }
        }
    }
    Some(bytes)
}

#[cfg(unix)]
fn allocation(meta: &std::fs::Metadata, _path: &Path) -> (String, u64) {
    use std::os::unix::fs::MetadataExt;
    (
        format!("file:{}:{}", meta.dev(), meta.ino()),
        meta.blocks().saturating_mul(512),
    )
}
#[cfg(not(unix))]
fn allocation(meta: &std::fs::Metadata, path: &Path) -> (String, u64) {
    (path.display().to_string(), meta.len())
}

#[cfg(target_os = "linux")]
fn open_files(process: &SessionProcess, entries: &mut Vec<DiskUsageEntry>) {
    let pid = process.process.pid;
    let Ok((identity, _)) = super::kernel_access::process::inspect(pid) else {
        return;
    };
    let first = entries.len();
    let stamp = crate::session::unix_epoch_ms();
    entries.push(DiskUsageEntry {
        id: format!("{}:open-files", process.process.process_id),
        path: None,
        used_bytes: None,
        sampled_at_ms: stamp,
        owner: Some(process.owner.clone()),
        process_id: Some(process.process.process_id.clone()),
    });
    let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return;
    };
    let mut seen = BTreeSet::new();
    let mut complete = true;
    for (index, fd) in fds.enumerate() {
        if index >= 1024 {
            complete = false;
            break;
        }
        let Ok(fd) = fd else {
            complete = false;
            continue;
        };
        let Ok(meta) = std::fs::metadata(fd.path()) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let (id, used_bytes) = allocation(&meta, &fd.path());
        if !seen.insert(id.clone()) {
            continue;
        }
        let target = std::fs::read_link(fd.path()).ok();
        let path = target
            .filter(|path| !path.to_string_lossy().ends_with(" (deleted)"))
            .map(|path| path.display().to_string());
        entries.push(DiskUsageEntry {
            id,
            path,
            used_bytes: Some(used_bytes),
            sampled_at_ms: crate::session::unix_epoch_ms(),
            owner: Some(process.owner.clone()),
            process_id: Some(process.process.process_id.clone()),
        });
    }
    if identity.alive() && complete {
        entries[first].used_bytes = Some(0);
    } else {
        entries.truncate(first + 1);
    }
}
#[cfg(not(target_os = "linux"))]
fn open_files(_process: &SessionProcess, _entries: &mut Vec<DiskUsageEntry>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("disk-usage-{:032x}", rand::random::<u128>()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn occupancy_falls_after_deletion_and_never_claims_a_partial_walk_is_complete() {
        let root = TempDir::new();
        let path = root.path().join("output");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&vec![1; 1024 * 1024]).unwrap();
        file.sync_all().unwrap();
        let before = allocated_tree(root.path(), ENTRIES, Duration::from_secs(2)).unwrap();
        assert!(before >= 1024 * 1024);
        assert_eq!(allocated_tree(root.path(), 1, WALK_TIME), None);
        std::fs::remove_file(path).unwrap();
        let after = allocated_tree(root.path(), ENTRIES, Duration::from_secs(2)).unwrap();
        assert!(after < before);
    }

    #[cfg(unix)]
    #[test]
    fn directory_walk_does_not_follow_links_or_count_a_hard_link_twice() {
        let root = TempDir::new();
        let outside = TempDir::new();
        std::fs::write(outside.path().join("large"), vec![1; 1024 * 1024]).unwrap();
        std::fs::write(root.path().join("file"), vec![1; 8192]).unwrap();
        let before = allocated_tree(root.path(), ENTRIES, Duration::from_secs(2)).unwrap();
        std::fs::hard_link(root.path().join("file"), root.path().join("same")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        let after = allocated_tree(root.path(), ENTRIES, Duration::from_secs(2)).unwrap();
        assert!(
            after < before + 8192,
            "shared allocation or linked tree counted twice"
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn an_unlinked_open_file_keeps_its_allocation_and_process_owner() {
        let root = TempDir::new();
        let path = root.path().join("held");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&vec![1; 3 * 1024 * 1024]).unwrap();
        file.sync_all().unwrap();
        let (id, bytes) = allocation(&file.metadata().unwrap(), &path);
        let process = SessionProcess {
            remote_available: false,
            remote_refusal: None,
            estimate: None,
            placement: None,
            owner: SessionProcessOwner::Agent("agent-test".into()),
            state: super::super::process_resources::SessionProcessState::Running,
            process: super::super::process_resources::ProcessSample {
                process_id: "owned-process".into(),
                pid: std::process::id(),
                command: "test".into(),
                resident_set_bytes: 0,
                cpu_millis: 0,
                disk_io_bytes: None,
                disk_written_bytes: None,
                deleted_open_bytes: 0,
                outlived: false,
                sampled_at_ms: crate::session::unix_epoch_ms(),
                age_ms: 0,
            },
        };
        std::fs::remove_file(path).unwrap();
        let entries = sample(Vec::new(), std::slice::from_ref(&process));
        let held = entries.iter().find(|entry| entry.id == id).unwrap();
        assert_eq!(held.used_bytes, Some(bytes));
        assert_eq!(held.path, None);
        assert_eq!(held.owner.as_ref(), Some(&process.owner));
        assert_eq!(held.process_id.as_deref(), Some("owned-process"));
        drop(file);
        assert!(!sample(Vec::new(), &[process])
            .iter()
            .any(|entry| entry.id == id));
    }
}
