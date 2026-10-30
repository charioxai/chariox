//! Linux memory controller domains. Only newly created directories are changed.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

struct Budget {
    path: PathBuf,
    limit: std::sync::atomic::AtomicU64,
}
impl Drop for Budget {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}
/// Everything one owner started, however many times its provider restarted, is
/// one accounting unit: one cgroup that carries the limit, with a leaf per
/// workload under it. A leaf only tells the workloads apart; its memory is
/// charged to the unit.
struct Unit {
    path: PathBuf,
    _budget: Arc<Budget>,
    limit: u64,
}
impl Drop for Unit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}
/// How often the unit's limit events had fired when a leaf joined it, so a
/// breach that ended earlier workloads does not end a later one.
#[derive(Clone, Copy, Default)]
struct Events {
    high: u64,
    oom: u64,
    oom_kill: u64,
}
impl Unit {
    fn events(&self) -> Option<Events> {
        let events = std::fs::read_to_string(self.path.join("memory.events")).ok()?;
        let count = |key: &str| {
            events
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0)
        };
        Some(Events {
            high: count("high "),
            oom: count("oom "),
            oom_kill: count("oom_kill "),
        })
    }
}
fn unit(key: &str, limit: u64) -> Option<Arc<Unit>> {
    static UNITS: OnceLock<Mutex<BTreeMap<String, Weak<Unit>>>> = OnceLock::new();
    let mut units = UNITS.get_or_init(Mutex::default).lock().ok()?;
    units.retain(|_, unit| unit.strong_count() > 0);
    if let Some(unit) = units.get(key).and_then(Weak::upgrade) {
        return Some(unit);
    }
    let budget = budget()?;
    let path = budget.path.join(format!("unit-{}", rand::random::<u128>()));
    std::fs::create_dir(&path).ok()?;
    let unit = Arc::new(Unit {
        path,
        _budget: budget,
        limit,
    });
    (|| -> std::io::Result<()> {
        // Throttle at the user limit so the controller can record the typed
        // outcome and end the tree. The killable hard ceiling is a backstop
        // for bursts between observations, not the normal termination path.
        let hard_limit = limit
            .checked_add((limit / 4).max(64 * 1024 * 1024))
            .ok_or_else(|| std::io::Error::other("process memory limit overflow"))?;
        std::fs::write(unit.path.join("memory.high"), limit.to_string())?;
        std::fs::write(unit.path.join("memory.max"), hard_limit.to_string())?;
        std::fs::write(unit.path.join("memory.swap.max"), "0")?;
        std::fs::write(unit.path.join("memory.oom.group"), "1")?;
        Ok(())
    })()
    .ok()?;
    units.insert(key.to_string(), Arc::downgrade(&unit));
    Some(unit)
}
pub(super) struct Domain {
    path: PathBuf,
    unit: Arc<Unit>,
    baseline: Events,
}
impl Domain {
    pub fn create(unit_key: &str, limit: u64) -> Option<Self> {
        let unit = unit(unit_key, limit)?;
        let path = unit
            .path
            .join(format!("process-{}", rand::random::<u128>()));
        std::fs::create_dir(&path).ok()?;
        let baseline = unit.events().unwrap_or_default();
        Some(Self {
            path,
            unit,
            baseline,
        })
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
    pub fn failure_limit(&self) -> Option<u64> {
        let events = self.unit.events()?;
        let limit = self.unit.limit;
        let charged = std::fs::read_to_string(self.unit.path.join("memory.current"))
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok());
        if events.high > self.baseline.high || charged.is_some_and(|bytes| bytes > limit) {
            return Some(limit);
        }
        (events.oom_kill > self.baseline.oom_kill).then(|| {
            if events.oom > self.baseline.oom {
                limit
            } else {
                self.unit
                    ._budget
                    .limit
                    .load(std::sync::atomic::Ordering::Relaxed)
            }
        })
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

fn budget() -> Option<Arc<Budget>> {
    static BUDGET: OnceLock<Mutex<Weak<Budget>>> = OnceLock::new();
    let mut slot = BUDGET.get_or_init(|| Mutex::new(Weak::new())).lock().ok()?;
    if let Some(budget) = slot.upgrade() {
        return Some(budget);
    }
    let current = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let relative = current.lines().find_map(|line| line.strip_prefix("0::"))?;
    let own = Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/'));
    // A systemd scope has internal processes: use a sibling under its already
    // delegated parent. Do not change controllers on any existing directory.
    let parent = own.parent()?;
    let controllers = std::fs::read_to_string(parent.join("cgroup.subtree_control")).ok()?;
    if !controllers.split_whitespace().any(|name| name == "memory") {
        return None;
    }
    let path = parent.join(format!(
        "chariox-{}-{}",
        std::process::id(),
        rand::random::<u128>()
    ));
    std::fs::create_dir(&path).ok()?;
    let budget = Arc::new(Budget {
        path,
        limit: std::sync::atomic::AtomicU64::new(0),
    });
    let configured = (|| -> std::io::Result<()> {
        let memory = std::fs::read_to_string("/proc/meminfo")?;
        let physical = memory
            .lines()
            .find_map(|line| line.strip_prefix("MemTotal:"))
            .and_then(|line| line.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| std::io::Error::other("physical memory unavailable"))?
            * 1024;
        let mut inherited = physical;
        for ancestor in parent.ancestors().take_while(|path| {
            path.starts_with("/sys/fs/cgroup") && *path != Path::new("/sys/fs/cgroup")
        }) {
            let value = std::fs::read_to_string(ancestor.join("memory.max"))?;
            if value.trim() != "max" {
                inherited =
                    inherited.min(value.trim().parse::<u64>().map_err(std::io::Error::other)?);
            }
        }
        let reserve = (inherited / 10).max(512 * 1024 * 1024);
        let available = inherited
            .checked_sub(reserve)
            .ok_or_else(|| std::io::Error::other("insufficient memory reserve"))?;
        budget
            .limit
            .store(available, std::sync::atomic::Ordering::Relaxed);
        std::fs::write(budget.path.join("memory.max"), available.to_string())?;
        std::fs::write(budget.path.join("memory.swap.max"), "0")?;
        std::fs::write(budget.path.join("cgroup.subtree_control"), "+memory")?;
        Ok(())
    })();
    configured.ok()?;
    *slot = Arc::downgrade(&budget);
    Some(budget)
}
