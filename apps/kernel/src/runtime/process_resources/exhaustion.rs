//! Real memory exhaustion. The machine is out of memory when its memory
//! pressure stays high AND the pool is nearly full; free memory alone (the
//! old 512 MiB line) never fired on the incident of 2026-11-23, when b3 thrashed
//! at 25 GB available. Then the one command that grows most is throttled,
//! paused and stopped, step by step, and its agent is told each step with the
//! numbers. Agents otherwise use everything the kernel and the OS leave.
//!
//! The decisions live here as plain state; `Pool` reads the machine and carries
//! the steps out.
use super::MIB;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// Every number below: Miguel 11-23 (approved 2026-11-23, via orch15).

/// Memory pressure (PSI "some", 10 s average) at or above this is exhaustion
/// when it lasts and the pool is nearly full. Miguel 11-23.
pub(super) const PSI_TRIGGER_PERCENT: f64 = 20.0;
/// How long the pressure and the full pool must hold together before the kernel
/// acts. Miguel 11-23.
pub(super) const PSI_SUSTAIN: Duration = Duration::from_secs(5);
/// The pool is nearly full from this share of it in use. It only filters out
/// page-cache churn with little memory behind it. Miguel 11-23.
pub(super) const POOL_TRIGGER_PERCENT: u64 = 70;
/// Memory pressure below this is over. Miguel 11-23.
pub(super) const PSI_CLEAR_PERCENT: f64 = 10.0;
/// Pressure must stay below `PSI_CLEAR_PERCENT` this long before a throttled or
/// paused command is let go. Miguel 11-23.
pub(super) const CLEAR_SUSTAIN: Duration = Duration::from_secs(10);
/// The command that grew most over this window is the one acted on. Miguel 11-23.
pub(super) const GROWTH_WINDOW: Duration = Duration::from_secs(10);
/// A command must have grown at least this much over the window to be acted
/// on at all: intervene as little as possible. Miguel 11-23.
pub(super) const GROWTH_MIN_BYTES: u64 = 64 * MIB;
/// Miguel 11-27 (approved via orch15): when pressure and a full pool hold but
/// nobody grew `GROWTH_MIN_BYTES` over the last `GROWTH_WINDOW` (the kernel's
/// own throttle can stop a runaway's growth before the pressure average has
/// risen), the window widens to this, so a machine in distress for minutes is
/// not left alone because its cause stopped growing meanwhile.
pub(super) const GROWTH_WINDOW_WIDE: Duration = Duration::from_secs(60);
/// Miguel 11-27 (approved via orch15): a command that was paused and let go,
/// and grows into exhaustion again, is stopped at once instead of being given
/// the same throttle and pause again (a runaway would otherwise cycle for ever
/// as each pause relieves the pressure).
pub(super) const STOP_A_RE_GROWER_AT_ONCE: bool = true;
/// Throttle at the trigger, pause this long after it. Miguel 11-23.
pub(super) const PAUSE_AFTER: Duration = Duration::from_secs(3);
/// Stop this long after the trigger, when pressure still holds. Miguel 11-23.
pub(super) const STOP_AFTER: Duration = Duration::from_secs(13);
/// What the OS keeps for itself out of the machine. Miguel 11-23.
pub(super) const OS_RESERVE_BYTES: u64 = 512 * MIB;
/// What the kernel's own cgroup protects (`memory.min`): actual use only, so a
/// small kernel on a small machine costs what it uses. Miguel 11-23.
pub(crate) const KERNEL_RESERVATION_BYTES: u64 = 2048 * MIB;
/// The same for the executor kernel, which measured 3.6 GB at its peak.
/// Miguel 11-23.
pub(crate) const EXECUTOR_KERNEL_RESERVATION_BYTES: u64 = 4096 * MIB;
/// Samples of a command's memory are kept this far apart.
const SAMPLE_EVERY: Duration = Duration::from_secs(1);

/// What the machine reports right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Gauge {
    /// Memory pressure, "some" over 10 s, in percent.
    pub psi: f64,
    /// The pool: the machine less the kernel's use and the OS reserve.
    pub pool_bytes: u64,
    /// What the agent workloads (everything but the kernel) hold of it.
    pub pool_used: u64,
}

impl Gauge {
    pub(super) fn pool_percent(&self) -> u64 {
        if self.pool_bytes == 0 {
            return 100;
        }
        (self.pool_used as u128 * 100 / self.pool_bytes as u128) as u64
    }

    /// Pressure high and the pool nearly full.
    fn hot(&self) -> bool {
        self.psi >= PSI_TRIGGER_PERCENT && self.pool_percent() >= POOL_TRIGGER_PERCENT
    }

    /// Not over yet: the steps go on.
    fn pressing(&self) -> bool {
        self.psi >= PSI_CLEAR_PERCENT && self.pool_percent() >= POOL_TRIGGER_PERCENT
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Throttle,
    Pause,
    Stop,
    /// Pressure ended (or the command ended): undo throttle and pause.
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Action {
    pub pid: u32,
    pub step: Step,
    /// What it grew over the window (which one), and what it holds now.
    pub grew: u64,
    pub window: Duration,
    pub resident: u64,
    pub gauge: Gauge,
    /// When the steps began: the stop reports how long they took.
    pub since: Duration,
    /// It was paused before and grew again: no second throttle.
    pub repeat: bool,
}

struct Active {
    pid: u32,
    started: Instant,
    last: Step,
    grew: u64,
    window: Duration,
}

#[derive(Default)]
pub(super) struct Exhaustion {
    hot_since: Option<Instant>,
    calm_since: Option<Instant>,
    active: Option<Active>,
    history: BTreeMap<u32, VecDeque<(Instant, u64)>>,
    sampled: Option<Instant>,
    /// Commands that were paused and let go: when they grow into exhaustion
    /// again they are stopped, not given the same steps again.
    paused_before: BTreeMap<u32, ()>,
}

impl Exhaustion {
    /// One look at the machine. `commands` are the agent commands that can be
    /// acted on (pid, resident bytes).
    pub(super) fn tick(
        &mut self,
        now: Instant,
        gauge: Gauge,
        commands: &[(u32, u64)],
    ) -> Vec<Action> {
        self.record(now, commands);
        // Over when the pressure is below the clear line, or the pool no longer
        // nearly full (the trigger's other half), for the clear time.
        if gauge.pressing() {
            self.calm_since = None;
        } else {
            self.calm_since.get_or_insert(now);
        }
        let resident = |pid: u32| {
            commands
                .iter()
                .find(|(candidate, _)| *candidate == pid)
                .map(|(_, resident)| *resident)
        };
        let mut actions = Vec::new();
        if let Some(active) = &mut self.active {
            let pid = active.pid;
            let since = now.saturating_duration_since(active.started);
            let action = |step, active: &Active| Action {
                pid,
                step,
                grew: active.grew,
                window: active.window,
                resident: resident(pid).unwrap_or(0),
                gauge,
                since,
                repeat: false,
            };
            let calm = self
                .calm_since
                .is_some_and(|at| now.saturating_duration_since(at) >= CLEAR_SUSTAIN);
            if resident(pid).is_none() || calm {
                if active.last == Step::Pause {
                    self.paused_before.insert(pid, ());
                }
                actions.push(action(Step::Release, active));
                self.active = None;
            } else if gauge.pressing() {
                if active.last == Step::Pause && since >= STOP_AFTER {
                    actions.push(action(Step::Stop, active));
                    self.active = None;
                } else if active.last == Step::Throttle && since >= PAUSE_AFTER {
                    active.last = Step::Pause;
                    actions.push(action(Step::Pause, active));
                }
            }
            return actions;
        }
        if !gauge.hot() {
            self.hot_since = None;
            return actions;
        }
        let hot = *self.hot_since.get_or_insert(now);
        if now.saturating_duration_since(hot) < PSI_SUSTAIN {
            return actions;
        }
        let Some((pid, grew, window)) = self.top_grower(now, commands) else {
            return actions;
        };
        self.hot_since = None;
        let repeat = STOP_A_RE_GROWER_AT_ONCE && self.paused_before.contains_key(&pid);
        let step = if repeat { Step::Stop } else { Step::Throttle };
        actions.push(Action {
            pid,
            step,
            grew,
            window,
            resident: resident(pid).unwrap_or(0),
            gauge,
            since: Duration::ZERO,
            repeat,
        });
        if !repeat {
            self.active = Some(Active {
                pid,
                started: now,
                last: Step::Throttle,
                grew,
                window,
            });
        }
        actions
    }

    /// Protection is off or the machine unreadable: the command being acted on
    /// is let go. Returns its pid, if there was one.
    pub(super) fn release_all(&mut self) -> Vec<u32> {
        self.hot_since = None;
        self.calm_since = None;
        self.active
            .take()
            .map(|active| active.pid)
            .into_iter()
            .collect()
    }

    fn record(&mut self, now: Instant, commands: &[(u32, u64)]) {
        if self
            .sampled
            .is_some_and(|at| now.saturating_duration_since(at) < SAMPLE_EVERY)
        {
            return;
        }
        self.sampled = Some(now);
        self.history
            .retain(|pid, _| commands.iter().any(|(candidate, _)| candidate == pid));
        self.paused_before
            .retain(|pid, _| commands.iter().any(|(candidate, _)| candidate == pid));
        for (pid, resident) in commands {
            let points = self.history.entry(*pid).or_default();
            points.push_back((now, *resident));
            while points.len() > 1
                && points
                    .get(1)
                    .is_some_and(|(at, _)| now.saturating_duration_since(*at) >= GROWTH_WINDOW_WIDE)
            {
                points.pop_front();
            }
        }
    }

    /// What `pid` grew over `window` ending now: from the newest sample at
    /// least that old (the oldest one when the command is younger).
    fn grew_over(&self, now: Instant, pid: u32, resident: u64, window: Duration) -> Option<u64> {
        let points = self.history.get(&pid)?;
        let base = points
            .iter()
            .rev()
            .find(|(at, _)| now.saturating_duration_since(*at) >= window)
            .or_else(|| points.front())?;
        Some(resident.saturating_sub(base.1))
    }

    /// What each command grew over the window, largest first (for the log).
    pub(super) fn growths(&self, now: Instant, commands: &[(u32, u64)]) -> Vec<(u32, u64, u64)> {
        let mut rows = commands
            .iter()
            .map(|(pid, resident)| {
                (
                    *pid,
                    *resident,
                    self.grew_over(now, *pid, *resident, GROWTH_WINDOW)
                        .unwrap_or(0),
                )
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|(_, resident, grew)| std::cmp::Reverse((*grew, *resident)));
        rows.truncate(4);
        rows
    }

    /// The command with the largest growth over the window, at least the
    /// minimum; over the wide window when none reached it in the narrow one.
    fn top_grower(&self, now: Instant, commands: &[(u32, u64)]) -> Option<(u32, u64, Duration)> {
        [GROWTH_WINDOW, GROWTH_WINDOW_WIDE]
            .into_iter()
            .find_map(|window| {
                commands
                    .iter()
                    .filter_map(|(pid, resident)| {
                        Some((*pid, self.grew_over(now, *pid, *resident, window)?))
                    })
                    .filter(|(_, grew)| *grew >= GROWTH_MIN_BYTES)
                    .max_by_key(|(_, grew)| *grew)
                    .map(|(pid, grew)| (pid, grew, window))
            })
    }
}

/// "some avg10" of a `memory.pressure` file or `/proc/pressure/memory`.
pub(super) fn psi_some_avg10(text: &str) -> Option<f64> {
    text.lines()
        .find_map(|line| line.strip_prefix("some "))?
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))?
        .parse()
        .ok()
}

/// The pool: the machine less what the kernel itself holds and the OS reserve.
pub(super) fn pool_bytes(machine: u64, kernel_use: u64) -> u64 {
    machine
        .saturating_sub(kernel_use)
        .saturating_sub(OS_RESERVE_BYTES)
}

/// What the kernel itself holds: its resident memory.
#[cfg(target_os = "linux")]
pub(super) fn kernel_use() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|text| text.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or(0, |pages| pages.saturating_mul(4096))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn kernel_use() -> u64 {
    0
}

/// The kernel's reservation: the executor's when `CHARIOX_KERNEL_MEMORY_RESERVATION_MIB`
/// names one, else the default.
pub(crate) fn kernel_reservation() -> u64 {
    std::env::var("CHARIOX_KERNEL_MEMORY_RESERVATION_MIB")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .and_then(|mib| mib.checked_mul(MIB))
        .unwrap_or(KERNEL_RESERVATION_BYTES)
}

/// The command that sets the reservation on a systemd service, for the line the
/// kernel logs where it is missing. The executor kernel's is the larger one.
pub(crate) fn protection_hint() -> String {
    format!(
        "systemctl set-property <service> MemoryMin={}M (default {}M; the executor kernel {}M, with CHARIOX_KERNEL_MEMORY_RESERVATION_MIB={} in its environment)",
        kernel_reservation() / MIB,
        KERNEL_RESERVATION_BYTES / MIB,
        EXECUTOR_KERNEL_RESERVATION_BYTES / MIB,
        EXECUTOR_KERNEL_RESERVATION_BYTES / MIB,
    )
}

/// What the kernel's own program has held at most: `VmHWM`.
#[cfg(target_os = "linux")]
fn kernel_peak() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("VmHWM:"))?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
        .map_or(0, |kib| kib.saturating_mul(1024))
}

/// What the kernel's cgroup should protect: the reservation, or a quarter more
/// than the kernel has measurably needed (rounded up to 256 MiB) when that is
/// more: a busier kernel protects itself without a hand-set number.
#[cfg(target_os = "linux")]
pub(crate) fn wanted_protection() -> u64 {
    let measured = kernel_peak().saturating_add(kernel_peak() / 4);
    let rounded = measured.div_ceil(256 * MIB).saturating_mul(256 * MIB);
    kernel_reservation().max(rounded)
}

/// The systemd service or scope the kernel runs in, and whether it is a user
/// manager's, from its own cgroup.
#[cfg(target_os = "linux")]
fn systemd_unit() -> Option<(String, bool)> {
    let own = std::fs::read_to_string("/proc/self/cgroup")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("0::").map(str::to_string))?;
    // The innermost unit owns the cgroup: a user service sits below the user
    // manager's own `user@UID.service`.
    let unit = own
        .split('/')
        .rfind(|part| part.ends_with(".service") || part.ends_with(".scope"))?
        .to_string();
    Some((unit, own.contains("/user@")))
}

/// Raises what protects the kernel to `target`: its service manager carries
/// `MemoryMin` up through the slices above (a plain write would not), and the
/// kernel's own cgroup (the `supervisor` subgroup of a delegated unit, which the
/// manager does not set) is written by the kernel itself. Each part is tried;
/// the caller reads the result back and says what holds.
#[cfg(target_os = "linux")]
pub(crate) fn raise_protection(target: u64) -> Result<(), String> {
    use crate::runtime::bounded_process::BoundedCommandExt as _;
    let mut failures = Vec::new();
    match systemd_unit() {
        Some((unit, user)) => {
            let mut command = crate::process_spawn::command("systemctl");
            if user {
                command.arg("--user");
            }
            match command
                .args([
                    "set-property",
                    "--runtime",
                    &unit,
                    &format!("MemoryMin={target}"),
                ])
                .bounded_output()
            {
                Ok(output) if output.status.success() => {}
                Ok(output) => {
                    failures.push(String::from_utf8_lossy(&output.stderr).trim().to_string())
                }
                Err(error) => failures.push(error.to_string()),
            }
        }
        None => failures.push("not running under systemd".to_string()),
    }
    // The kernel's own cgroup, and every level above it that is short and that
    // this user may write (a root kernel's slices; a user manager's levels are
    // the installer's). Only memory actually in use is protected.
    if let Some(own) = own_cgroup() {
        let root = Path::new("/sys/fs/cgroup");
        for level in own
            .ancestors()
            .take_while(|path| *path != root && path.starts_with(root))
        {
            let current = std::fs::read_to_string(level.join("memory.min"))
                .ok()
                .and_then(|text| text.trim().parse::<u64>().ok());
            if current.is_some_and(|current| current >= target) {
                continue;
            }
            if let Err(error) = std::fs::write(level.join("memory.min"), target.to_string()) {
                failures.push(format!("{}: {error}", level.display()));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn wanted_protection() -> u64 {
    kernel_reservation()
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn raise_protection(_target: u64) -> Result<(), String> {
    Err("not supported on this host".into())
}

/// What protects the kernel's own cgroup from reclaim: the smallest
/// `memory.min` from the kernel's cgroup up to (not including) the root.
/// Protection holds only as far as every level above carries it, and a
/// delegated unit's `supervisor` subgroup is the kernel alone, its sibling
/// budgets being the agents: the unit's own value would protect them as much.
/// A level without the file protects nothing.
fn protection_in(root: &Path, own: &Path) -> Option<u64> {
    own.ancestors()
        .take_while(|path| *path != root && path.starts_with(root))
        .map(|path| {
            std::fs::read_to_string(path.join("memory.min"))
                .ok()
                .and_then(|text| text.trim().parse::<u64>().ok())
                .unwrap_or(0)
        })
        .min()
}

#[cfg(target_os = "linux")]
fn own_cgroup() -> Option<PathBuf> {
    let own = std::fs::read_to_string("/proc/self/cgroup")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("0::").map(str::to_string))?;
    Some(Path::new("/sys/fs/cgroup").join(own.trim_start_matches('/')))
}

/// What the kernel's own cgroup protects now, against what it needs: `None`
/// where the host has no cgroup memory protection to read.
#[cfg(target_os = "linux")]
pub(crate) fn kernel_protection() -> Option<(u64, u64)> {
    let protected = protection_in(Path::new("/sys/fs/cgroup"), &own_cgroup()?)?;
    Some((protected, wanted_protection()))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn kernel_protection() -> Option<(u64, u64)> {
    None
}

pub(super) fn gb(bytes: u64) -> String {
    super::waits::size(bytes)
}

fn command_name(line: &str) -> String {
    let line = line
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .replace('`', "'");
    if line.chars().count() <= 100 {
        return line;
    }
    format!("{}…", line.chars().take(100).collect::<String>())
}

/// What each step says to the command's own error stream, its agent's inbox
/// and the pane. The first words name the machine's condition and the numbers.
pub(super) fn throttled_line(command: &str, action: &Action) -> String {
    format!(
        "chariox: your process `{}` was throttled: the machine is out of memory (memory pressure {:.0}%, {}% of the memory pool in use) and this process grew {} in {} s, the most of anything running (now {}). Its memory growth is slowed. If the pressure goes on it is paused in {} s and stopped in {} s. Use less memory (lower parallelism such as -j, cap the test's memory, split the work).\n",
        command_name(command),
        action.gauge.psi,
        action.gauge.pool_percent(),
        gb(action.grew),
        action.window.as_secs(),
        gb(action.resident),
        PAUSE_AFTER.as_secs(),
        STOP_AFTER.as_secs(),
    )
}

pub(super) fn paused_line(command: &str, action: &Action) -> String {
    format!(
        "chariox: your process `{}` was paused: the machine is still out of memory (memory pressure {:.0}%, {}% of the memory pool in use) and this process grew {} in {} s, the most of anything running (now {}). It is stopped in {} s if the pressure does not end, and it continues by itself when it ends.\n",
        command_name(command),
        action.gauge.psi,
        action.gauge.pool_percent(),
        gb(action.grew),
        action.window.as_secs(),
        gb(action.resident),
        STOP_AFTER.saturating_sub(action.since).as_secs(),
    )
}

pub(super) fn resumed_line(command: &str) -> String {
    format!(
        "chariox: your process `{}` continues: the machine's memory pressure ended.\n",
        command_name(command)
    )
}

pub(super) fn stopped_line(command: &str, action: &Action) -> String {
    let how = if action.repeat {
        format!(
            "it was paused before for the same reason and grew {} in {} s again",
            gb(action.grew),
            action.window.as_secs()
        )
    } else {
        format!(
            "it grew {} in {} s, the most of anything running, and the pressure held for {} s while it was throttled and paused",
            gb(action.grew),
            action.window.as_secs(),
            action.since.as_secs()
        )
    };
    format!(
        "chariox: your process `{}` was stopped: the machine ran out of memory (memory pressure {:.0}%, {}% of the memory pool in use); {how}; it held {}. Run it again with less memory (lower parallelism such as -j, cap the test's memory, split the work).\n",
        command_name(command),
        action.gauge.psi,
        action.gauge.pool_percent(),
        gb(action.resident),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gauge(psi: f64, used_percent: u64) -> Gauge {
        Gauge {
            psi,
            pool_bytes: 100 * 1024 * MIB,
            pool_used: used_percent * 1024 * MIB,
        }
    }

    fn secs(start: Instant, n: u64) -> Instant {
        start + Duration::from_secs(n)
    }

    /// One command that grows by `per_second` bytes each second.
    fn run(
        machine: &mut Exhaustion,
        start: Instant,
        seconds: std::ops::Range<u64>,
        gauge_at: impl Fn(u64) -> Gauge,
        per_second: u64,
    ) -> Vec<(u64, Action)> {
        let mut seen = Vec::new();
        for t in seconds {
            for action in machine.tick(
                secs(start, t),
                gauge_at(t),
                &[(7, 100 * MIB + t * per_second)],
            ) {
                seen.push((t, action));
            }
        }
        seen
    }

    #[test]
    fn psi_is_read_from_a_pressure_file() {
        let text = "some avg10=12.50 avg60=3.00 avg300=1.00 total=10\nfull avg10=1.00 avg60=0.00 avg300=0.00 total=1\n";
        assert_eq!(psi_some_avg10(text), Some(12.5));
        assert_eq!(psi_some_avg10("garbage"), None);
    }

    #[test]
    fn the_pool_is_the_machine_less_the_kernel_and_the_os_reserve() {
        assert_eq!(
            pool_bytes(8 * 1024 * MIB, 300 * MIB),
            8 * 1024 * MIB - 300 * MIB - 512 * MIB
        );
        assert_eq!(pool_bytes(MIB, 4 * MIB), 0);
    }

    #[test]
    fn a_runaway_is_throttled_then_paused_then_stopped_on_time() {
        let start = Instant::now();
        let mut machine = Exhaustion::default();
        let steps = run(&mut machine, start, 0..40, |_| gauge(45.0, 90), 200 * MIB);
        let at = |step| steps.iter().find(|(_, a)| a.step == step).map(|(t, _)| *t);
        let throttled = at(Step::Throttle).expect("throttled");
        assert_eq!(throttled, 5, "pressure and a full pool hold for 5 s first");
        assert_eq!(at(Step::Pause), Some(throttled + 3));
        assert_eq!(at(Step::Stop), Some(throttled + 13));
        let stop = steps.iter().find(|(_, a)| a.step == Step::Stop).unwrap().1;
        assert!(stop.grew >= GROWTH_MIN_BYTES && stop.pid == 7);
    }

    #[test]
    fn pressure_without_a_full_pool_or_a_full_pool_without_pressure_does_nothing() {
        let start = Instant::now();
        let mut machine = Exhaustion::default();
        assert!(run(&mut machine, start, 0..30, |_| gauge(60.0, 40), 200 * MIB).is_empty());
        assert!(run(&mut machine, start, 30..60, |_| gauge(2.0, 95), 200 * MIB).is_empty());
    }

    #[test]
    fn pressure_must_hold_for_the_whole_sustain_time() {
        let start = Instant::now();
        let mut machine = Exhaustion::default();
        let flicker = |t: u64| {
            if t % 4 == 3 {
                gauge(5.0, 90)
            } else {
                gauge(50.0, 90)
            }
        };
        assert!(run(&mut machine, start, 0..40, flicker, 200 * MIB).is_empty());
    }

    #[test]
    fn a_command_that_did_not_grow_enough_is_left_alone() {
        let start = Instant::now();
        let mut machine = Exhaustion::default();
        let steps = run(&mut machine, start, 0..40, |_| gauge(50.0, 90), MIB);
        assert!(steps.is_empty(), "{steps:?}");
    }

    #[test]
    fn the_command_that_grows_most_is_chosen_not_the_oldest_or_largest() {
        let start = Instant::now();
        let mut machine = Exhaustion::default();
        let mut first = None;
        for t in 0..10u64 {
            let commands = [
                (1, 50_000 * MIB),
                (2, 100 * MIB + t * 300 * MIB),
                (3, 100 * MIB + t * 100 * MIB),
            ];
            for action in machine.tick(secs(start, t), gauge(40.0, 85), &commands) {
                first.get_or_insert(action);
            }
        }
        assert_eq!(first.map(|a| a.pid), Some(2));
    }

    #[test]
    fn pressure_that_ends_releases_the_command_after_the_clear_time() {
        let start = Instant::now();
        let mut machine = Exhaustion::default();
        let gauge_at = |t: u64| {
            if t < 8 {
                gauge(50.0, 90)
            } else {
                gauge(3.0, 60)
            }
        };
        let steps = run(&mut machine, start, 0..40, gauge_at, 200 * MIB);
        let release = steps
            .iter()
            .find(|(_, a)| a.step == Step::Release)
            .expect("released");
        assert_eq!(release.0, 8 + 10);
        assert!(steps.iter().all(|(_, a)| a.step != Step::Stop));
    }

    #[test]
    fn a_command_paused_before_that_grows_into_exhaustion_again_is_stopped_at_once() {
        let start = Instant::now();
        let mut machine = Exhaustion::default();
        let gauge_at = |t: u64| {
            if (5..15).contains(&t) || t >= 30 {
                gauge(50.0, 90)
            } else {
                gauge(3.0, 60)
            }
        };
        let steps = run(&mut machine, start, 0..50, gauge_at, 200 * MIB);
        let kinds = steps.iter().map(|(_, a)| a.step).collect::<Vec<_>>();
        assert_eq!(kinds.first(), Some(&Step::Throttle), "{kinds:?}");
        assert!(kinds.contains(&Step::Pause) && kinds.contains(&Step::Release));
        let stop = steps
            .iter()
            .find(|(_, a)| a.step == Step::Stop)
            .expect("stop");
        assert!(stop.1.repeat);
    }

    #[test]
    fn a_target_that_ends_by_itself_is_released() {
        let start = Instant::now();
        let mut machine = Exhaustion::default();
        let mut released = false;
        for t in 0..12u64 {
            let commands: Vec<(u32, u64)> = if t < 7 {
                vec![(7, 100 * MIB + t * 200 * MIB)]
            } else {
                Vec::new()
            };
            released |= machine
                .tick(secs(start, t), gauge(50.0, 90), &commands)
                .iter()
                .any(|a| a.step == Step::Release);
        }
        assert!(released);
    }

    #[test]
    fn a_runaway_the_kernel_throttled_before_pressure_rose_is_found_over_the_wide_window() {
        let start = Instant::now();
        let mut machine = Exhaustion::default();
        let mut first = None;
        for t in 0..40u64 {
            // It grew for 15 s, then stood still (the host throttled it); pressure only rises at t = 20.
            let resident = 100 * MIB + t.min(15) * 300 * MIB;
            let gauge = if t >= 20 {
                gauge(60.0, 90)
            } else {
                gauge(2.0, 90)
            };
            for action in machine.tick(secs(start, t), gauge, &[(7, resident)]) {
                first.get_or_insert((t, action));
            }
        }
        let (t, action) =
            first.expect("acted although it had stopped growing in the narrow window");
        assert_eq!((t, action.pid, action.step), (25, 7, Step::Throttle));
        assert_eq!(action.window, GROWTH_WINDOW_WIDE);
        assert!(
            stopped_line("x", &action).contains("60 s")
                || throttled_line("x", &action).contains("60 s")
        );
    }

    fn cgroup_tree(levels: &[(&str, u64)]) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("cg-{:016x}", rand::random::<u64>()));
        let mut dir = root.clone();
        for (name, min) in levels {
            dir = dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("memory.min"), min.to_string()).unwrap();
        }
        (root, dir)
    }

    // The shipped unit: the service carries the minimum, the kernel sits in its
    // `supervisor` subgroup and the agents' budgets are siblings of it.
    #[test]
    fn a_service_minimum_alone_does_not_protect_the_kernels_supervisor_subgroup() {
        let two = 2048 * MIB;
        let (root, own) = cgroup_tree(&[
            ("system.slice", two),
            ("kernel.service", two),
            ("supervisor", 0),
        ]);
        std::fs::create_dir_all(root.join("system.slice/kernel.service/chariox-1-2")).unwrap();
        assert_eq!(
            protection_in(&root, &own),
            Some(0),
            "an agents' sibling shares the service minimum, the kernel is not named"
        );
        std::fs::write(own.join("memory.min"), two.to_string()).unwrap();
        assert_eq!(protection_in(&root, &own), Some(two));
        // A level above that carries nothing ends the protection there.
        std::fs::write(root.join("system.slice/memory.min"), "0").unwrap();
        assert_eq!(protection_in(&root, &own), Some(0));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_level_protects_nothing_and_a_plain_service_is_judged_alone() {
        let (root, own) = cgroup_tree(&[
            ("system.slice", 4096 * MIB),
            ("executor.service", 4096 * MIB),
        ]);
        assert_eq!(protection_in(&root, &own), Some(4096 * MIB));
        std::fs::remove_file(root.join("system.slice/memory.min")).unwrap();
        assert_eq!(protection_in(&root, &own), Some(0));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_innermost_service_owns_a_user_managers_cgroup_path() {
        let path = "/user.slice/user-1000.slice/user@1000.service/app.slice/chariox-kernel.service/supervisor";
        let unit = path
            .split('/')
            .rfind(|part| part.ends_with(".service") || part.ends_with(".scope"));
        assert_eq!(unit, Some("chariox-kernel.service"));
    }

    #[test]
    fn pressure_in_between_with_a_pool_no_longer_nearly_full_still_releases() {
        let start = Instant::now();
        let mut machine = Exhaustion::default();
        // Hot until t = 8, then PSI stays at 15 % (above the clear line) while the pool falls to 50 %.
        let gauge_at = |t: u64| {
            if t < 8 {
                gauge(50.0, 90)
            } else {
                gauge(15.0, 50)
            }
        };
        let steps = run(&mut machine, start, 0..40, gauge_at, 200 * MIB);
        let release = steps
            .iter()
            .find(|(_, a)| a.step == Step::Release)
            .expect("released");
        assert_eq!(release.0, 8 + 10);
    }

    #[test]
    fn each_step_says_what_happened_with_the_numbers_and_what_to_do() {
        let action = Action {
            pid: 7,
            step: Step::Throttle,
            grew: 3 * 1024 * MIB,
            window: GROWTH_WINDOW,
            resident: 9 * 1024 * MIB,
            gauge: gauge(45.0, 91),
            since: Duration::from_secs(13),
            repeat: false,
        };
        let throttled = throttled_line("node usage.test.js", &action);
        for part in [
            "throttled",
            "out of memory",
            "3 GB",
            "10 s",
            "9 GB",
            "91%",
            "-j",
        ] {
            assert!(throttled.contains(part), "{part}: {throttled}");
        }
        assert!(paused_line("cargo build", &action).contains("paused"));
        assert!(resumed_line("cargo build").contains("continues"));
        let stopped = stopped_line("cargo build", &action);
        for part in ["stopped", "3 GB", "9 GB", "13 s", "Run it again"] {
            assert!(stopped.contains(part), "{part}: {stopped}");
        }
    }
}
