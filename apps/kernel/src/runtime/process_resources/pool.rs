//! The one memory ceiling every agent and terminal shares. Nothing is stopped
//! while it has room, however far one owner is above its fair share; under
//! real pressure one command yields: a fast-growing runaway, else the
//! largest command of the owner furthest above its share. Admission runs in
//! the same pass: a heavy command waits for room, and a growing one is paused
//! before anything is stopped.
use super::*;
#[cfg(target_os = "macos")]
use crate::runtime::bounded_process::BoundedCommandExt as _;
use std::collections::BTreeMap;
use std::sync::Weak;
use std::time::Instant;

/// A command growing this fast is a runaway and goes first.
const RUNAWAY_BYTES_PER_SECOND: u64 = 128 * MIB;
/// A stopped command's memory takes a moment to show as free; no second stop
/// for the same cause before then.
const SETTLE: Duration = Duration::from_secs(1);
/// A paused command whose room does not come ends instead of waiting for
/// ever. The kernel's stall signal averages ten seconds and judges a pool
/// every pass; a pause that outlasts six such windows is waiting on a pool
/// that is full by accounting but never stalls (an idle holder).
const PAUSE_LIMIT_MS: u64 = 60_000;

pub(super) struct Pool {
    pub(super) ceiling: u64,
    /// Pressure starts when less than this is left under the ceiling.
    margin: u64,
    /// The host's own free memory counts too (the kernel's shared pool).
    host: bool,
    /// Whether the machine itself reports memory pressure: little available
    /// memory or tasks stalling on it. The pool's accounting of resident
    /// bytes is not pressure (shared and file-backed pages count once per
    /// process), so on the kernel's pool nothing yields to it alone.
    machine_pressure: Box<dyn Fn() -> bool + Send + Sync>,
    #[cfg(target_os = "linux")]
    budget: Mutex<Weak<cgroup::Budget>>,
    members: Mutex<Vec<Weak<Mutex<Observation>>>>,
    stopped_at: Mutex<BTreeMap<String, Instant>>,
    pub(super) ledger: Mutex<admission::Ledger>,
    pub(super) launches: Mutex<BTreeMap<String, super::launch::Reservation>>,
    pub(super) launch_generation: std::sync::atomic::AtomicU64,
    pub(super) volume: Box<admission::Volume>,
}

/// One command of a workload: the root's own process or a subtree under it.
#[derive(Clone, Copy)]
pub(super) struct CommandUse {
    pub pid: u32,
    /// Bytes of deleted files its processes still hold open.
    pub deleted_open: u64,
    /// Its process group: the shell's job it belongs to.
    pub group: Option<u32>,
    pub resident: u64,
    pub growth: u64,
    /// The median of its growth over the last minute while it ran: what a
    /// pause is sized by, since one spike says little of how it goes on.
    pub steady_growth: u64,
    pub writes: u64,
    pub write_growth: u64,
    /// The process it runs past wrapping shells: what admission recognises.
    pub effective: u32,
    /// Part of the harness (the workload's own process and the launcher
    /// chain that started with it): never held or paused.
    pub root: bool,
}

pub(super) enum Cause {
    Pressure,
    Cap(u64),
}

/// Headroom and launch consumption from the same sample: a newer observation
/// must not release memory that this headroom has not yet charged.
pub(super) struct MemoryRoom {
    pub memory: u64,
    /// The machine's free memory, not the ceiling, set `memory`.
    pub host_bound: bool,
    pub growing: u64,
    observed: BTreeMap<String, u64>,
}

impl MemoryRoom {
    pub(super) fn remaining(
        &self,
        launches: &BTreeMap<String, super::launch::Reservation>,
    ) -> BTreeMap<String, u64> {
        launches
            .iter()
            .map(|(id, launch)| {
                (
                    id.clone(),
                    launch
                        .memory
                        .saturating_sub(self.observed.get(id).copied().unwrap_or(0)),
                )
            })
            .collect()
    }
}

impl Pool {
    #[cfg(test)]
    pub(super) fn new(ceiling: u64, host: bool) -> Self {
        Self::with(ceiling, host, None, Box::new(admission::statvfs))
    }

    fn with(
        ceiling: u64,
        host: bool,
        peaks: Option<std::path::PathBuf>,
        volume: Box<admission::Volume>,
    ) -> Self {
        let margin = (ceiling / 20).max((512 * MIB).min(ceiling / 10));
        Self {
            ceiling,
            margin,
            host,
            machine_pressure: Box::new(move || host_short(margin) || machine_stalled()),
            #[cfg(target_os = "linux")]
            budget: Mutex::new(Weak::new()),
            members: Mutex::default(),
            stopped_at: Mutex::default(),
            ledger: Mutex::new(admission::Ledger::new(peaks)),
            launches: Mutex::default(),
            launch_generation: std::sync::atomic::AtomicU64::new(0),
            volume,
        }
    }

    /// The machine's pressure is the test's to set.
    #[cfg(test)]
    pub(super) fn with_machine_pressure(
        mut self,
        pressure: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        self.machine_pressure = pressure;
        self
    }

    /// A pool whose volumes report `volume` (free, size) bytes.
    #[cfg(test)]
    pub(super) fn with_volume(ceiling: u64, volume: Box<admission::Volume>) -> Self {
        Self::with(ceiling, false, None, volume)
    }

    /// The kernel's pool: the machine (or the delegated parent's limit) minus
    /// a reserve for the kernel, the OS and the user's own apps.
    pub(super) fn shared() -> Arc<Self> {
        static SHARED: std::sync::OnceLock<Arc<Pool>> = std::sync::OnceLock::new();
        SHARED
            .get_or_init(|| {
                let ceiling = machine_memory()
                    .and_then(|total| total.checked_sub((total / 10).max(512 * MIB)))
                    .unwrap_or(u64::MAX);
                Arc::new(Self::with(
                    ceiling,
                    true,
                    Some(crate::config::default_state_dir().join("admission-peaks.json")),
                    Box::new(admission::statvfs),
                ))
            })
            .clone()
    }

    #[cfg(target_os = "linux")]
    pub(super) fn budget(&self) -> Option<Arc<cgroup::Budget>> {
        let mut slot = self.budget.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(budget) = slot.upgrade() {
            return Some(budget);
        }
        let budget = cgroup::Budget::create(self.ceiling, self.margin)?;
        *slot = Arc::downgrade(&budget);
        Some(budget)
    }

    pub(super) fn join(&self, member: &Arc<Mutex<Observation>>) {
        let mut members = self.members.lock().unwrap_or_else(|e| e.into_inner());
        members.retain(|member| member.strong_count() > 0);
        members.push(Arc::downgrade(member));
    }

    /// Waiting commands and Start now overrides, for a kernel that replaces
    /// its program in place.
    pub(super) fn held_commands(&self) -> Vec<admission::HeldCommand> {
        self.members()
            .iter()
            .flat_map(|member| {
                let observation = member.lock().unwrap_or_else(|e| e.into_inner());
                observation
                    .admission
                    .iter()
                    .filter_map(|(pid, entry)| admission::HeldCommand::of(*pid, entry))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub(super) fn members(&self) -> Vec<Arc<Mutex<Observation>>> {
        self.members
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    }

    /// Called by every workload's watch after each sample. `unit_charged` is
    /// the unit's cgroup charge when it has one.
    pub(super) fn enforce(&self, current: &Arc<Mutex<Observation>>, unit_charged: Option<u64>) {
        let members = self.members();
        let light = admission::light(self.ceiling);
        let mut ledger = self.ledger.lock().unwrap_or_else(|e| e.into_inner());
        ledger.register(current, light);
        let (unit, cap) = {
            let current = current.lock().unwrap_or_else(|e| e.into_inner());
            (current.unit.clone(), current.cap)
        };
        if let Some(cap) = cap {
            let used = unit_charged.unwrap_or_else(|| {
                members
                    .iter()
                    .map(|member| member.lock().unwrap_or_else(|e| e.into_inner()))
                    .filter(|member| member.unit == unit)
                    .map(|member| member.resident())
                    .sum()
            });
            if used >= cap {
                self.stop_one(&members, Some(&unit), Cause::Cap(cap));
            }
        }
        let sampled = self.room(&members);
        let growing = sampled.growing;
        let launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
        let launch_memory = sampled.remaining(&launches);
        // Unused predictions gate admission, not pressure. Charging them here
        // can strand a paused descendant while its parent keeps the reservation.
        let room = admission::Room {
            memory: sampled.memory,
            host_bound: sampled.host_bound,
            tight: sampled.memory < self.margin.max(growing)
                && (!self.host || self.under_pressure()),
            light,
            growth_room: admission::growth_room(self.ceiling),
            capacity: self.ceiling.saturating_sub(self.margin),
            volume: &*self.volume,
            launches: &launches,
            launch_memory,
        };
        let may_stop = ledger.pass(&members, current, &room);
        let refused = ledger.take_refused(current);
        let fillers = ledger.take_fillers(current);
        let capacity = room.capacity;
        drop(ledger);
        self.end_oversized(current, refused, capacity);
        self.end_fillers(current, fillers);
        self.end_overdue_wait(&members, &room);
        drop(launches);
        // Paused and held commands do not grow: with nothing growing the
        // pool waits rather than stops.
        if may_stop && growing > 0 && self.pressed(&members) {
            self.stop_one(&members, None, Cause::Pressure);
        }
    }

    /// A command filling a nearly full disk ends now, with the reason in its
    /// own error stream.
    fn end_fillers(
        &self,
        current: &Arc<Mutex<Observation>>,
        fillers: Vec<(u32, admission::Filling)>,
    ) {
        let control = current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .control
            .clone();
        let Some(control) = control else {
            return;
        };
        for (pid, filling) in fillers {
            let notice = super::waits::filling_line(&filling);
            crate::logging::warn_with_fields(
                "process_resources.disk_stop",
                notice.trim_end(),
                serde_json::json!({"pid": pid, "free_bytes": filling.free, "floor_bytes": filling.floor,
                    "written_bytes": filling.written, "deleted_open_bytes": filling.deleted_open,
                    "rate_bytes_per_second": filling.rate}),
            );
            let _ = control.send(WatchCommand::DiskStop { pid, notice });
        }
    }

    /// A command whose known peak exceeds the whole pool can never run here:
    /// it ends now, with the reason in its own error stream.
    fn end_oversized(
        &self,
        current: &Arc<Mutex<Observation>>,
        refused: Vec<(u32, u64)>,
        capacity: u64,
    ) {
        let control = current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .control
            .clone();
        let Some(control) = control else {
            return;
        };
        for (pid, needs) in refused {
            let notice = format!(
                "chariox: stopped this command: it needs about {} and this machine's memory pool is {}, so it can never fit here. Run it on a larger machine or lower its memory use.\n",
                super::waits::size(needs),
                super::waits::size(capacity),
            );
            crate::logging::warn_with_fields(
                "process_resources.memory_stop",
                notice.trim_end(),
                serde_json::json!({"pid": pid, "needs_bytes": needs, "pool_bytes": capacity}),
            );
            let _ = control.send(WatchCommand::MemoryStop {
                pid,
                limit_bytes: capacity,
                notice,
            });
        }
    }

    /// Memory left under the pressure line, and how fast running commands
    /// grow together (bytes per second): admission pauses before the line.
    /// Waiting commands do not count as growing.
    pub(super) fn room(&self, members: &[Arc<Mutex<Observation>>]) -> MemoryRoom {
        let mut observed = BTreeMap::<String, u64>::new();
        #[cfg(target_os = "linux")]
        let mut cgroup_observed = BTreeMap::<String, u64>::new();
        let (outside_cgroup, growing) = members
            .iter()
            .map(|member| member.lock().unwrap_or_else(|e| e.into_inner()))
            .fold((0u64, 0u64), |(outside, growing), member| {
                let resident = member.resident();
                if member.in_cgroup {
                    #[cfg(target_os = "linux")]
                    // Leaves share their unit's controller; only the unit has counters.
                    if let Some(charged) = member
                        .cgroup
                        .as_deref()
                        .and_then(std::path::Path::parent)
                        .and_then(cgroup::unreclaimable)
                    {
                        cgroup_observed
                            .entry(member.unit.clone())
                            .or_insert(charged);
                    }
                } else {
                    let memory = observed.entry(member.unit.clone()).or_default();
                    *memory = memory.saturating_add(resident);
                }
                (
                    outside + if member.in_cgroup { 0 } else { resident },
                    growing
                        + member
                            .commands
                            .iter()
                            .filter(|command| {
                                member
                                    .admission
                                    .get(&command.pid)
                                    .is_none_or(|entry| entry.wait.is_none())
                            })
                            .map(|command| command.growth)
                            .sum::<u64>(),
                )
            });
        #[cfg(target_os = "linux")]
        let budget = self
            .budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade();
        #[cfg(target_os = "linux")]
        let charged = budget.as_ref().map_or(0, |budget| budget.used());
        #[cfg(not(target_os = "linux"))]
        let charged: u64 = 0;
        // Never credit more domain charge than the aggregate headroom included.
        #[cfg(target_os = "linux")]
        {
            let mut remaining = charged;
            for (unit, memory) in cgroup_observed {
                let credited = memory.min(remaining);
                remaining -= credited;
                let memory = observed.entry(unit).or_default();
                *memory = memory.saturating_add(credited);
            }
        }
        let mut room = self
            .ceiling
            .saturating_sub(self.margin)
            .saturating_sub(charged.saturating_add(outside_cgroup));
        let mut host_bound = false;
        if self.host {
            if let Some(host) = host_room(self.margin) {
                // RSS can charge the same shared pages many times. Physical
                // headroom decides admission and resumption too, or a command
                // paused during real pressure could stay stranded after it ends.
                room = host;
                host_bound = true;
                #[cfg(target_os = "linux")]
                if budget.is_some() {
                    let budget_room = self
                        .ceiling
                        .saturating_sub(self.margin)
                        .saturating_sub(charged);
                    if budget_room < room {
                        room = budget_room;
                        host_bound = false;
                    }
                }
            }
        }
        MemoryRoom {
            memory: room,
            host_bound,
            growing,
            observed,
        }
    }

    fn pressed(&self, members: &[Arc<Mutex<Observation>>]) -> bool {
        let outside_cgroup = members
            .iter()
            .map(|member| member.lock().unwrap_or_else(|e| e.into_inner()))
            .filter(|member| !member.in_cgroup)
            .map(|member| member.resident())
            .sum::<u64>();
        #[cfg(target_os = "linux")]
        let budget = self
            .budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade();
        #[cfg(target_os = "linux")]
        let (charged, stalled) = budget
            .as_ref()
            .map_or((0, false), |budget| (budget.used(), budget.stalled()));
        #[cfg(not(target_os = "linux"))]
        let (charged, stalled): (u64, bool) = (0, false);
        stalled
            || if self.host {
                self.under_pressure()
            } else {
                charged.saturating_add(outside_cgroup) > self.ceiling.saturating_sub(self.margin)
            }
    }

    /// The workload budget can be full while the host has plenty of memory.
    /// Unlike RSS accounting, its hard limit is an actual allocation boundary.
    fn under_pressure(&self) -> bool {
        #[cfg(target_os = "linux")]
        if self
            .budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade()
            .is_some_and(|budget| budget.pressured(self.margin))
        {
            return true;
        }
        (self.machine_pressure)()
    }

    /// A command paused or held for longer than `PAUSE_LIMIT_MS` ends with the
    /// memory-stop outcome and its reason: room that did not come in that time
    /// is not coming, and it must not stay stopped with no outcome.
    fn end_overdue_wait(&self, members: &[Arc<Mutex<Observation>>], room: &admission::Room) {
        let mut stopped_at = self.stopped_at.lock().unwrap_or_else(|e| e.into_inner());
        if stopped_at.get("").is_some_and(|at| at.elapsed() < SETTLE) {
            return;
        }
        let Some((waiting, blocked)) = admission::blocked_waiting(members, PAUSE_LIMIT_MS, room)
        else {
            return;
        };
        let notice = super::waits::overdue_line(
            PAUSE_LIMIT_MS,
            &blocked,
            waiting.resident,
            if room.host_bound {
                None
            } else {
                Some(self.ceiling)
            },
        );
        if self.end_waiting(waiting, notice) {
            stopped_at.insert(String::new(), Instant::now());
        }
    }

    /// Ends a paused command with the memory-stop outcome; whether it was sent.
    fn end_waiting(&self, waiting: admission::PausedCommand, notice: String) -> bool {
        crate::logging::warn_with_fields(
            "process_resources.memory_stop",
            notice.trim_end(),
            serde_json::json!({
                "pid": waiting.pid,
                "resident_bytes": waiting.resident,
                "ceiling_bytes": self.ceiling,
                "waiting": true,
            }),
        );
        waiting
            .control
            .send(WatchCommand::MemoryStop {
                pid: waiting.pid,
                limit_bytes: self.ceiling,
                notice,
            })
            .is_ok()
    }

    fn stop_one(&self, members: &[Arc<Mutex<Observation>>], unit: Option<&str>, cause: Cause) {
        let mut stopped_at = self.stopped_at.lock().unwrap_or_else(|e| e.into_inner());
        let key = unit.unwrap_or_default().to_string();
        if stopped_at.get(&key).is_some_and(|at| at.elapsed() < SETTLE) {
            return;
        }
        // A paused command holds memory and makes no progress: it yields
        // before a running command does, the newest first.
        if matches!(cause, Cause::Pressure) {
            if let Some(waiting) = admission::youngest_paused(members) {
                let notice = format!(
                    "chariox: stopped this waiting command under memory pressure: the pool is full, so the newest waiting command ends first ({} MiB resident) and the oldest running one can finish. Run it again; it starts when there is room.\n",
                    waiting.resident / MIB
                );
                if self.end_waiting(waiting, notice) {
                    stopped_at.insert(key, Instant::now());
                }
                return;
            }
        }
        let snapshot = members
            .iter()
            .map(|member| {
                let observation = member.lock().unwrap_or_else(|e| e.into_inner());
                // A held command has not grown: stopping it frees nothing.
                let commands = observation
                    .commands
                    .iter()
                    .filter(|command| {
                        !observation
                            .admission
                            .get(&command.pid)
                            .is_some_and(|entry| {
                                matches!(entry.wait, Some(admission::Wait::Held(_)))
                            })
                    })
                    .copied()
                    .collect::<Vec<_>>();
                (
                    observation.unit.clone(),
                    commands,
                    observation.control.clone(),
                )
            })
            .collect::<Vec<_>>();
        let mut units = BTreeMap::<&str, u64>::new();
        for (owner, commands, _) in &snapshot {
            *units.entry(owner).or_default() += commands.iter().map(|c| c.resident).sum::<u64>();
        }
        let share = self.ceiling / units.len().max(1) as u64;
        let units = &units;
        let candidates = snapshot
            .iter()
            .filter(|(owner, _, _)| unit.is_none_or(|unit| unit == owner))
            .flat_map(|(owner, commands, control)| {
                commands
                    .iter()
                    .map(move |command| (owner, *command, control))
            })
            .collect::<Vec<_>>();
        // A runaway soon passes every share: it goes first. Otherwise the
        // owner furthest above its share (shares are equal) gives up its
        // largest command.
        let runaway = candidates
            .iter()
            .filter(|(_, command, _)| command.growth >= RUNAWAY_BYTES_PER_SECOND)
            .max_by_key(|(_, command, _)| command.growth);
        let Some((owner, command, Some(control))) = runaway.or_else(|| {
            candidates
                .iter()
                .max_by_key(|(owner, command, _)| (units[owner.as_str()], command.resident))
        }) else {
            return;
        };
        let owner_bytes = units[owner.as_str()];
        let (limit_bytes, notice) = match cause {
            Cause::Cap(cap) => (
                cap,
                format!(
                    "chariox: stopped this command at its owner-set memory cap of {} MiB ({} MiB in use)\n",
                    cap / MIB,
                    owner_bytes / MIB
                ),
            ),
            Cause::Pressure => (
                share,
                format!(
                    "chariox: stopped this command under memory pressure: {} MiB resident, {} MiB for its owner, fair share {} MiB of the {} MiB shared ceiling\n",
                    command.resident / MIB,
                    owner_bytes / MIB,
                    share / MIB,
                    self.ceiling / MIB
                ),
            ),
        };
        crate::logging::warn_with_fields(
            "process_resources.memory_stop",
            notice.trim_end(),
            serde_json::json!({
                "pid": command.pid,
                "resident_bytes": command.resident,
                "growth_bytes_per_second": command.growth,
                "owner_bytes": owner_bytes,
                "share_bytes": share,
                "ceiling_bytes": self.ceiling,
                "limit_bytes": limit_bytes,
            }),
        );
        if control
            .send(WatchCommand::MemoryStop {
                pid: command.pid,
                limit_bytes,
                notice,
            })
            .is_ok()
        {
            stopped_at.insert(key, Instant::now());
        }
    }
}

#[cfg(target_os = "linux")]
fn machine_memory() -> Option<u64> {
    let memory = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total = memory
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)?;
    for limited in cgroup::bounding() {
        if let Some(limit) = std::fs::read_to_string(limited.join("memory.max"))
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
        {
            total = total.min(limit);
        }
    }
    Some(total)
}

#[cfg(target_os = "macos")]
fn machine_memory() -> Option<u64> {
    let output = crate::process_spawn::command("sysctl")
        .args(["-n", "hw.memsize"])
        .bounded_output()
        .ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn machine_memory() -> Option<u64> {
    None
}

/// The host itself is short: the user's apps or other cgroups took the room.
#[cfg(target_os = "linux")]
fn host_short(margin: u64) -> bool {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return false;
    };
    let available = text
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|line| line.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|kib| kib.saturating_mul(1024));
    available.is_some_and(|bytes| bytes < margin)
        || cgroup::bounding()
            .iter()
            .any(|limited| cgroup::headroom(limited).is_some_and(|bytes| bytes < margin))
}

/// Tasks of the machine stall on memory a tenth of the time or more.
#[cfg(target_os = "linux")]
fn machine_stalled() -> bool {
    std::fs::read_to_string("/proc/pressure/memory").is_ok_and(|text| {
        text.lines()
            .find_map(|line| line.strip_prefix("full avg10="))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<f64>().ok())
            .is_some_and(|share| share >= 10.0)
    })
}

#[cfg(not(target_os = "linux"))]
fn machine_stalled() -> bool {
    false
}

/// Memory the host itself has left above `margin`: the user's apps or other
/// cgroups may have taken the room.
#[cfg(target_os = "linux")]
fn host_room(margin: u64) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let available = text
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|line| line.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())?
        .saturating_mul(1024);
    let room = cgroup::bounding()
        .iter()
        .filter_map(|limited| cgroup::headroom(limited))
        .fold(available, u64::min);
    Some(room.saturating_sub(margin))
}

#[cfg(not(target_os = "linux"))]
fn host_room(margin: u64) -> Option<u64> {
    host_short(margin).then_some(0)
}

/// macOS reports its own memory pressure; act only when it is critical.
#[cfg(target_os = "macos")]
fn host_short(_margin: u64) -> bool {
    let mut level: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: the name is NUL-terminated and the output buffer is one c_int.
    let read = unsafe {
        libc::sysctlbyname(
            c"kern.memorystatus_vm_pressure_level".as_ptr(),
            (&mut level as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    read == 0 && level >= 4
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn host_short(_margin: u64) -> bool {
    false
}
