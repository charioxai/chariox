//! Linux memory controller domains. Only newly created directories are changed.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

/// Every workload of a pool, under the shared ceiling.
pub(super) struct Budget {
    path: PathBuf,
    ceiling: u64,
    units: Mutex<BTreeMap<String, Weak<Unit>>>,
}
impl Drop for Budget {
    fn drop(&mut self) {
        remove_empty(&self.path);
    }
}
/// Everything one owner started, however many times its provider restarted, is
/// one accounting unit: one cgroup that carries the owner's share and any
/// owner-set cap, with a leaf per workload under it. A leaf only tells the
/// workloads apart; its memory is charged to the unit.
struct Unit {
    path: PathBuf,
    budget: Arc<Budget>,
}
impl Drop for Unit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
        self.budget.share();
    }
}

impl Budget {
    pub fn create(ceiling: u64, margin: u64) -> Option<Arc<Self>> {
        // A systemd scope has internal processes: use a sibling under its already
        // delegated parent. Do not change controllers on any existing directory.
        let parent = delegated_parent()?;
        remove_leftovers(&parent);
        let path = parent.join(format!(
            "chariox-{}-{}",
            std::process::id(),
            rand::random::<u128>()
        ));
        std::fs::create_dir(&path).ok()?;
        let budget = Arc::new(Self {
            path,
            ceiling,
            units: Mutex::default(),
        });
        (|| -> std::io::Result<()> {
            // The ceiling is a backstop; the controller stops one command
            // before it. Above the pressure line every allocation is slowed.
            std::fs::write(budget.path.join("memory.max"), ceiling.to_string())?;
            let brake = ceiling.saturating_sub(margin / 2);
            std::fs::write(budget.path.join("memory.high"), brake.to_string())?;
            std::fs::write(budget.path.join("memory.swap.max"), "0")?;
            std::fs::write(budget.path.join("cgroup.subtree_control"), "+memory")?;
            Ok(())
        })()
        .ok()?;
        // The kernel and the user's own apps keep the larger CPU weight.
        if std::fs::read_to_string(parent.join("cgroup.subtree_control"))
            .is_ok_and(|controllers| controllers.split_whitespace().any(|name| name == "cpu"))
        {
            let _ = std::fs::write(budget.path.join("cpu.weight"), "50");
        }
        Some(budget)
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Memory the workloads hold that cannot be reclaimed without swap.
    pub fn used(&self) -> u64 {
        unreclaimable(&self.path).unwrap_or(0)
    }

    /// Workloads stall on memory a tenth of the time or more.
    pub fn stalled(&self) -> bool {
        std::fs::read_to_string(self.path.join("memory.pressure")).is_ok_and(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("full avg10="))
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse::<f64>().ok())
                .is_some_and(|share| share >= 10.0)
        })
    }

    fn unit(self: &Arc<Self>, key: &str, cap: Option<u64>) -> Option<Arc<Unit>> {
        let mut units = self.units.lock().ok()?;
        units.retain(|_, unit| unit.strong_count() > 0);
        if let Some(unit) = units.get(key).and_then(Weak::upgrade) {
            return Some(unit);
        }
        let path = self.path.join(format!("unit-{}", rand::random::<u128>()));
        std::fs::create_dir(&path).ok()?;
        let unit = Arc::new(Unit {
            path,
            budget: self.clone(),
        });
        if let Some(cap) = cap {
            // Throttle at the owner's cap so the controller stops one command
            // there; the hard limit only catches bursts between observations.
            let hard_limit = cap.checked_add((cap / 4).max(64 * 1024 * 1024))?;
            (|| -> std::io::Result<()> {
                std::fs::write(unit.path.join("memory.high"), cap.to_string())?;
                std::fs::write(unit.path.join("memory.max"), hard_limit.to_string())
            })()
            .ok()?;
        }
        units.insert(key.to_string(), Arc::downgrade(&unit));
        drop(units);
        self.share();
        Some(unit)
    }

    /// Under pressure, reclaim takes from owners above their fair share first.
    fn share(&self) {
        // A unit dropping its last reference calls back here: never hold the lock then.
        let live = match self.units.lock() {
            Ok(units) => units.values().filter_map(Weak::upgrade).collect::<Vec<_>>(),
            Err(_) => return,
        };
        let share = self.ceiling / live.len().max(1) as u64;
        for unit in live {
            let _ = std::fs::write(unit.path.join("memory.low"), share.to_string());
        }
    }
}

pub(super) struct Domain {
    path: PathBuf,
    unit: Arc<Unit>,
}
impl Domain {
    pub fn create(budget: &Arc<Budget>, unit_key: &str, cap: Option<u64>) -> Option<Self> {
        let unit = budget.unit(unit_key, cap)?;
        let path = unit
            .path
            .join(format!("process-{}", rand::random::<u128>()));
        std::fs::create_dir(&path).ok()?;
        Some(Self { path, unit })
    }
    /// Moves what runs in `previous`, the domain a child of the previous kernel
    /// program was started in (hot reload), into this one and removes the
    /// directories that program left. The program keeps its PID across a hot
    /// reload, so only a domain of this PID's own budget is taken.
    pub fn take_over(&self, previous: &Path, pid: u32) -> bool {
        let Some(unit) = previous.parent() else {
            return false;
        };
        let Some(budget) = unit.parent() else {
            return false;
        };
        let named = |path: &Path, prefix: &str| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(prefix))
        };
        if !(previous.starts_with("/sys/fs/cgroup")
            && named(previous, "process-")
            && named(unit, "unit-")
            && named(budget, &format!("chariox-{}-", std::process::id())))
        {
            return false;
        }
        let Ok(members) = std::fs::read_to_string(previous.join("cgroup.procs")) else {
            return false;
        };
        for member in members.lines() {
            let _ = std::fs::write(self.path.join("cgroup.procs"), member);
        }
        let moved = self.owns(pid);
        if moved {
            for directory in [previous, unit, budget] {
                let _ = std::fs::remove_dir(directory);
            }
        }
        moved
    }
    pub fn pids(&self) -> std::io::Result<Vec<u32>> {
        Ok(std::fs::read_to_string(self.path.join("cgroup.procs"))?
            .lines()
            .filter_map(|line| line.parse().ok())
            .collect())
    }
    pub fn owns(&self, pid: u32) -> bool {
        let expected = self
            .path
            .strip_prefix("/sys/fs/cgroup")
            .ok()
            .map(|path| Path::new("/").join(path));
        std::fs::read_to_string(format!("/proc/{pid}/cgroup")).is_ok_and(|text| {
            text.lines().any(|line| {
                line.strip_prefix("0::")
                    .is_some_and(|path| Some(Path::new(path)) == expected.as_deref())
            })
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// What the owner's unit holds, for its owner-set cap.
    pub fn unit_charged(&self) -> Option<u64> {
        unreclaimable(&self.unit.path)
    }

    pub fn terminate(&self) {
        let _ = std::fs::write(self.path.join("cgroup.kill"), "1");
    }
}
impl Drop for Domain {
    fn drop(&mut self) {
        self.terminate();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while std::fs::read_to_string(self.path.join("cgroup.events"))
            .is_ok_and(|text| text.lines().any(|line| line == "populated 1"))
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if let Err(error) = std::fs::remove_dir(&self.path) {
            crate::logging::warn_with_fields(
                "process_resources.cleanup",
                "could not remove empty process cgroup",
                serde_json::json!({"path": self.path, "error": error.to_string()}),
            );
        }
    }
}

fn own() -> Option<PathBuf> {
    let current = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let relative = current.lines().find_map(|line| line.strip_prefix("0::"))?;
    Some(Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/')))
}

/// The parent of the kernel's own cgroup, when it delegates the memory
/// controller and this user can create the budget's directory there.
pub(super) fn delegated_parent() -> Option<PathBuf> {
    static PARENT: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    PARENT
        .get_or_init(|| delegated_parent_of(&own()?, can_create_budget))
        .clone()
}

fn delegated_parent_of(own: &Path, can_create: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let parent = own.parent()?;
    let controllers = std::fs::read_to_string(parent.join("cgroup.subtree_control")).ok()?;
    (controllers.split_whitespace().any(|name| name == "memory") && can_create(parent))
        .then(|| parent.to_path_buf())
}

/// A delegated controller does not make the parent writable for this user.
fn can_create_budget(parent: &Path) -> bool {
    let probe = parent.join(format!("chariox-probe-{}", std::process::id()));
    let created = std::fs::create_dir(&probe).is_ok();
    let _ = std::fs::remove_dir(&probe);
    created
}

/// The cgroups whose limits bound the workloads, where they actually run:
/// from the delegated parent their budget lives under, else from the kernel's
/// own cgroup, which they then share.
pub(super) fn bounding() -> Vec<PathBuf> {
    let Some(own) = own() else {
        return Vec::new();
    };
    let parent = delegated_parent();
    bounding_for(Path::new("/sys/fs/cgroup"), &own, |path| {
        parent.as_deref() == Some(path)
    })
}

fn bounding_for(root: &Path, own: &Path, can_create: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    delegated_parent_of(own, can_create)
        .unwrap_or_else(|| own.to_path_buf())
        .ancestors()
        .take_while(|path| path.starts_with(root))
        .map(Path::to_path_buf)
        .collect()
}

/// How much more a limited cgroup can hold before its limit, without swap.
pub(super) fn headroom(path: &Path) -> Option<u64> {
    let limit = std::fs::read_to_string(path.join("memory.max"))
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(limit.saturating_sub(unreclaimable(path)?))
}

/// Charged memory minus the page cache the cgroup can drop (tmpfs stays).
fn unreclaimable(path: &Path) -> Option<u64> {
    let current = std::fs::read_to_string(path.join("memory.current"))
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    let stat = std::fs::read_to_string(path.join("memory.stat")).ok()?;
    let value = |key: &str| {
        stat.lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };
    Some(current.saturating_sub(value("file ").saturating_sub(value("shmem "))))
}

/// Budgets of kernels that ended without removing them (a crash, a kill):
/// only those whose process is gone, and only directories already empty.
fn remove_leftovers(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|name| name.strip_prefix("chariox-"))
            .and_then(|rest| rest.split_once('-'))
            .and_then(|(pid, _)| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid != std::process::id() && !Path::new(&format!("/proc/{pid}")).exists() {
            remove_empty(&entry.path());
        }
    }
}

/// Removes a cgroup and its descendants bottom-up; a populated one stays.
fn remove_empty(path: &Path) {
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                remove_empty(&entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// root/service.slice/kernel.service, the slice delegating memory and the
    /// kernel's own cgroup limited to 1 GiB.
    fn service() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("pc-cgroup-{:016x}", rand::random::<u64>()));
        let own = root.join("service.slice/kernel.service");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(
            own.parent().unwrap().join("cgroup.subtree_control"),
            "memory\n",
        )
        .unwrap();
        std::fs::write(own.join("memory.max"), "1073741824\n").unwrap();
        (root, own)
    }

    #[test]
    fn an_unwritable_delegated_parent_leaves_the_kernels_own_limit_in_force() {
        let (root, own) = service();
        let bounds = bounding_for(&root, &own, |_| false);
        assert_eq!(bounds.first(), Some(&own), "{bounds:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_writable_delegated_parent_bounds_the_sibling_budget_by_the_parent_only() {
        let (root, own) = service();
        let bounds = bounding_for(&root, &own, |_| true);
        assert_eq!(bounds.first(), own.parent().map(Path::to_path_buf).as_ref());
        assert!(!bounds.contains(&own), "{bounds:?}");
        let _ = std::fs::remove_dir_all(root);
    }
}
