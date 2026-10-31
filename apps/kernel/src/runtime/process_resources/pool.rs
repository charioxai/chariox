//! The one memory ceiling every agent and terminal shares. Nothing is stopped
//! while it has room, however far one owner is above its fair share; under
//! real pressure one command yields: a fast-growing runaway, else the
//! largest command of the owner furthest above its share. Admission runs in
//! the same pass: a heavy command waits for room, and a growing one is paused
//! before anything is stopped.
use super::*;
use std::collections::BTreeMap;
use std::sync::Weak;
use std::time::Instant;

/// A command growing this fast is a runaway and goes first.
const RUNAWAY_BYTES_PER_SECOND: u64 = 128 * MIB;
/// A stopped command's memory takes a moment to show as free; no second stop
/// for the same cause before then.
const SETTLE: Duration = Duration::from_secs(1);

pub(super) struct Pool {
    ceiling: u64,
    /// Pressure starts when less than this is left under the ceiling.
    margin: u64,
    /// The host's own free memory counts too (the kernel's shared pool).
    host: bool,
    #[cfg(target_os = "linux")]
    budget: Mutex<Weak<cgroup::Budget>>,
    members: Mutex<Vec<Weak<Mutex<Observation>>>>,
    stopped_at: Mutex<BTreeMap<String, Instant>>,
    ledger: Mutex<admission::Ledger>,
    volume: Box<admission::Volume>,
}

/// One command of a workload: the root's own process or a subtree under it.
#[derive(Clone, Copy)]
pub(super) struct CommandUse {
    pub pid: u32,
    pub resident: u64,
    pub growth: u64,
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
        Self {
            ceiling,
            margin: (ceiling / 20).max((512 * MIB).min(ceiling / 10)),
            host,
            #[cfg(target_os = "linux")]
            budget: Mutex::new(Weak::new()),
            members: Mutex::default(),
            stopped_at: Mutex::default(),
            ledger: Mutex::new(admission::Ledger::new(peaks)),
            volume,
        }
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

    fn members(&self) -> Vec<Arc<Mutex<Observation>>> {
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
        let (memory, growing) = self.room(&members);
        let room = admission::Room {
            memory,
            tight: memory < self.margin.max(growing),
            light,
            growth_room: admission::growth_room(self.ceiling),
            volume: &*self.volume,
        };
        let may_stop = ledger.pass(&members, current, &room);
        drop(ledger);
        // Paused and held commands do not grow: with nothing growing the
        // pool waits rather than stops.
        if may_stop && growing > 0 && self.pressed(&members) {
            self.stop_one(&members, None, Cause::Pressure);
        }
    }

    /// Memory left under the pressure line, and how fast running commands
    /// grow together (bytes per second): admission pauses before the line.
    /// Waiting commands do not count as growing.
    fn room(&self, members: &[Arc<Mutex<Observation>>]) -> (u64, u64) {
        let (outside_cgroup, growing) = members
            .iter()
            .map(|member| member.lock().unwrap_or_else(|e| e.into_inner()))
            .fold((0u64, 0u64), |(outside, growing), member| {
                (
                    outside
                        + if member.in_cgroup {
                            0
                        } else {
                            member.resident()
                        },
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
        let charged = self
            .budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade()
            .map_or(0, |budget| budget.used());
        #[cfg(not(target_os = "linux"))]
        let charged = 0;
        let mut room = self
            .ceiling
            .saturating_sub(self.margin)
            .saturating_sub(charged.saturating_add(outside_cgroup));
        if self.host {
            if let Some(host) = host_room(self.margin) {
                room = room.min(host);
            }
        }
        (room, growing)
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
        let (charged, stalled) = (0, false);
        stalled
            || charged.saturating_add(outside_cgroup) > self.ceiling.saturating_sub(self.margin)
            || (self.host && host_short(self.margin))
    }

    fn stop_one(&self, members: &[Arc<Mutex<Observation>>], unit: Option<&str>, cause: Cause) {
        let mut stopped_at = self.stopped_at.lock().unwrap_or_else(|e| e.into_inner());
        let key = unit.unwrap_or_default().to_string();
        if stopped_at.get(&key).is_some_and(|at| at.elapsed() < SETTLE) {
            return;
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
        .output()
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
