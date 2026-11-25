//! Last-resort resource intervention for kernel-owned workloads. Physical
//! headroom decides memory emergencies; accounting and predicted peaks alone
//! must not interrupt a healthy machine. Each resource has an independent
//! machine-local permission, and disabling it releases its waiting commands.
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
    protection: std::sync::RwLock<crate::config::ResourceProtection>,
    /// Pressure starts when less than this is left under the ceiling.
    margin: u64,
    /// The host's own free memory counts too (the kernel's shared pool).
    host: bool,
    /// Whether the machine itself reports memory pressure: little available
    /// memory or tasks stalling on it. The pool's accounting of resident
    /// bytes is not pressure (shared and file-backed pages count once per
    /// process), so on the kernel's pool nothing yields to it alone.
    machine_pressure: Box<dyn Fn() -> bool + Send + Sync>,
    /// The machine's memory pressure and pool use for real exhaustion; none
    /// where the host reports neither.
    gauge: Box<dyn Fn() -> Option<exhaustion::Gauge> + Send + Sync>,
    exhaustion: Mutex<exhaustion::Exhaustion>,
    /// The unit whose `memory.high` the throttle lowered, to lift it again.
    throttled: Mutex<Option<std::path::PathBuf>>,
    /// When the kernel's own memory protection was last compared to its need.
    protection_checked: Mutex<Option<Instant>>,
    /// When the machine's pressure was last logged, at most every 10 s.
    gauge_logged: Mutex<Option<Instant>>,
    /// A hot reload is replacing the program: nothing new is held.
    handing_over: std::sync::atomic::AtomicBool,
    /// Holds and the handover's release exclude each other (see `hold_gate`).
    hold_gate: Mutex<()>,
    #[cfg(target_os = "linux")]
    budget: Mutex<Weak<cgroup::Budget>>,
    members: Mutex<Vec<Weak<Mutex<Observation>>>>,
    stopped_at: Mutex<BTreeMap<String, Instant>>,
    pub(super) ledger: Mutex<admission::Ledger>,
    pub(super) launches: Mutex<BTreeMap<String, super::launch::Reservation>>,
    /// Launches whose hold was logged: one line when it begins, one when it
    /// starts, however often the queue asks again.
    pub(super) launch_waiting: Mutex<std::collections::BTreeSet<String>>,
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
    /// The latest measured rate, without the runaway detector's decay. Only
    /// current growth can justify stopping work after a successful pause.
    pub measured_growth: u64,
    /// The median of its growth over the last minute while it ran: what a
    /// pause is sized by, since one spike says little of how it goes on.
    pub steady_growth: u64,
    pub writes: u64,
    /// Its latest write rate, held and halved per sample: says it writes now.
    pub write_growth: u64,
    /// What it wrote per second over the last two seconds: how fast it fills a
    /// volume, since one sample of a writer that writes in bursts says little.
    pub write_rate: u64,
    /// The process it runs past wrapping shells: what admission recognises.
    pub effective: u32,
    /// Part of the harness (the workload's own process and the launcher
    /// chain that started with it): never held or paused.
    pub root: bool,
}

pub(super) enum Cause {
    Pressure,
    /// Memory stayed short while commands paused for it freed nothing.
    Stalled(Duration),
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
        // At most 512 MiB of physical headroom: a large host must not reserve
        // gigabytes or hold routine work merely because predictions overlap.
        let margin = (ceiling / 100).clamp(64 * MIB, 512 * MIB).min(ceiling / 10);
        Self {
            ceiling,
            protection: std::sync::RwLock::new(crate::config::ResourceProtection::default()),
            margin,
            host,
            machine_pressure: Box::new(move || host_short(margin)),
            #[cfg(not(test))]
            gauge: Box::new(host_gauge),
            #[cfg(test)]
            gauge: Box::new(|| None),
            exhaustion: Mutex::default(),
            throttled: Mutex::new(None),
            gauge_logged: Mutex::new(None),
            handing_over: std::sync::atomic::AtomicBool::new(false),
            hold_gate: Mutex::new(()),
            protection_checked: Mutex::new(None),
            #[cfg(target_os = "linux")]
            budget: Mutex::new(Weak::new()),
            members: Mutex::default(),
            stopped_at: Mutex::default(),
            ledger: Mutex::new(admission::Ledger::new(peaks)),
            launches: Mutex::default(),
            launch_waiting: Mutex::default(),
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

    /// The machine's memory pressure and pool use are the test's to set.
    #[cfg(test)]
    pub(super) fn with_gauge(
        mut self,
        gauge: Box<dyn Fn() -> Option<exhaustion::Gauge> + Send + Sync>,
    ) -> Self {
        self.gauge = gauge;
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
                // The machine less what the kernel holds and the OS reserve
                // (Miguel 11-23): one agent may use nearly all the rest.
                let ceiling = machine_memory()
                    .map(|total| exhaustion::pool_bytes(total, exhaustion::kernel_use()))
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
        let budget = cgroup::Budget::create()?;
        *slot = Arc::downgrade(&budget);
        Some(budget)
    }

    pub(super) fn configure(&self, settings: crate::config::ResourceProtection) {
        {
            let mut current = self.protection.write().unwrap_or_else(|e| e.into_inner());
            if *current == settings {
                return;
            }
            *current = settings;
        }
        for member in self.members() {
            let control = member
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .control
                .clone();
            if let Some(control) = control {
                let _ = control.send(WatchCommand::Apply);
            }
        }
    }

    pub(super) fn protection(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, crate::config::ResourceProtection> {
        self.protection.read().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn capacity(&self) -> u64 {
        self.ceiling.saturating_sub(self.margin)
    }

    pub(super) fn memory_emergency(&self) -> bool {
        !self.host || self.under_pressure()
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
        let protection = self.protection();
        let members = self.members();
        self.exhaust(&members, current, protection.memory);
        let light = admission::light(self.ceiling);
        let mut ledger = self.ledger.lock().unwrap_or_else(|e| e.into_inner());
        ledger.register(current, light);
        let (unit, cap) = {
            let current = current.lock().unwrap_or_else(|e| e.into_inner());
            (current.unit.clone(), current.cap)
        };
        if let Some(cap) = cap.filter(|_| protection.memory) {
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
            memory: if protection.memory {
                sampled.memory
            } else {
                u64::MAX
            },
            margin: self.margin,
            growing,
            host_bound: sampled.host_bound,
            pressured: protection.memory && (!self.host || self.under_pressure()),
            memory_protection: protection.memory,
            disk_protection: protection.disk,
            light,
            growth_room: admission::growth_room(self.ceiling),
            capacity: self.ceiling.saturating_sub(self.margin),
            volume: &*self.volume,
            launches: &launches,
            launch_memory,
        };
        let may_stop = ledger.pass(&members, current, &room);
        let stalled = ledger
            .stalled(&members, &room)
            .then_some(ledger.stall_window);
        let refused = ledger.take_refused(current);
        let fillers = ledger.take_fillers(current);
        let capacity = room.capacity;
        drop(ledger);
        if room.pressured {
            self.end_oversized(current, refused, capacity);
            self.end_overdue_wait(&members, &room);
        }
        if protection.disk {
            self.end_fillers(current, fillers);
        }
        if let Some(window) = stalled {
            drop(launches);
            self.stop_one(&members, None, Cause::Stalled(window));
            return;
        }
        drop(launches);
        // Paused and held commands do not grow: with nothing growing the
        // pool waits rather than stops.
        if protection.memory && may_stop && growing > 0 {
            self.stop_under_pressure(&members);
        }
    }

    /// Real memory exhaustion: when the machine's memory pressure holds and the
    /// pool is nearly full, the command that grew most is throttled, paused
    /// and stopped, and its agent told each step.
    fn exhaust(
        &self,
        members: &[Arc<Mutex<Observation>>],
        current: &Arc<Mutex<Observation>>,
        protected: bool,
    ) {
        {
            let mut checked = self
                .protection_checked
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if checked.is_none_or(|at| at.elapsed() >= Duration::from_secs(60)) {
                *checked = Some(Instant::now());
                drop(checked);
                super::check_kernel_protection();
            }
        }
        if self.handing_over.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let candidates = admission::exhaustion_candidates(members);
        let gauge = if protected { (self.gauge)() } else { None };
        let Some(gauge) = gauge else {
            // Protection off, or nothing to read: nothing stays held by it.
            let mut machine = self.exhaustion.lock().unwrap_or_else(|e| e.into_inner());
            let actions = machine.release_all();
            drop(machine);
            for pid in actions {
                if let Some(found) = candidates.iter().find(|found| found.pid == pid) {
                    self.carry_release(found, current);
                }
            }
            self.lift_throttle();
            return;
        };
        let commands = candidates
            .iter()
            .map(|found| (found.pid, found.resident))
            .collect::<Vec<_>>();
        if gauge.psi >= exhaustion::PSI_CLEAR_PERCENT
            || gauge.pool_percent() >= exhaustion::POOL_TRIGGER_PERCENT
        {
            let mut logged = self.gauge_logged.lock().unwrap_or_else(|e| e.into_inner());
            if logged.is_none_or(|at| at.elapsed() >= Duration::from_secs(10)) {
                *logged = Some(Instant::now());
                let growers = self
                    .exhaustion
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .growths(Instant::now(), &commands);
                crate::logging::info_with_fields(
                    "process_resources.exhaustion_gauge",
                    "memory pressure or pool use is high",
                    serde_json::json!({
                        "psi_some_avg10": gauge.psi,
                        "pool_percent": gauge.pool_percent(),
                        "pool_bytes": gauge.pool_bytes,
                        "pool_used_bytes": gauge.pool_used,
                        "candidates": candidates.len(),
                        "top_growers": growers.iter().map(|(pid, resident, grew)| serde_json::json!({"pid": pid, "resident": resident, "grew_over_window": grew, "root": candidates.iter().find(|c| c.pid == *pid).is_some_and(|c| c.root)})).collect::<Vec<_>>(),
                    }),
                );
            }
        }
        let actions = self
            .exhaustion
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tick(Instant::now(), gauge, &commands);
        for action in actions {
            let Some(found) = candidates.iter().find(|found| found.pid == action.pid) else {
                if action.step == exhaustion::Step::Release {
                    self.lift_throttle();
                }
                continue;
            };
            let member = &found.member;
            let command = member
                .lock()
                .map(|observation| admission::command_text(&observation, action.pid))
                .unwrap_or_default();
            crate::logging::warn_with_fields(
                "process_resources.exhaustion",
                match action.step {
                    exhaustion::Step::Throttle => "memory exhaustion: throttled the top grower",
                    exhaustion::Step::Pause => "memory exhaustion: paused the top grower",
                    exhaustion::Step::Stop => "memory exhaustion: stopped the top grower",
                    exhaustion::Step::Release => "memory exhaustion ended: released the command",
                },
                serde_json::json!({
                    "pid": action.pid,
                    "command": command,
                    "grew_bytes": action.grew,
                    "resident_bytes": action.resident,
                    "psi_some_avg10": action.gauge.psi,
                    "pool_percent": action.gauge.pool_percent(),
                    "pool_bytes": action.gauge.pool_bytes,
                    "repeat": action.repeat,
                }),
            );
            match action.step {
                exhaustion::Step::Throttle => self.carry_throttle(&command, &action, member),
                exhaustion::Step::Pause if found.root => {
                    // The workload itself (a job, a provider): its whole domain holds.
                    Self::freeze(
                        member,
                        action.pid,
                        true,
                        &command,
                        exhaustion::paused_line(&command, &action),
                    );
                }
                exhaustion::Step::Pause => {
                    let note = admission::ExhaustionNote {
                        paused: exhaustion::paused_line(&command, &action),
                        resumed: exhaustion::resumed_line(&command),
                    };
                    // Under the gate: a handover that has begun holds nothing more.
                    let gate = self.hold_gate();
                    let paused = !self.handing_over()
                        && admission::exhaustion_pause(member, action.pid, note);
                    drop(gate);
                    if paused && !Arc::ptr_eq(member, current) {
                        Self::apply(member);
                    }
                }
                exhaustion::Step::Stop => self.carry_stop(&command, &action, found),
                exhaustion::Step::Release => {
                    self.lift_throttle();
                    self.carry_release_with(found, current, &command);
                }
            }
        }
    }

    fn apply(member: &Arc<Mutex<Observation>>) {
        let control = member
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .control
            .clone();
        if let Some(control) = control {
            let _ = control.send(WatchCommand::Apply);
        }
    }

    fn carry_release(&self, found: &admission::Candidate, current: &Arc<Mutex<Observation>>) {
        self.carry_release_with(found, current, "");
    }

    fn carry_release_with(
        &self,
        found: &admission::Candidate,
        current: &Arc<Mutex<Observation>>,
        command: &str,
    ) {
        if found.root {
            Self::freeze(
                &found.member,
                found.pid,
                false,
                command,
                exhaustion::resumed_line(command),
            );
            return;
        }
        admission::exhaustion_release(&found.member, found.pid);
        if !Arc::ptr_eq(&found.member, current) {
            Self::apply(&found.member);
        }
    }

    fn freeze(
        member: &Arc<Mutex<Observation>>,
        pid: u32,
        freeze: bool,
        command: &str,
        message: String,
    ) {
        let control = member
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .control
            .clone();
        if let Some(control) = control {
            let _ = control.send(WatchCommand::ExhaustFreeze {
                pid,
                freeze,
                command: command.to_string(),
                message,
            });
        }
    }

    /// Taken around every hold or release of a workload and around the whole
    /// handover release: one at a time, so a hold in flight finishes (and is
    /// recorded) before the release looks, and later ones see the handover.
    pub(super) fn hold_gate(&self) -> std::sync::MutexGuard<'_, ()> {
        self.hold_gate.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether a hot reload is replacing the program: nothing new is held.
    pub(super) fn handing_over(&self) -> bool {
        self.handing_over.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The program is being replaced (hot reload): what memory exhaustion holds
    /// is let go and nothing new is held, so no process stays frozen under a
    /// program that does not know it (the next one judges the machine afresh).
    pub(super) fn release_for_handover(&self) {
        // Held to the end: no hold can start or finish in the middle of the release.
        let _gate = self.hold_gate();
        self.handing_over
            .store(true, std::sync::atomic::Ordering::SeqCst);
        for member in self.members() {
            let (domain, pids) = {
                let mut observation = member.lock().unwrap_or_else(|e| e.into_inner());
                if observation.frozen.is_empty() {
                    continue;
                }
                observation.frozen.clear();
                (
                    observation.cgroup.clone(),
                    observation
                        .samples
                        .iter()
                        .map(|sample| sample.pid)
                        .collect::<Vec<_>>(),
                )
            };
            #[cfg(target_os = "linux")]
            if let Some(domain) = domain {
                let _ = std::fs::write(domain.join("cgroup.freeze"), "0");
            }
            #[cfg(not(target_os = "linux"))]
            let _ = domain;
            // Without a cgroup the freeze was a stop signal to each process.
            #[cfg(unix)]
            for pid in pids {
                if pid > 1 && pid <= i32::MAX as u32 {
                    // SAFETY: a positive pid of a process this watch sampled a moment ago.
                    unsafe { libc::kill(pid as i32, libc::SIGCONT) };
                }
            }
            #[cfg(not(unix))]
            let _ = pids;
        }
        self.lift_throttle();
    }

    /// Slows the owner's memory growth: its unit may not grow past what it holds.
    /// A workload without a cgroup has no such control; it is paused later.
    fn carry_throttle(
        &self,
        command: &str,
        action: &exhaustion::Action,
        member: &Arc<Mutex<Observation>>,
    ) {
        #[cfg(target_os = "linux")]
        {
            // Under the gate: the handover lifts the throttle and nothing is lowered after it.
            let _gate = self.hold_gate();
            if self.handing_over() {
                return;
            }
            let unit = member
                .lock()
                .ok()
                .and_then(|observation| observation.cgroup.clone())
                .and_then(|domain| domain.parent().map(std::path::Path::to_path_buf));
            if let Some(unit) = unit {
                let held = std::fs::read_to_string(unit.join("memory.current"))
                    .ok()
                    .and_then(|text| text.trim().parse::<u64>().ok());
                if let Some(held) = held {
                    if std::fs::write(unit.join("memory.high"), held.max(64 * MIB).to_string())
                        .is_ok()
                    {
                        *self.throttled.lock().unwrap_or_else(|e| e.into_inner()) = Some(unit);
                    }
                }
            }
        }
        let message = exhaustion::throttled_line(command, action);
        Self::tell(
            member,
            action.pid,
            message,
            waits::ExhaustionStep::Throttled,
        );
    }

    fn lift_throttle(&self) {
        #[cfg(target_os = "linux")]
        if let Some(unit) = self
            .throttled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = std::fs::write(unit.join("memory.high"), "max");
        }
    }

    fn carry_stop(&self, command: &str, action: &exhaustion::Action, found: &admission::Candidate) {
        let member = &found.member;
        let notice = exhaustion::stopped_line(command, action);
        if found.root {
            // The workload itself ends with its typed outcome; the agent and
            // the pane hear the stop here.
            Self::tell(
                member,
                action.pid,
                notice.clone(),
                waits::ExhaustionStep::Stopped,
            );
        } else {
            admission::exhaustion_mark(
                member,
                action.pid,
                admission::ExhaustionNote {
                    paused: String::new(),
                    resumed: String::new(),
                },
            );
        }
        let control = member
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .control
            .clone();
        if let Some(control) = control {
            let _ = control.send(WatchCommand::MemoryStop {
                pid: action.pid,
                limit_bytes: action.gauge.pool_bytes,
                notice,
            });
        }
        self.lift_throttle();
    }

    /// Tells the command's own error stream, its agent and its pane.
    pub(super) fn tell(
        member: &Arc<Mutex<Observation>>,
        pid: u32,
        message: String,
        step: waits::ExhaustionStep,
    ) {
        announce(pid, &message);
        let process_id = member
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .samples
            .iter()
            .find(|sample| sample.pid == pid)
            .map(|sample| sample.process_id.clone());
        // A frozen workload reads as paused, as in the process list.
        let state = member
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .frozen
            .get(&pid)
            .filter(|_| step == waits::ExhaustionStep::Paused)
            .map_or(SessionProcessState::Running, |(since_ms, command)| {
                super::projection::frozen_state(command, *since_ms)
            });
        if let Some(process_id) = process_id {
            waits::push(waits::WaitEvent {
                keys: keys_of(member),
                process_id,
                change: waits::WaitChange::Exhaustion {
                    step,
                    message: message.trim_end().to_string(),
                },
                state,
                owner: super::listed_owner(member),
            });
        }
    }

    fn stop_under_pressure(&self, members: &[Arc<Mutex<Observation>>]) {
        if self.pressed(members) {
            self.stop_one(members, None, Cause::Pressure);
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
                            .map(|command| command.measured_growth)
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
        let charged = budget.as_ref().map_or(0, |budget| budget.used());
        #[cfg(not(target_os = "linux"))]
        let charged: u64 = 0;
        if self.host {
            self.under_pressure()
        } else {
            charged.saturating_add(outside_cgroup) > self.ceiling.saturating_sub(self.margin)
        }
    }

    /// Only physical host/ancestor headroom is an emergency. The kernel's
    /// accounting domain does not impose an artificial allocation boundary.
    fn under_pressure(&self) -> bool {
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
        if self.end_waiting(waiting, notice, false) {
            stopped_at.insert(String::new(), Instant::now());
        }
    }

    /// Ends a paused command with the memory-stop outcome; whether it was sent.
    fn end_waiting(
        &self,
        waiting: admission::PausedCommand,
        notice: String,
        stalled: bool,
    ) -> bool {
        crate::logging::warn_with_fields(
            "process_resources.memory_stop",
            notice.trim_end(),
            serde_json::json!({
                "pid": waiting.pid,
                "resident_bytes": waiting.resident,
                "ceiling_bytes": self.ceiling,
                "waiting": true,
                "stalled": stalled,
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
        // before a running command does, the newest first. When waiting
        // freed nothing, the largest one yields.
        let paused = match cause {
            Cause::Pressure => admission::youngest_paused(members).map(|waiting| {
                let notice = format!(
                    "chariox: stopped this waiting command under memory pressure: the pool is full, so the newest waiting command ends first ({} MiB resident) and the oldest running one can finish. Run it again; it starts when there is room.\n",
                    waiting.resident / MIB
                );
                (waiting, notice)
            }),
            Cause::Stalled(window) => admission::largest_paused(members).map(|waiting| {
                let paused_for = crate::session::unix_epoch_ms().saturating_sub(waiting.since_ms);
                let notice = format!(
                    "chariox: stopped this command: it was paused for memory for {} and the {} MiB it holds is the largest of the paused commands. The pool gained no room for {} s while the paused commands freed nothing, so waiting longer would not help. Run it again; it starts when there is room.\n",
                    super::waits::duration(paused_for),
                    waiting.resident / MIB,
                    window.as_secs()
                );
                (waiting, notice)
            }),
            Cause::Cap(_) => None,
        };
        if let Some((waiting, notice)) = paused {
            if self.end_waiting(waiting, notice, matches!(cause, Cause::Stalled(_))) {
                stopped_at.insert(key, Instant::now());
            }
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
            Cause::Stalled(_) => return,
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

/// Memory pressure and pool use of the machine the workloads run on: the
/// tightest limited cgroup above them when there is one (a small machine is
/// emulated that way), else the host.
#[cfg(all(target_os = "linux", not(test)))]
fn host_gauge() -> Option<exhaustion::Gauge> {
    let total = machine_memory()?;
    let available = host_room(0)?;
    let limited = cgroup::bounding()
        .into_iter()
        .filter_map(|dir| {
            let limit = std::fs::read_to_string(dir.join("memory.max"))
                .ok()?
                .trim()
                .parse::<u64>()
                .ok()?;
            Some((limit, dir))
        })
        .min_by_key(|(limit, _)| *limit)
        .and_then(|(_, dir)| std::fs::read_to_string(dir.join("memory.pressure")).ok());
    let pressure = limited
        .or_else(|| std::fs::read_to_string("/proc/pressure/memory").ok())
        .and_then(|text| exhaustion::psi_some_avg10(&text))?;
    let kernel = exhaustion::kernel_use();
    Some(exhaustion::Gauge {
        psi: pressure,
        pool_bytes: exhaustion::pool_bytes(total, kernel),
        pool_used: total.saturating_sub(available).saturating_sub(kernel),
    })
}

#[cfg(all(not(target_os = "linux"), not(test)))]
fn host_gauge() -> Option<exhaustion::Gauge> {
    None
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
    meminfo_below_margin(&text, margin)
        || cgroup::bounding()
            .iter()
            .any(|limited| cgroup::headroom(limited).is_some_and(|bytes| bytes < margin))
}

#[cfg(target_os = "linux")]
fn meminfo_below_margin(text: &str, margin: u64) -> bool {
    text.lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|line| line.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|kib| kib.saturating_mul(1024) < margin)
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn sysstat_headroom_gates_the_typed_runaway_stop() {
    let (control, stopped) = std::sync::mpsc::channel();
    let command = |pid, resident, growth| CommandUse {
        pid,
        deleted_open: 0,
        group: None,
        resident,
        growth,
        measured_growth: growth,
        steady_growth: growth,
        writes: 0,
        write_growth: 0,
        write_rate: 0,
        effective: pid,
        root: true,
    };
    let members = [
        ("healthy", command(2, 80 * 1024 * MIB, 0)),
        (
            "runaway",
            command(3, 20 * 1024 * MIB, RUNAWAY_BYTES_PER_SECOND),
        ),
    ]
    .map(|(unit, command)| {
        Arc::new(Mutex::new(Observation {
            unit: unit.into(),
            commands: vec![command],
            control: Some(control.clone()),
            ..Observation::default()
        }))
    });
    let pool_for = |meminfo: &'static str| {
        Pool::new(112 * 1024 * MIB, true)
            .with_machine_pressure(Box::new(move || meminfo_below_margin(meminfo, 512 * MIB)))
    };
    // Extremes of the six b3 sysstat samples during the outage. High PSI
    // and swap occupancy do not enter the host-headroom decision.
    for meminfo in [
        "MemAvailable: 1266836 kB\nSwapTotal: 17825780 kB\nSwapFree: 56 kB\n",
        "MemAvailable: 1532584 kB\nSwapTotal: 17825780 kB\nSwapFree: 116 kB\n",
    ] {
        pool_for(meminfo).stop_under_pressure(&members);
        assert!(stopped.try_recv().is_err());
    }

    // Synthetic no-swap control below the existing margin, with the same work.
    pool_for("MemAvailable: 524287 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n")
        .stop_under_pressure(&members);
    match stopped
        .try_recv()
        .expect("the controller should stop a command")
    {
        WatchCommand::MemoryStop {
            pid,
            limit_bytes,
            notice,
        } => {
            assert_eq!(pid, 3);
            assert_eq!(limit_bytes, 56 * 1024 * MIB);
            assert!(notice.contains("under memory pressure"));
        }
        _ => panic!("expected the typed memory stop command"),
    }
    assert!(stopped.try_recv().is_err());
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
