//! Resource domains for kernel-owned providers and terminals. The kernel never
//! joins a workload's domain; observations and termination share its ownership.
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

mod control;
use control::WatchCommand;
pub(crate) use control::{
    end_owner, outside_ended_for_key, start_now_for_key, stop_for_key, stop_released,
};
pub use control::{ProcessControlOutcome, ProcessControlResult, ProcessControlTarget};
mod admission;
mod pool;
mod projection;
mod sampling;
mod session_sweep;
#[cfg(test)]
pub(crate) use admission::WaitReason;
#[cfg(test)]
pub(crate) use projection::released_keys_for_session;
#[cfg(test)]
pub(crate) use projection::SessionProcessState;
pub(crate) use projection::{for_session, keys_for_session};
pub use projection::{SessionProcess, SessionProcessOwner};
pub(crate) use session_sweep::SessionSweep;
#[cfg(test)]
mod admission_tests;
#[cfg(target_os = "linux")]
mod cgroup;
#[cfg(test)]
mod pressure_tests;
#[cfg(test)]
mod tests;

pub use sampling::ProcessSample;
const MIB: u64 = 1024 * 1024;
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
    /// The owner-set hard cap; without one only the shared ceiling applies.
    cap: Option<u64>,
    pool: Arc<pool::Pool>,
    #[cfg(target_os = "linux")]
    cgroup: Option<Arc<cgroup::Domain>>,
}

impl PreparedDomain {
    /// `unit` names whose accounting unit the workload joins: everything one
    /// owner started shares one fair share and any owner-set cap, however
    /// often it restarted.
    pub(crate) fn new(unit: &str) -> std::io::Result<Self> {
        let cap = configured_cap(
            std::env::var("CHARIOX_PROCESS_MEMORY_LIMIT_MIB")
                .ok()
                .as_deref(),
        )?;
        Ok(Self::in_pool(unit, cap, pool::Pool::shared()))
    }

    fn in_pool(unit: &str, cap: Option<u64>, pool: Arc<pool::Pool>) -> Self {
        #[cfg(target_os = "linux")]
        let cgroup = pool
            .budget()
            .and_then(|budget| cgroup::Domain::create(&budget, unit, cap))
            .map(Arc::new);
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
        Self {
            unit: unit.to_string(),
            cap,
            pool,
            #[cfg(target_os = "linux")]
            cgroup,
        }
    }

    /// The portable resident-memory watchdog, whatever the host's cgroups allow.
    #[cfg(test)]
    fn watchdog(unit: &str, cap: Option<u64>, pool: Arc<pool::Pool>) -> Self {
        Self {
            unit: unit.to_string(),
            cap,
            pool,
            #[cfg(target_os = "linux")]
            cgroup: None,
        }
    }

    /// A domain without a cgroup, for tests that only need the launch shape:
    /// an unstarted cgroup domain ends its processes when dropped.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self::watchdog("test", None, pool::Pool::shared())
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
        let in_cgroup = self.cgroup.is_some();
        #[cfg(not(target_os = "linux"))]
        let in_cgroup = false;
        let state = Arc::new(Mutex::new(Observation {
            unit: self.unit.clone(),
            cap: self.cap,
            in_cgroup,
            holdable: true,
            ..Observation::default()
        }));
        self.pool.join(&state);
        let watched = state.clone();
        let (stop, stopped) = mpsc::channel();
        state.lock().unwrap_or_else(|e| e.into_inner()).control = Some(stop.clone());
        let mut released = false;
        // Per command: process-table time, resident and written bytes, and
        // their growth per second.
        let mut previous = std::collections::BTreeMap::<u32, Rates>::new();
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
                // Rates use the process table's own time: watches share a
                // cached table, so a tick can see the same table again.
                let sampled_at = samples.iter().map(|p| p.sampled_at_ms).max().unwrap_or(0);
                let commands = tree
                    .commands(&samples)
                    .into_iter()
                    .map(|(pid, (resident, pids))| {
                        let writes = pids.iter().map(|pid| admission::written(*pid)).sum();
                        let rate = |now: u64, before: u64, at: u64, rate: u64| {
                            (now.saturating_sub(before).saturating_mul(1000) / (sampled_at - at))
                                .max(rate / 2)
                        };
                        // A runaway slowed by the pressure brake is still the
                        // runaway: its rate decays by half per new sample.
                        let (growth, write_growth) = match previous.get(&pid) {
                            Some(before) if sampled_at > before.at => (
                                rate(resident, before.resident, before.at, before.growth),
                                rate(writes, before.writes, before.at, before.write_growth),
                            ),
                            Some(before) => (before.growth, before.write_growth),
                            None => (0, 0),
                        };
                        pool::CommandUse {
                            pid,
                            resident,
                            growth,
                            writes,
                            write_growth,
                            effective: tree.effective(pid, &samples),
                            root: tree.in_harness(pid),
                        }
                    })
                    .collect::<Vec<_>>();
                previous = commands
                    .iter()
                    .map(|command| {
                        let rates = previous
                            .get(&command.pid)
                            .filter(|before| before.at >= sampled_at)
                            .copied()
                            .unwrap_or(Rates {
                                at: sampled_at,
                                resident: command.resident,
                                growth: command.growth,
                                writes: command.writes,
                                write_growth: command.write_growth,
                            });
                        (command.pid, rates)
                    })
                    .collect();
                {
                    let mut observation = watched.lock().unwrap_or_else(|e| e.into_inner());
                    observation.samples = samples;
                    observation.commands = commands;
                }
                #[cfg(target_os = "linux")]
                let unit_charged = self.cgroup.as_ref().and_then(|group| group.unit_charged());
                #[cfg(not(target_os = "linux"))]
                let unit_charged = None;
                self.pool.enforce(&watched, unit_charged);
                admit(&tree, &watched);
                if released && ended {
                    break;
                }
                match stopped.recv_timeout(INTERVAL) {
                    Ok(WatchCommand::Release) => released = true,
                    Ok(WatchCommand::MemoryStop {
                        pid,
                        limit_bytes,
                        notice,
                    }) => {
                        if tree.is_root(pid) {
                            // The workload itself is the command: its owner
                            // reports the typed memory outcome.
                            watched
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .memory_stop = Some(MemoryStop {
                                limit_bytes,
                                notice,
                            });
                            #[cfg(target_os = "linux")]
                            if let Some(group) = &self.cgroup {
                                group.terminate();
                            }
                        } else {
                            announce(pid, &notice);
                        }
                        tree.kill(pid);
                    }
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
                    Ok(WatchCommand::StartNow { process_id, reply }) => {
                        let started = tree.command_of(&process_id).is_some_and(|command| {
                            watched
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .admission
                                .get_mut(&command)
                                .is_some_and(admission::Entry::force)
                        });
                        admit(&tree, &watched);
                        let _ = reply.send(if started {
                            Ok(vec![process_id])
                        } else {
                            Err("the process is not waiting; refresh Health".into())
                        });
                    }
                    Ok(WatchCommand::Apply) | Err(mpsc::RecvTimeoutError::Timeout) => {}
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
        })
    }
}

/// Process-table time, resident and written bytes of one command, and their
/// growth per second.
#[derive(Clone, Copy)]
struct Rates {
    at: u64,
    resident: u64,
    growth: u64,
    writes: u64,
    write_growth: u64,
}

/// Applies admission's decisions: a waiting command's subtree is stopped
/// (again each tick, so what it started meanwhile stops too); one that may run
/// continues.
fn admit(tree: &sampling::ProcessTree, watched: &Arc<Mutex<Observation>>) {
    let signals = watched
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .admission
        .iter_mut()
        .filter_map(|(command, entry)| {
            let waiting = entry.wait.is_some();
            let signal = (waiting || entry.stopped).then_some((*command, waiting));
            entry.stopped = waiting;
            signal
        })
        .collect::<Vec<_>>();
    #[cfg(unix)]
    for (command, waiting) in signals {
        tree.signal(
            command,
            if waiting {
                libc::SIGSTOP
            } else {
                libc::SIGCONT
            },
        );
    }
    #[cfg(not(unix))]
    let _ = (tree, signals);
}

fn configured_cap(value: Option<&str>) -> std::io::Result<Option<u64>> {
    value
        .map(|value| {
            value
                .parse::<u64>()
                .ok()
                .filter(|mib| *mib >= 16)
                .and_then(|mib| mib.checked_mul(MIB))
                .ok_or_else(|| {
                    std::io::Error::other(
                        "CHARIOX_PROCESS_MEMORY_LIMIT_MIB must be an integer of at least 16 MiB",
                    )
                })
        })
        .transpose()
}

/// Tells a stopped command's own error stream why, so the agent reading its
/// tool output sees the reason. Only a stream the command already writes to is
/// reopened, and only appended to.
fn announce(pid: u32, notice: &str) {
    #[cfg(target_os = "linux")]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let writable = std::fs::read_to_string(format!("/proc/{pid}/fdinfo/2")).is_ok_and(|info| {
            info.lines()
                .find_map(|line| line.strip_prefix("flags:"))
                .and_then(|flags| u32::from_str_radix(flags.trim(), 8).ok())
                .is_some_and(|flags| flags & libc::O_ACCMODE as u32 != libc::O_RDONLY as u32)
        });
        if writable {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .append(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
                .open(format!("/proc/{pid}/fd/2"))
            {
                let _ = file.write_all(notice.as_bytes());
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (pid, notice);
}

/// Why the workload's own process was stopped for memory.
struct MemoryStop {
    limit_bytes: u64,
    notice: String,
}

#[derive(Default)]
struct Observation {
    /// The accounting unit the workload belongs to (its agent, or itself).
    unit: String,
    cap: Option<u64>,
    in_cgroup: bool,
    /// The commands that start from now on may wait for room: not a person's
    /// own terminal command. Each keeps the policy it started with.
    holdable: bool,
    control: Option<mpsc::Sender<WatchCommand>>,
    outside_ended: bool,
    samples: Vec<ProcessSample>,
    commands: Vec<pool::CommandUse>,
    /// Admission per command, by the command's top process.
    admission: std::collections::BTreeMap<u32, admission::Entry>,
    memory_stop: Option<MemoryStop>,
    /// Who Health lists the processes under when no run or terminal does.
    listed: Option<projection::Listing>,
    /// Set once the workload's own process ended: what it left running is
    /// still this owner's, listed, counted and stoppable until it ends.
    released: bool,
}
impl Observation {
    fn resident(&self) -> u64 {
        self.samples.iter().map(|p| p.resident_set_bytes).sum()
    }
}

pub(crate) struct ProcessDomain {
    state: Arc<Mutex<Observation>>,
    stop: mpsc::Sender<WatchCommand>,
    worker: Option<JoinHandle<()>>,
    released: bool,
    started: u64,
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
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.listed = Some(projection::Listing {
                session_id: session_id.to_string(),
                owner,
                opened_by,
            });
            state.released = true;
        }
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
    /// Set once the workload itself was the command stopped for memory.
    pub(crate) fn memory_limit_bytes(&self) -> Option<u64> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.memory_stop.as_ref().map(|stop| stop.limit_bytes)
    }
    /// What an owner without a typed outcome tells its reader about that stop.
    pub(crate) fn memory_stop_notice(&self) -> Option<String> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.memory_stop.as_ref().map(|stop| stop.notice.clone())
    }
    /// Lists the processes in Health under `owner` (a process the agent
    /// started through a runtime tool rather than its provider run).
    pub(crate) fn list_as(&self, session_id: &str, owner: SessionProcessOwner) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).listed = Some(projection::Listing {
            session_id: session_id.to_string(),
            owner,
            opened_by: None,
        });
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

/// Whether the commands the workload bound to `key` starts from now on may
/// wait for room: a person's own terminal command never does. Commands
/// already running keep the policy they started with.
pub(crate) fn set_holdable_for_key(key: &str, holdable: bool) {
    let state = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .and_then(std::sync::Weak::upgrade);
    if let Some(state) = state {
        state.lock().unwrap_or_else(|e| e.into_inner()).holdable = holdable;
    }
}

/// What the commands the workload bound to `key` starts next would get.
#[cfg(test)]
pub(crate) fn holdable_for_key(key: &str) -> Option<bool> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .and_then(std::sync::Weak::upgrade)
        .map(|state| state.lock().unwrap_or_else(|e| e.into_inner()).holdable)
}

pub(crate) fn has_key(key: &str) -> bool {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .is_some_and(|state| state.strong_count() > 0)
}
