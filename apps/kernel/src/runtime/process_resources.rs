//! Resource domains for kernel-owned providers and terminals. The kernel never
//! joins a workload's domain; observations and termination share its ownership.
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

mod control;
mod disk_use;
use control::WatchCommand;
pub(crate) use control::{end_owner, outside_ended_for_key, start_now_for_key, stop_for_key};
#[cfg(test)]
pub(crate) use control::{hold_registry_for_test, pause_sampling_for_key, stop_released};
pub use control::{ProcessControlOutcome, ProcessControlResult, ProcessControlTarget};
pub(crate) use handover::{adopt_released, hand_over_released, ReleasedJobs};

/// The commands whose admission has stopped them, for a kernel that replaces
/// its program in place.
pub(crate) fn hand_over_admission() -> Vec<HeldCommand> {
    pool::Pool::shared().held_commands()
}
mod admission;

pub(crate) fn command_line(argv: &[String]) -> String {
    admission::display(argv)
}

mod launch;
pub use launch::LaunchWait;
pub(crate) use launch::{LaunchAdmission, LaunchPermit};
mod handover;
mod pool;
mod projection;
mod sampling;
mod session_sweep;
mod submissions;
mod terminal_groups;
mod waits;
pub(crate) use admission::WaitReason;
pub(crate) use admission::{adopt_admission, volume_floor, HeldCommand};
#[cfg(test)]
pub(crate) use projection::released_keys_for_session;
pub(crate) use projection::SessionProcessState;
pub(crate) use projection::Waiting;
pub(crate) use projection::{for_session, keys_for_session};
pub use projection::{SessionProcess, SessionProcessOwner};
pub(crate) use session_sweep::SessionSweep;
pub(crate) use submissions::Submissions;
#[cfg(test)]
pub(crate) use waits::push_for_tests as push_wait_event;
pub(crate) use waits::{take_wait_events, WaitChange, WaitEvent};
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
/// A kernel short of descriptors for long says so once per this interval.
const SHORT_LOG_INTERVAL: Duration = Duration::from_secs(60);
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
    /// Without a cgroup, what every process of the workload inherits, so a
    /// job that detached from its parent is still found.
    #[cfg(target_os = "linux")]
    marker: Option<String>,
    /// Who launched what, as the watch's first sample finds it: an agent's
    /// unless the workload is an adopted terminal.
    submissions: Submissions,
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
        #[cfg(target_os = "linux")]
        let marker = cgroup.is_none().then(sampling::new_marker);
        Self {
            unit: unit.to_string(),
            cap,
            pool,
            #[cfg(target_os = "linux")]
            cgroup,
            #[cfg(target_os = "linux")]
            marker,
            submissions: Submissions::new(true),
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
            #[cfg(target_os = "linux")]
            marker: Some(sampling::new_marker()),
            submissions: Submissions::new(true),
        }
    }

    /// For a child the previous kernel program started (hot reload): it joins
    /// its owner's unit in this program's budget, and the domain it had goes.
    /// A child that cannot move is watched by resident memory instead.
    pub(crate) fn adopted(unit: &str, previous: Option<&std::path::Path>) -> std::io::Result<Self> {
        Ok(Self::new(unit)?.taking_over(previous))
    }

    #[cfg_attr(not(target_os = "linux"), allow(unused_mut, unused_variables))]
    fn taking_over(mut self, previous: Option<&std::path::Path>) -> Self {
        #[cfg(target_os = "linux")]
        if let Some(group) = self.cgroup.take() {
            let moved = previous.is_some_and(|previous| group.take_over(previous));
            if previous.is_some() && !moved {
                // It may hold what moved before the takeover fell short: that
                // keeps running where it is, watched by its resident memory.
                group.leave_members_running();
                crate::logging::warn_with_fields(
                    "process_resources.handover",
                    "a handed-over process could not join its owner's memory domain; watching its resident memory instead",
                    serde_json::json!({"unit": self.unit}),
                );
            }
            self.cgroup = moved.then_some(group);
        }
        self
    }

    /// The commands keep the owners the previous program found for them, from
    /// the watch's first sample.
    pub(crate) fn launched_by(mut self, submissions: Submissions) -> Self {
        self.submissions = submissions;
        self
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
            let admission = match &self.marker {
                None => {
                    "printf '%s\\n' \"$$\" > \"$1/cgroup.procs\" || exit 125; shift; exec \"$@\""
                        .to_string()
                }
                Some(marker) => format!(
                    "export {}; shift; exec \"$@\"",
                    sampling::marker_entry(marker)
                ),
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
        sampling::ensure_available(pid)?;
        self.watch(sampling::ProcessTree::new(pid), None)
    }

    /// Watches `tree` as the watch of `listing`'s owner: a released workload
    /// whose previous program handed it over. It is listed, counted and
    /// stoppable again, and ends with its last process.
    fn start_released(
        self,
        tree: sampling::ProcessTree,
        listing: projection::Listing,
    ) -> std::io::Result<()> {
        sampling::ensure_available(tree.root())?;
        let domain = self.watch(tree, Some(listing))?;
        // The watch owns itself from here, as for any released workload.
        drop(domain);
        Ok(())
    }

    fn watch(
        self,
        mut tree: sampling::ProcessTree,
        listing: Option<projection::Listing>,
    ) -> std::io::Result<ProcessDomain> {
        let root = tree.root();
        #[cfg(target_os = "linux")]
        if let Some(group) = &self.cgroup {
            tree.set_cgroup(group.clone());
        }
        #[cfg(target_os = "linux")]
        if let Some(marker) = &self.marker {
            tree.set_marker(marker);
        }
        #[cfg(target_os = "linux")]
        let in_cgroup = self.cgroup.is_some();
        #[cfg(not(target_os = "linux"))]
        let in_cgroup = false;
        #[cfg(target_os = "linux")]
        let cgroup = self.cgroup.as_ref().map(|group| group.path().to_path_buf());
        #[cfg(not(target_os = "linux"))]
        let cgroup = None;
        let handed_over = listing.is_some();
        // Preserve the inherited listing before admitting the first client;
        // the asynchronous worker may not have reached its first tick yet.
        let samples = if handed_over {
            tree.sample().unwrap_or_else(|error| {
                crate::logging::warn_with_fields(
                    "process_resources.handover",
                    "inherited process observation unavailable; retaining ownership until sampling recovers",
                    serde_json::json!({"pid": root, "error": error.to_string()}),
                );
                Vec::new()
            })
        } else {
            Vec::new()
        };
        let owned = handed_over.then(|| tree.owned());
        let state = Arc::new(Mutex::new(Observation {
            samples,
            owned,
            unit: self.unit.clone(),
            cap: self.cap,
            in_cgroup,
            cgroup,
            listed: listing,
            released: handed_over,
            root,
            submissions: self.submissions,
            ..Observation::default()
        }));
        self.pool.join(&state);
        let watched = state.clone();
        let (stop, stopped) = mpsc::channel();
        state.lock().unwrap_or_else(|e| e.into_inner()).control = Some(stop.clone());
        let mut released = handed_over;
        // When the monitor last said it is waiting for the kernel's resources.
        let mut short_logged: Option<std::time::Instant> = None;
        // Per command: process-table time, resident and written bytes, and
        // their growth per second.
        let mut previous = std::collections::BTreeMap::<u32, Rates>::new();
        let mut write_growth_cache =
            std::collections::BTreeMap::<u32, disk_use::WriteGrowth>::new();
        let worker = std::thread::Builder::new()
            .name("chariox-process-resources".into())
            .spawn(move || loop {
                let ended = match tree.sample() {
                    Ok(samples) => {
                // A job that left soon after its parent may not be observed yet;
                // its cgroup still holds it, and ending the watch would kill it.
                // Without one, only a fresh process table can tell.
                #[cfg(target_os = "linux")]
                let ended = released
                    && samples.is_empty()
                    && match &self.cgroup {
                        Some(group) => group.pids().is_ok_and(|pids| pids.is_empty()),
                        None => tree.left_nothing_running(),
                    };
                #[cfg(not(target_os = "linux"))]
                let ended = released && samples.is_empty();
                // Rates use the process table's own time: watches share a
                // cached table, so a tick can see the same table again.
                let sampled_at = samples.iter().map(|p| p.sampled_at_ms).max().unwrap_or(0);
                let deleted = samples
                    .iter()
                    .map(|sample| (sample.pid, sample.deleted_open_bytes))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let mut disk_writes = std::collections::BTreeMap::new();
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
                        if write_growth >= admission::GROWING {
                            let volumes = write_growth_cache.entry(pid).or_default().sample(&pids, sampled_at);
                            disk_writes.insert(pid, volumes);
                        } else {
                            write_growth_cache.remove(&pid);
                        }
                        pool::CommandUse {
                            pid,
                            deleted_open: pids
                                .iter()
                                .filter_map(|pid| deleted.get(pid))
                                .sum(),
                            group: process_group(pid),
                            resident,
                            growth,
                            writes,
                            write_growth,
                            effective: tree.effective(pid, &samples),
                            root: tree.in_harness(pid),
                        }
                    })
                    .collect::<Vec<_>>();
                write_growth_cache.retain(|pid, _| disk_writes.contains_key(pid));
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
                    observation.disk_writes = disk_writes;
                    observation.owned = Some(tree.owned());
                }
                #[cfg(target_os = "linux")]
                let unit_charged = self.cgroup.as_ref().and_then(|group| group.unit_charged());
                #[cfg(not(target_os = "linux"))]
                let unit_charged = None;
                self.pool.enforce(&watched, unit_charged);
                        admit(&tree, &watched);
                        ended
                    }
                    // The kernel is short of descriptors or memory for a
                    // moment, which says nothing of the tree: leave it and
                    // what was last sampled alone, and read again next tick.
                    Err(sampling::SamplingError::Short(errno)) => {
                        if short_logged.is_none_or(|at| at.elapsed() >= SHORT_LOG_INTERVAL) {
                            short_logged = Some(std::time::Instant::now());
                            crate::logging::warn_with_fields(
                                "process_resources.monitor",
                                "process resource sampling is waiting for the kernel to get descriptors or memory back",
                                serde_json::json!({"pid": root, "errno": errno}),
                            );
                        }
                        false
                    }
                    // A signal ended `ps` (a hot reload ends the children the
                    // kernel leaves behind while a sample runs): the tree
                    // says nothing, so read again next tick.
                    Err(sampling::SamplingError::Ended) => false,
                    Err(error) => {
                        crate::logging::error_with_fields(
                            "process_resources.monitor",
                            "process resource sampling failed; stopping its owned tree",
                            serde_json::json!({"pid": root, "error": error.to_string()}),
                        );
                        tree.terminate();
                        break;
                    }
                };
                if ended {
                    break;
                }
                match stopped.recv_timeout(INTERVAL) {
                    #[cfg(test)]
                    Ok(WatchCommand::PauseSampling { reply, resume }) => {
                        let _ = reply.send(Ok(Vec::new()));
                        let _ = resume.recv();
                    }
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
                            tell_stopped(&watched, pid, notice);
                        }
                        tree.kill(pid);
                    }
                    Ok(WatchCommand::DiskStop { pid, notice }) => {
                        announce(pid, &notice);
                        tell_stopped(&watched, pid, notice);
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
        if handed_over {
            registry().lock().unwrap_or_else(|e| e.into_inner()).insert(
                format!("released-{:032x}", rand::random::<u128>()),
                Arc::downgrade(&state),
            );
        }
        Ok(ProcessDomain {
            state,
            stop,
            worker: Some(worker),
            released: handed_over,
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
    let now = crate::session::unix_epoch_ms();
    let (signals, changes) = {
        let mut observation = watched.lock().unwrap_or_else(|e| e.into_inner());
        let observation = &mut *observation;
        let mut signals = Vec::new();
        let mut changes = Vec::new();
        for (command, entry) in &mut observation.admission {
            let waiting = entry.wait.is_some();
            if waiting || entry.stopped {
                signals.push((*command, waiting));
            }
            let change = match (waiting, entry.stopped) {
                (true, false) => Some(waits::WaitChange::Began),
                (false, true) => Some(waits::WaitChange::Resumed {
                    waited_ms: now.saturating_sub(entry.since_ms),
                }),
                _ => None,
            };
            entry.stopped = waiting;
            if let Some(change) = change {
                let process_id = observation
                    .samples
                    .iter()
                    .find(|sample| sample.pid == *command)
                    .map(|sample| sample.process_id.clone());
                changes.push((*command, process_id, change, projection::state_of(entry)));
            }
        }
        (signals, changes)
    };
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
    for (command, process_id, change, state) in changes {
        let line = match change {
            waits::WaitChange::Began => waits::began_line(&state),
            waits::WaitChange::Resumed { waited_ms } => Some(waits::resumed_line(waited_ms)),
            waits::WaitChange::Stopped { .. } => None,
        };
        crate::logging::info_with_fields(
            "process_resources.admission_wait",
            match change {
                waits::WaitChange::Began => "a command began to wait",
                waits::WaitChange::Resumed { .. } => "a waiting command continues",
                waits::WaitChange::Stopped { .. } => "a waiting command was ended",
            },
            serde_json::json!({"pid": command, "state": format!("{state:?}"),
            "waited_ms": match change {
                waits::WaitChange::Resumed { waited_ms } => waited_ms,
                _ => 0,
            }}),
        );
        if let Some(line) = line {
            announce(command, &line);
        }
        if let Some(process_id) = process_id {
            waits::push(waits::WaitEvent {
                keys: keys_of(watched),
                process_id,
                change,
                state,
            });
        }
    }
}

/// The room hears that the pool ended the command at `pid`, and why.
fn tell_stopped(watched: &Arc<Mutex<Observation>>, pid: u32, notice: String) {
    let (process_id, state) = {
        let observation = watched.lock().unwrap_or_else(|e| e.into_inner());
        let Some(sample) = observation.samples.iter().find(|sample| sample.pid == pid) else {
            return;
        };
        (
            sample.process_id.clone(),
            observation
                .admission
                .get(&pid)
                .map_or(SessionProcessState::Running, projection::state_of),
        )
    };
    waits::push(waits::WaitEvent {
        keys: keys_of(watched),
        process_id,
        change: waits::WaitChange::Stopped {
            reason: notice.trim_end().to_string(),
        },
        state,
    });
}

/// The keys the workload is registered under.
fn keys_of(watched: &Arc<Mutex<Observation>>) -> Vec<String> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(_, state)| {
            state
                .upgrade()
                .is_some_and(|state| Arc::ptr_eq(&state, watched))
        })
        .map(|(key, _)| key.clone())
        .collect()
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
    /// The memory domain directory, which a hot reload hands to the next program.
    cgroup: Option<std::path::PathBuf>,
    /// The workload's own process.
    root: u32,
    /// Whether each command may wait for room follows who submitted the line
    /// that started it: a person's own terminal command never does.
    submissions: submissions::Submissions,
    control: Option<mpsc::Sender<WatchCommand>>,
    outside_ended: bool,
    samples: Vec<ProcessSample>,
    commands: Vec<pool::CommandUse>,
    disk_writes: std::collections::BTreeMap<u32, Vec<disk_use::VolumeWrites>>,
    /// Admission per command, by the command's top process.
    admission: std::collections::BTreeMap<u32, admission::Entry>,
    memory_stop: Option<MemoryStop>,
    /// Who Health lists the processes under when no run or terminal does.
    listed: Option<projection::Listing>,
    /// Set once the workload's own process ended: what it left running is
    /// still this owner's, listed, counted and stoppable until it ends.
    released: bool,
    /// The watched processes at the last sample, which a hot reload hands over.
    owned: Option<sampling::Owned>,
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
    /// The memory domain a hot reload hands over with the process.
    pub(crate) fn cgroup_path(&self) -> Option<std::path::PathBuf> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.cgroup.clone()
    }
    /// Who launched each command, which a hot reload hands over with the process.
    pub(crate) fn submissions(&self) -> Submissions {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.submissions.clone()
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
/// Runs `f` while this thread holds the registry lock, as worker cleanup does.
#[cfg(test)]
pub(crate) fn with_registry_held<R>(f: impl FnOnce() -> R) -> R {
    let _held = registry().lock().unwrap_or_else(|e| e.into_inner());
    f()
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

fn observation_for_key(key: &str) -> Option<Arc<Mutex<Observation>>> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .and_then(std::sync::Weak::upgrade)
}

fn with_submissions_for_key(key: &str, update: impl FnOnce(&mut submissions::Submissions)) {
    if let Some(state) = observation_for_key(key) {
        update(&mut state.lock().unwrap_or_else(|e| e.into_inner()).submissions);
    }
}

/// The workload of `key` is a terminal's shell: its own start is a person's,
/// and `reports_lines` says whether the shell marks the lines it runs.
pub(crate) fn bind_terminal_for_key(key: &str, reports_lines: bool) {
    with_submissions_for_key(key, |lines| lines.for_terminal(reports_lines));
}

/// An Enter was accepted at the prompt of the terminal bound to `key`: the
/// command its line starts may wait for room unless a person submitted it.
pub(crate) fn submit_line_for_key(key: &str, holdable: bool) {
    let Some(state) = observation_for_key(key) else {
        return;
    };
    let root = {
        let observation = state.lock().unwrap_or_else(|e| e.into_inner());
        observation
            .submissions
            .takes_over(holdable)
            .then_some(observation.root)
    };
    // Read outside the lock: the sampler waits for it.
    let alive = root.map(sampling::live_groups).unwrap_or_default();
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .submissions
        .submit(holdable, std::time::Instant::now(), &alive);
}

/// The shell of the terminal bound to `key` started the next accepted line.
pub(crate) fn line_started_for_key(key: &str) {
    with_submissions_for_key(key, |lines| lines.line_started(std::time::Instant::now()));
}

/// The shell is back at its prompt; `jobs` are the process groups of its job
/// table.
pub(crate) fn line_finished_for_key(key: &str, jobs: &[u32]) {
    with_submissions_for_key(key, |lines| {
        lines.line_finished(jobs, std::time::Instant::now())
    });
}

fn process_group(pid: u32) -> Option<u32> {
    // SAFETY: getpgid only reads the kernel's process table.
    let group = unsafe { libc::getpgid(i32::try_from(pid).ok()?) };
    u32::try_from(group).ok().filter(|group| *group > 1)
}

/// Whether the command whose top process is `pid` may wait, once the sampler
/// has seen it.
#[cfg(test)]
pub(crate) fn command_holdable_for_key(key: &str, pid: u32) -> Option<bool> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .and_then(std::sync::Weak::upgrade)
        .and_then(|state| {
            state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .admission
                .get(&pid)
                .map(|entry| entry.holdable())
        })
}

pub(crate) fn has_key(key: &str) -> bool {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .is_some_and(|state| state.strong_count() > 0)
}

#[cfg(test)]
pub(crate) fn snapshot_row_count_for_test() -> usize {
    sampling::process_rows(std::process::id())
        .map(|rows| rows.len())
        .unwrap_or(0)
}
