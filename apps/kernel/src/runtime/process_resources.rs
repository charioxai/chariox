//! Resource domains for kernel-owned providers and terminals. The kernel never
//! joins a workload's domain; observations and termination share its ownership.
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

mod control;
use control::WatchCommand;
pub(crate) use control::{end_owner, outside_ended_for_key, stop_for_key, stop_released};
pub use control::{ProcessControlOutcome, ProcessControlResult, ProcessControlTarget};
mod projection;
mod sampling;
mod session_sweep;
#[cfg(test)]
pub(crate) use projection::released_keys_for_session;
pub(crate) use projection::{for_session, keys_for_session};
pub use projection::{SessionProcess, SessionProcessOwner};
pub(crate) use session_sweep::SessionSweep;
#[cfg(target_os = "linux")]
mod cgroup;
#[cfg(test)]
mod tests;

pub use sampling::ProcessSample;
const MIB: u64 = 1024 * 1024;
const DEFAULT_LIMIT: u64 = 16384 * MIB;
const INTERVAL: Duration = Duration::from_millis(250);

#[cfg(unix)]
pub(crate) fn validate_executable(
    program: &str,
    cwd: Option<&std::path::Path>,
    path: Option<&std::ffi::OsStr>,
) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    let cwd = cwd.unwrap_or_else(|| Path::new("."));
    let executable = |path: &Path| {
        std::fs::metadata(cwd.join(path))
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    let available = if program.contains('/') {
        executable(Path::new(program))
    } else {
        let path = path.unwrap_or_else(|| {
            std::ffi::OsStr::new("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        });
        std::env::split_paths(path).any(|directory| executable(&directory.join(program)))
    };
    if !available {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "provider executable is missing or not executable; install it and retry",
        ));
    }
    Ok(())
}

pub(crate) struct PreparedDomain {
    unit: String,
    limit: u64,
    #[cfg(target_os = "linux")]
    cgroup: Option<Arc<cgroup::Domain>>,
}

impl PreparedDomain {
    /// `unit` names whose accounting unit the workload joins: everything one
    /// owner started shares a single limit, however often it restarted.
    pub(crate) fn new(unit: &str) -> std::io::Result<Self> {
        let limit = configured_limit(
            std::env::var("CHARIOX_PROCESS_MEMORY_LIMIT_MIB")
                .ok()
                .as_deref(),
        )?;
        Self::with_limit(unit, limit)
    }

    fn with_limit(unit: &str, limit: u64) -> std::io::Result<Self> {
        if limit < 16 * MIB {
            return Err(std::io::Error::other(
                "process memory limit must be at least 16 MiB",
            ));
        }
        #[cfg(target_os = "linux")]
        let cgroup = cgroup::Domain::create(unit, limit).map(Arc::new);
        #[cfg(target_os = "linux")]
        if cgroup.is_none() {
            static NOTICE: std::sync::Once = std::sync::Once::new();
            NOTICE.call_once(|| {
                crate::logging::warn_with_fields(
                    "process_resources.watchdog",
                    "writable cgroup memory delegation unavailable; using resident-memory watchdog",
                    serde_json::json!({}),
                )
            });
        }
        Ok(Self {
            unit: unit.to_string(),
            limit,
            #[cfg(target_os = "linux")]
            cgroup,
        })
    }

    /// The portable resident-memory watchdog, whatever the host's cgroups allow.
    #[cfg(test)]
    fn watchdog(unit: &str, limit: u64) -> Self {
        Self {
            unit: unit.to_string(),
            limit,
            #[cfg(target_os = "linux")]
            cgroup: None,
        }
    }

    /// Join before executing the workload, so even its first fork inherits the
    /// domain. Positional arguments preserve the original argv without quoting.
    pub(crate) fn command(&self, program: String, args: Vec<String>) -> (String, Vec<String>) {
        #[cfg(target_os = "linux")]
        {
            // Kernel services may be OOM-protected. Workloads must be killable
            // by their own controller; inheriting -1000 makes memory.max loop.
            let admission = if self.cgroup.is_some() {
                "printf '%s\\n' \"$$\" > \"$1/cgroup.procs\" || exit 125; shift; exec \"$@\""
            } else {
                "shift; exec \"$@\""
            };
            let script = format!(
                "{} {admission}",
                crate::process_spawn::KILLABLE_SHELL_PREFIX
            );
            let mut wrapped = vec![
                "-c".into(),
                script,
                "chariox-resource-domain".into(),
                self.cgroup
                    .as_ref()
                    .map(|group| group.path().to_string_lossy().into_owned())
                    .unwrap_or_default(),
                program,
            ];
            wrapped.extend(args);
            ("/bin/sh".into(), wrapped)
        }
        #[cfg(not(target_os = "linux"))]
        (program, args)
    }

    pub(crate) fn start(self, pid: u32) -> std::io::Result<ProcessDomain> {
        if pid <= 1 || pid > i32::MAX as u32 {
            return Err(std::io::Error::other(
                "resource domain requires an owned child PID",
            ));
        }
        sampling::ensure_available()?;
        let mut tree = sampling::ProcessTree::new(pid);
        #[cfg(target_os = "linux")]
        if let Some(group) = &self.cgroup {
            tree.set_cgroup(group.clone());
        }
        #[cfg(target_os = "linux")]
        let cgroup = self.cgroup.clone();
        let state = Arc::new(Mutex::new(Observation {
            unit: self.unit.clone(),
            ..Observation::default()
        }));
        let watched = state.clone();
        let (stop, stopped) = mpsc::channel();
        state.lock().unwrap_or_else(|e| e.into_inner()).control = Some(stop.clone());
        let mut released = false;
        let worker = std::thread::Builder::new()
            .name("chariox-process-resources".into())
            .spawn(move || loop {
                let samples = match tree.sample() {
                    Ok(samples) => samples,
                    Err(error) => {
                        crate::logging::error_with_fields(
                            "process_resources.monitor",
                            "process resource sampling failed; stopping its owned tree",
                            serde_json::json!({"pid": pid, "error": error}),
                        );
                        tree.terminate();
                        break;
                    }
                };
                let resident = samples.iter().map(|p| p.resident_set_bytes).sum::<u64>();
                // A job that left soon after its parent may not be observed yet;
                // its cgroup still holds it, and ending the watch would kill it.
                #[cfg(target_os = "linux")]
                let ended = samples.is_empty()
                    && self
                        .cgroup
                        .as_ref()
                        .is_none_or(|group| group.pids().is_ok_and(|pids| pids.is_empty()));
                #[cfg(not(target_os = "linux"))]
                let ended = samples.is_empty();
                #[cfg(target_os = "linux")]
                let oom = self.cgroup.as_ref().and_then(|group| group.failure_limit());
                #[cfg(not(target_os = "linux"))]
                let oom: Option<u64> = None;
                #[cfg(target_os = "linux")]
                let use_watchdog = self.cgroup.is_none();
                #[cfg(not(target_os = "linux"))]
                let use_watchdog = true;
                let reserve_limit = reserve_limit_for(&watched, resident, use_watchdog);
                // Everything the owner started shares the limit, however often it restarted.
                let unit_resident = if use_watchdog {
                    unit_resident_for(&watched, resident)
                } else {
                    resident
                };
                let exceeded = oom.is_some()
                    || (use_watchdog && unit_resident > self.limit)
                    || reserve_limit.is_some();
                {
                    let mut observation = watched.lock().unwrap_or_else(|e| e.into_inner());
                    observation.samples = samples;
                    if exceeded {
                        observation.memory_limit_bytes =
                            Some(oom.or(reserve_limit).unwrap_or(self.limit));
                    }
                }
                if exceeded {
                    #[cfg(target_os = "linux")]
                    if let Some(group) = &self.cgroup {
                        group.terminate();
                    }
                    tree.terminate();
                }
                if released && ended {
                    break;
                }
                match stopped.recv_timeout(INTERVAL) {
                    Ok(WatchCommand::Release) => released = true,
                    Ok(WatchCommand::Stop { process_id, reply }) => {
                        let result = tree.stop(
                            process_id.as_deref(),
                            || {
                                watched
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .outside_ended = true;
                            },
                            || {
                                #[cfg(target_os = "linux")]
                                if let Some(group) = &self.cgroup {
                                    group.terminate();
                                }
                            },
                        );
                        let _ = reply.send(result);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => {
                        tree.terminate();
                        break;
                    }
                }
            })?;
        Ok(ProcessDomain {
            state,
            stop,
            worker: Some(worker),
            released: false,
            started: control::domain_started(),
            #[cfg(target_os = "linux")]
            cgroup,
        })
    }
}

fn configured_limit(value: Option<&str>) -> std::io::Result<u64> {
    match value {
        None => Ok(DEFAULT_LIMIT),
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|mib| *mib >= 16)
            .and_then(|mib| mib.checked_mul(MIB))
            .ok_or_else(|| {
                std::io::Error::other(
                    "CHARIOX_PROCESS_MEMORY_LIMIT_MIB must be an integer of at least 16 MiB",
                )
            }),
    }
}

#[derive(Default)]
struct Observation {
    /// The accounting unit the workload belongs to (its agent, or itself).
    unit: String,
    control: Option<mpsc::Sender<WatchCommand>>,
    outside_ended: bool,
    samples: Vec<ProcessSample>,
    memory_limit_bytes: Option<u64>,
    /// Set once the workload's own process ended: what it left running is
    /// still this owner's, listed, counted and stoppable until it ends.
    released_to: Option<projection::Released>,
}

pub(crate) struct ProcessDomain {
    state: Arc<Mutex<Observation>>,
    stop: mpsc::Sender<WatchCommand>,
    worker: Option<JoinHandle<()>>,
    released: bool,
    started: u64,
    #[cfg(target_os = "linux")]
    cgroup: Option<Arc<cgroup::Domain>>,
}
impl ProcessDomain {
    /// The workload's own process ended and its owner swept what it still
    /// owns; `opened_by` is the agent that opened a terminal owner. Processes that left on purpose (`setsid nohup job &`) are not
    /// ended with it: the memory watch stays until the last of them exits.
    /// An owner already gone for good gets nothing: they end with the domain.
    pub(crate) fn release(
        &mut self,
        session_id: &str,
        owner: SessionProcessOwner,
        opened_by: Option<String>,
    ) {
        let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
        if control::owner_ended(self.started, session_id, &owner, opened_by.as_deref()) {
            return;
        }
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .released_to = Some(projection::Released {
            session_id: session_id.to_string(),
            owner,
            opened_by,
        });
        // A new workload of the same owner may bind the same key; the released
        // processes keep their own entry until the last of them ends.
        registry.retain(|_, state| !state.ptr_eq(&Arc::downgrade(&self.state)));
        registry.insert(
            format!("released-{:032x}", rand::random::<u128>()),
            Arc::downgrade(&self.state),
        );
        drop(registry);
        self.released = self.stop.send(WatchCommand::Release).is_ok();
    }
    pub(crate) fn memory_limit_bytes(&self) -> Option<u64> {
        #[cfg(target_os = "linux")]
        if let Some(limit) = self.cgroup.as_ref().and_then(|group| group.failure_limit()) {
            return Some(limit);
        }
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .memory_limit_bytes
    }
    pub(crate) fn bind_key(&self, key: &str) {
        let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
        registry.retain(|_, state| state.strong_count() > 0);
        registry.insert(key.to_string(), Arc::downgrade(&self.state));
    }
    pub(crate) fn unbind_key(&self, key: &str) {
        let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
        if registry
            .get(key)
            .is_some_and(|state| state.ptr_eq(&Arc::downgrade(&self.state)))
        {
            registry.remove(key);
        }
    }
}
impl Drop for ProcessDomain {
    fn drop(&mut self) {
        control::domain_dropped(self.started);
        if self.released {
            // The watch thread owns the domain from here and ends with its last process.
            self.worker.take();
        } else {
            let _ = self.stop.send(WatchCommand::Shutdown);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if self.released {
            return;
        }
        registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, state| !state.ptr_eq(&Arc::downgrade(&self.state)));
    }
}

fn registry(
) -> &'static Mutex<std::collections::BTreeMap<String, std::sync::Weak<Mutex<Observation>>>> {
    static REGISTRY: std::sync::OnceLock<
        Mutex<std::collections::BTreeMap<String, std::sync::Weak<Mutex<Observation>>>>,
    > = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}
pub(crate) fn samples_for_key(key: &str) -> Vec<ProcessSample> {
    let state = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .and_then(std::sync::Weak::upgrade);
    state
        .map(|state| {
            state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .samples
                .clone()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|sample| {
            crate::session::unix_epoch_ms().saturating_sub(sample.sampled_at_ms) <= 5000
        })
        .collect()
}

/// Resident memory of every workload in `current`'s accounting unit.
fn unit_resident_for(current: &Arc<Mutex<Observation>>, resident: u64) -> u64 {
    let unit = current
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unit
        .clone();
    let entries = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter_map(std::sync::Weak::upgrade)
        .collect::<Vec<_>>();
    let mut seen = std::collections::BTreeSet::new();
    let mut total = resident;
    for state in entries {
        if Arc::ptr_eq(current, &state) || !seen.insert(Arc::as_ptr(&state) as usize) {
            continue;
        }
        let state = state.lock().unwrap_or_else(|e| e.into_inner());
        if state.unit == unit {
            total = total.saturating_add(
                state
                    .samples
                    .iter()
                    .map(|p| p.resident_set_bytes)
                    .sum::<u64>(),
            );
        }
    }
    total
}

fn reserve_limit_for(
    current: &Arc<Mutex<Observation>>,
    resident: u64,
    enforce_aggregate: bool,
) -> Option<u64> {
    if resident == 0 {
        return None;
    }
    let (budget, reserve, available) = memory_budget()?;
    let entries = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter_map(std::sync::Weak::upgrade)
        .collect::<Vec<_>>();
    let mut seen = std::collections::BTreeSet::new();
    let mut total = resident;
    let mut largest = resident;
    for state in entries {
        if Arc::ptr_eq(current, &state) || !seen.insert(Arc::as_ptr(&state) as usize) {
            continue;
        }
        let bytes = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .samples
            .iter()
            .map(|p| p.resident_set_bytes)
            .sum::<u64>();
        total = total.saturating_add(bytes);
        largest = largest.max(bytes);
    }
    (resident >= largest
        && ((enforce_aggregate && total > budget)
            || available.is_some_and(|bytes| bytes < reserve)))
    .then_some(budget)
}

fn memory_budget() -> Option<(u64, u64, Option<u64>)> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kib = |key: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(key))?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()?
                .checked_mul(1024)
        };
        let mut total = kib("MemTotal:")?;
        let mut available = kib("MemAvailable:");
        if let Some(relative) = std::fs::read_to_string("/proc/self/cgroup")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("0::").map(str::to_string))
            })
        {
            let own = std::path::Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/'));
            for ancestor in own
                .ancestors()
                .take_while(|path| path.starts_with("/sys/fs/cgroup"))
            {
                if let Some(limit) = std::fs::read_to_string(ancestor.join("memory.max"))
                    .ok()
                    .and_then(|value| value.trim().parse::<u64>().ok())
                {
                    total = total.min(limit);
                    if let Some(used) = std::fs::read_to_string(ancestor.join("memory.current"))
                        .ok()
                        .and_then(|value| value.trim().parse::<u64>().ok())
                    {
                        // Inactive file cache is reclaimable by this cgroup;
                        // charging it as unavailable would kill healthy trees
                        // after a build even with ample working-set headroom.
                        let reclaimable = std::fs::read_to_string(ancestor.join("memory.stat"))
                            .ok()
                            .and_then(|text| {
                                text.lines().find_map(|line| {
                                    line.strip_prefix("inactive_file ")
                                        .and_then(|value| value.parse::<u64>().ok())
                                })
                            })
                            .unwrap_or(0);
                        available = Some(
                            available
                                .unwrap_or(u64::MAX)
                                .min(limit.saturating_sub(used.saturating_sub(reclaimable))),
                        );
                    }
                }
            }
        }
        let reserve = (total / 10).max(512 * MIB);
        Some((total.checked_sub(reserve)?, reserve, available))
    }
    #[cfg(target_os = "macos")]
    {
        static MEMORY: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
        let total = (*MEMORY.get_or_init(|| {
            let output = crate::process_spawn::command("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .ok()?;
            let total = String::from_utf8(output.stdout)
                .ok()?
                .trim()
                .parse::<u64>()
                .ok()?;
            Some(total)
        }))?;
        let reserve = (total / 10).max(512 * MIB);
        Some((total.checked_sub(reserve)?, reserve, None))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

pub(crate) fn has_key(key: &str) -> bool {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .is_some_and(|state| state.strong_count() > 0)
}
