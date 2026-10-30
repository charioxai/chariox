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

/// The parent of the kernel's own cgroup, when it delegates the memory controller.
pub(super) fn delegated_parent() -> Option<PathBuf> {
    let parent = own()?.parent()?.to_path_buf();
    let controllers = std::fs::read_to_string(parent.join("cgroup.subtree_control")).ok()?;
    controllers
        .split_whitespace()
        .any(|name| name == "memory")
        .then_some(parent)
}

/// The cgroups whose limits bound the workloads: from the delegated parent
/// their budget lives under, else from the kernel's own cgroup they share.
pub(super) fn bounding() -> Vec<PathBuf> {
    delegated_parent()
        .or_else(own)
        .map(|start| {
            start
                .ancestors()
                .take_while(|path| path.starts_with("/sys/fs/cgroup"))
                .map(Path::to_path_buf)
                .collect()
        })
        .unwrap_or_default()
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
