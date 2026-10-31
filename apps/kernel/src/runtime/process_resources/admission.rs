//! Admission (OF-01..OF-05): a heavy command waits for room instead of growing
//! into a full machine. A command whose last run peaked above the light bar is
//! held before it grows when the pool has no room for that peak; an unknown
//! command growing into a tight pool is paused before anything is stopped.
//! Waiting commands start by themselves, in turn across owners, once their
//! room is free. Light commands never wait.
use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Commands that stay under this never wait (OF-03); small pools scale it.
const MAX_LIGHT: u64 = 512 * MIB;
/// Room a paused command of unknown peak needs before it continues.
const MAX_GROWTH_ROOM: u64 = 1024 * MIB;
/// Disk floor: max(5 %, 10 GiB) of the volume (OF-05).
const DISK_FLOOR: u64 = 10 * 1024 * MIB;
/// Slower than this a command is not growing (or writing).
const GROWING: u64 = MIB;
/// A start needs a moment to show in the samples.
const SETTLE: Duration = Duration::from_secs(1);
/// A paused command continues with room for this long at its last rate.
const AHEAD: u64 = 2;
const REMEMBERED: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    Memory,
    Disk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Wait {
    /// Stopped before it grew: it waits for room for its expected peak.
    Held(WaitReason),
    /// Stopped while growing into a tight pool or a full disk.
    Paused(WaitReason),
}
impl Wait {
    fn reason(self) -> WaitReason {
        match self {
            Wait::Held(reason) | Wait::Paused(reason) => reason,
        }
    }
}

/// The largest memory and the bytes written by one run of a command.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct Peak {
    pub memory: u64,
    pub writes: u64,
}

/// What the kernel recognises a command by: its normalized argv and cwd,
/// hashed. The line is what clients show.
#[derive(Clone)]
pub(super) struct Seen {
    pid: u32,
    key: String,
    pub line: String,
    cwd: Option<PathBuf>,
}

/// Admission's view of one command (the top of a subtree under the root).
#[derive(Default)]
pub(super) struct Entry {
    arrived: u64,
    pub wait: Option<Wait>,
    /// It started with room for its expected peak, or grew past the light
    /// bar unknown: from now on only a pause can stop its growth.
    admitted: bool,
    /// The user started it now: it never waits again.
    forced: bool,
    /// It may wait for room: set when it starts, by who started it, and
    /// never changed after (a person's own terminal command never waits).
    holdable: bool,
    /// The watch has stopped its subtree (`SIGSTOP`).
    pub stopped: bool,
    pub seen: Option<Seen>,
    expected: Option<Peak>,
    /// Bytes per second it grew (or wrote) when it was paused.
    paused_rate: u64,
    peak: Peak,
    resident: u64,
    writes: u64,
    pub position: u32,
    pub need: u64,
    pub free: u64,
}
impl Entry {
    /// The volume its writes go to, from the directory it runs in.
    fn disk(&self, room: &Room) -> Option<Disk> {
        self.seen
            .as_ref()
            .and_then(|seen| disk_room(room.volume, seen.cwd.as_deref()?))
    }

    /// Writes it still has ahead toward its expected peak; only a heavy
    /// writer counts.
    fn writes_ahead(&self, room: &Room) -> u64 {
        self.expected
            .filter(|peak| peak.writes >= room.light)
            .map_or(0, |peak| peak.writes.saturating_sub(self.writes))
    }

    /// Memory it still has ahead toward its expected peak; only a heavy
    /// command counts.
    fn memory_ahead(&self, room: &Room) -> u64 {
        self.expected
            .filter(|peak| peak.memory >= room.light)
            .map_or(0, |peak| peak.memory.saturating_sub(self.resident))
    }

    /// Start now: the user admits it whatever the room.
    pub(super) fn force(&mut self) -> bool {
        if self.wait.take().is_none() {
            return false;
        }
        self.forced = true;
        self.admitted = true;
        true
    }
}

/// The machine-wide part of admission, under the pool.
pub(super) struct Ledger {
    arrivals: u64,
    resumed_at: BTreeMap<u8, Instant>,
    peaks: Peaks,
}

/// The volume holding a path: which one it is, its free bytes and its size.
#[derive(Clone, Copy, Debug)]
pub(super) struct Space {
    pub id: u64,
    pub free: u64,
    pub total: u64,
}
pub(super) type Volume = dyn Fn(&Path) -> Option<Space> + Send + Sync;

/// What one pass needs from the pool.
pub(super) struct Room<'a> {
    /// Memory left under the pressure line.
    pub memory: u64,
    /// Memory is short: pause the newest growing command.
    pub tight: bool,
    pub light: u64,
    pub growth_room: u64,
    pub volume: &'a Volume,
}

impl Ledger {
    pub(super) fn new(file: Option<PathBuf>) -> Self {
        Self {
            arrivals: 0,
            resumed_at: BTreeMap::new(),
            peaks: Peaks::load(file),
        }
    }

    /// Records the current workload's commands: new ones arrive, ended ones
    /// leave their peak for the next run of the same command.
    pub(super) fn register(&mut self, current: &Arc<Mutex<Observation>>, light: u64) {
        let mut observation = current.lock().unwrap_or_else(|e| e.into_inner());
        let observation = &mut *observation;
        let submissions = &observation.submissions;
        let live = observation
            .commands
            .iter()
            .filter(|command| !command.root)
            .map(|command| command.pid)
            .collect::<std::collections::BTreeSet<_>>();
        let ended = observation
            .admission
            .keys()
            .filter(|pid| !live.contains(pid))
            .copied()
            .collect::<Vec<_>>();
        for pid in ended {
            if let Some(Entry {
                seen: Some(seen),
                peak,
                ..
            }) = observation.admission.remove(&pid)
            {
                self.peaks.record(&seen.key, peak, light);
            }
        }
        for command in observation.commands.iter().filter(|command| !command.root) {
            let entry = observation.admission.entry(command.pid).or_insert_with(|| {
                self.arrivals += 1;
                Entry {
                    arrived: self.arrivals,
                    holdable: submissions.owner_of(command.started),
                    ..Entry::default()
                }
            });
            entry.resident = command.resident;
            entry.writes = command.writes;
            entry.peak.memory = entry.peak.memory.max(command.resident);
            entry.peak.writes = entry.peak.writes.max(command.writes);
            // Until it is decided, follow a shell to the command it runs.
            if !entry.admitted
                && entry.wait.is_none()
                && entry
                    .seen
                    .as_ref()
                    .is_none_or(|seen| seen.pid != command.effective)
            {
                entry.seen = recognise(command.effective);
                entry.expected = entry
                    .seen
                    .as_ref()
                    .and_then(|seen| self.peaks.get(&seen.key));
            }
        }
    }

    /// One admission pass after a sample of `current`. Returns whether the
    /// pool may stop a command (nothing could be paused instead).
    pub(super) fn pass(
        &mut self,
        members: &[Arc<Mutex<Observation>>],
        current: &Arc<Mutex<Observation>>,
        room: &Room,
    ) -> bool {
        let mut state = State::read(members, room);
        let mut available = room.memory.saturating_sub(state.reserved);
        let mut changed = Vec::new();
        self.hold(current, room, &mut state, &mut available);
        let mut may_stop = false;
        if room.tight {
            match self.pause(members, WaitReason::Memory, |entry, command| {
                (entry.resident >= room.light
                    && command.growth >= GROWING
                    && Some(entry.arrived) != state.oldest)
                    .then_some(command.growth)
            }) {
                Some(owner) => changed.push(owner),
                None => may_stop = true,
            }
        } else if let Some(owner) = self.resume(
            members,
            WaitReason::Memory,
            &mut state,
            room,
            &mut available,
        ) {
            changed.push(owner);
        }
        let full = |entry: &Entry| entry.disk(room).is_some_and(|disk| disk.free < disk.floor);
        if let Some(owner) = self.pause(members, WaitReason::Disk, |entry, command| {
            (command.write_growth >= GROWING && full(entry)).then_some(command.write_growth)
        }) {
            changed.push(owner);
        }
        if let Some(owner) =
            self.resume(members, WaitReason::Disk, &mut state, room, &mut available)
        {
            changed.push(owner);
        }
        let state = State::read(members, room);
        number(
            members,
            room,
            &state,
            room.memory.saturating_sub(state.reserved),
        );
        for owner in changed {
            if !Arc::ptr_eq(&owner, current) {
                let control = owner
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .control
                    .clone();
                if let Some(control) = control {
                    let _ = control.send(WatchCommand::Apply);
                }
            }
        }
        may_stop
    }

    /// New commands of `current` with a known heavy peak start only with room
    /// for it; otherwise they are held before they grow.
    fn hold(
        &mut self,
        current: &Arc<Mutex<Observation>>,
        room: &Room,
        state: &mut State,
        available: &mut u64,
    ) {
        let mut observation = current.lock().unwrap_or_else(|e| e.into_inner());
        for entry in observation.admission.values_mut() {
            if entry.admitted || entry.forced || entry.wait.is_some() {
                continue;
            }
            let Some(expected) = entry.expected else {
                // Unknown: light until it grows.
                entry.admitted = entry.resident >= room.light;
                continue;
            };
            let need = expected.memory.saturating_sub(entry.resident);
            let disk = entry.disk(room);
            let ahead = entry.writes_ahead(room);
            entry.wait = if !entry.holdable {
                None
            } else if expected.memory >= room.light && need > *available && state.heavy_running {
                Some(Wait::Held(WaitReason::Memory))
            } else if disk.is_some_and(|disk| ahead > state.spare(disk)) {
                Some(Wait::Held(WaitReason::Disk))
            } else {
                None
            };
            if entry.wait.is_none() {
                entry.admitted = true;
                *available = available.saturating_sub(need);
                if let Some(disk) = disk {
                    state.reserve(disk, ahead);
                }
                state.running(entry, room);
            }
        }
    }

    /// Pauses the newest running command `pausable` selects (it gives the
    /// command's rate).
    fn pause(
        &mut self,
        members: &[Arc<Mutex<Observation>>],
        reason: WaitReason,
        pausable: impl Fn(&Entry, &pool::CommandUse) -> Option<u64>,
    ) -> Option<Arc<Mutex<Observation>>> {
        let mut newest: Option<(u64, u32, u64, &Arc<Mutex<Observation>>)> = None;
        for member in members {
            let observation = member.lock().unwrap_or_else(|e| e.into_inner());
            for command in &observation.commands {
                let Some(entry) = observation.admission.get(&command.pid) else {
                    continue;
                };
                if !entry.holdable || entry.wait.is_some() || entry.forced {
                    continue;
                }
                if let Some(rate) = pausable(entry, command) {
                    if newest.is_none_or(|(arrived, ..)| entry.arrived > arrived) {
                        newest = Some((entry.arrived, command.pid, rate, member));
                    }
                }
            }
        }
        let (_, pid, rate, member) = newest?;
        if let Some(entry) = member
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admission
            .get_mut(&pid)
        {
            entry.wait = Some(Wait::Paused(reason));
            entry.paused_rate = rate;
        }
        crate::logging::info_with_fields(
            "process_resources.admission_pause",
            "paused a growing command instead of stopping one",
            serde_json::json!({"pid": pid, "reason": reason, "rate_bytes_per_second": rate}),
        );
        Some(member.clone())
    }

    /// Starts the first command in line for `reason` once its room is free,
    /// or when nothing heavy runs (waiting longer cannot make room).
    fn resume(
        &mut self,
        members: &[Arc<Mutex<Observation>>],
        reason: WaitReason,
        state: &mut State,
        room: &Room,
        available: &mut u64,
    ) -> Option<Arc<Mutex<Observation>>> {
        if self
            .resumed_at
            .get(&(reason as u8))
            .is_some_and(|at| at.elapsed() < SETTLE)
        {
            return None;
        }
        let (member, pid) = line(members, reason).into_iter().next()?;
        let mut observation = member.lock().unwrap_or_else(|e| e.into_inner());
        let entry = observation.admission.get_mut(&pid)?;
        // Both resources must have room now: what it waited for, and the
        // other one for the peak it is known to reach.
        let (memory, disk) = match reason {
            WaitReason::Memory => (need(entry, reason, room), entry.writes_ahead(room)),
            WaitReason::Disk => (entry.memory_ahead(room), need(entry, reason, room)),
        };
        let volume = entry.disk(room);
        let fits = (memory <= *available || !state.heavy_running)
            && volume.is_none_or(|volume| disk <= state.spare(volume));
        if !fits {
            return None;
        }
        *available = available.saturating_sub(
            entry
                .expected
                .map_or(0, |peak| peak.memory.saturating_sub(entry.resident)),
        );
        if let Some(volume) = volume {
            state.reserve(volume, entry.writes_ahead(room));
        }
        entry.wait = None;
        entry.admitted = true;
        state.running(entry, room);
        drop(observation);
        self.resumed_at.insert(reason as u8, Instant::now());
        Some(member)
    }
}

/// Machine-wide facts one pass decides on.
struct State {
    /// Growth admitted commands still have ahead toward their expected peak.
    reserved: u64,
    /// The oldest running heavy command: never paused, so work progresses.
    oldest: Option<u64>,
    /// Something heavy runs, so waiting can make room.
    heavy_running: bool,
    /// Writes admitted commands still have ahead, by volume.
    disk: BTreeMap<u64, u64>,
}
impl State {
    /// What a volume has above its floor, less what admitted commands will
    /// still write there.
    fn spare(&self, disk: Disk) -> u64 {
        disk.above_floor()
            .saturating_sub(self.disk.get(&disk.id).copied().unwrap_or(0))
    }

    fn reserve(&mut self, disk: Disk, bytes: u64) {
        if bytes > 0 {
            *self.disk.entry(disk.id).or_default() += bytes;
        }
    }

    /// Notes a command that runs now: the rest of the pass must see it as the
    /// heavy work it is, however it came to run.
    fn running(&mut self, entry: &Entry, room: &Room) {
        let heavy = entry.resident >= room.light
            || entry.admitted && entry.expected.is_some_and(|peak| peak.memory >= room.light);
        if heavy {
            self.heavy_running = true;
            self.oldest = Some(
                self.oldest
                    .map_or(entry.arrived, |oldest| oldest.min(entry.arrived)),
            );
        }
    }

    fn read(members: &[Arc<Mutex<Observation>>], room: &Room) -> Self {
        let mut state = State {
            reserved: 0,
            oldest: None,
            heavy_running: false,
            disk: BTreeMap::new(),
        };
        for member in members {
            let observation = member.lock().unwrap_or_else(|e| e.into_inner());
            for entry in observation.admission.values() {
                if entry.wait.is_some() {
                    continue;
                }
                if entry.admitted {
                    state.reserved += entry
                        .expected
                        .map_or(0, |peak| peak.memory.saturating_sub(entry.resident));
                    let ahead = entry.writes_ahead(room);
                    if let Some(disk) = entry.disk(room).filter(|_| ahead > 0) {
                        state.reserve(disk, ahead);
                    }
                }
                state.running(entry, room);
            }
        }
        state
    }
}

/// What a waiting command still needs to continue.
fn need(entry: &Entry, reason: WaitReason, room: &Room) -> u64 {
    let expected = entry.expected.map(|peak| match reason {
        WaitReason::Memory => peak.memory.saturating_sub(entry.resident),
        WaitReason::Disk => peak.writes.saturating_sub(entry.writes),
    });
    match entry.wait {
        Some(Wait::Held(_)) => expected.unwrap_or(room.growth_room),
        // It continues only with room to grow on for a while at its last
        // rate, so it is not paused again at once.
        _ => expected
            .filter(|need| *need > 0)
            .unwrap_or(room.growth_room)
            .max(entry.paused_rate.saturating_mul(AHEAD)),
    }
}

/// The waiting commands for `reason` in the order they start: by arrival,
/// one per owner in turn, so one owner's ten builds cannot starve the others.
fn line(
    members: &[Arc<Mutex<Observation>>],
    reason: WaitReason,
) -> Vec<(Arc<Mutex<Observation>>, u32)> {
    let mut owners = BTreeMap::<String, Vec<(u64, Arc<Mutex<Observation>>, u32)>>::new();
    for member in members {
        let observation = member.lock().unwrap_or_else(|e| e.into_inner());
        for (pid, entry) in &observation.admission {
            if entry.wait.is_some_and(|wait| wait.reason() == reason) {
                owners.entry(observation.unit.clone()).or_default().push((
                    entry.arrived,
                    member.clone(),
                    *pid,
                ));
            }
        }
    }
    let mut owners = owners.into_values().collect::<Vec<_>>();
    for waiting in &mut owners {
        waiting.sort_by_key(|(arrived, ..)| *arrived);
    }
    owners.sort_by_key(|waiting| waiting[0].0);
    let mut line = Vec::new();
    for turn in 0..owners.iter().map(Vec::len).max().unwrap_or(0) {
        for waiting in &owners {
            if let Some((_, member, pid)) = waiting.get(turn) {
                line.push((member.clone(), *pid));
            }
        }
    }
    line
}

/// Writes each waiting command's place in line, need and free room, for
/// clients.
fn number(members: &[Arc<Mutex<Observation>>], room: &Room, state: &State, available: u64) {
    for reason in [WaitReason::Memory, WaitReason::Disk] {
        for (position, (member, pid)) in line(members, reason).into_iter().enumerate() {
            let mut observation = member.lock().unwrap_or_else(|e| e.into_inner());
            let Some(entry) = observation.admission.get_mut(&pid) else {
                continue;
            };
            entry.position = position as u32 + 1;
            entry.need = match (reason, entry.expected) {
                (WaitReason::Memory, Some(peak)) => peak.memory,
                (WaitReason::Disk, Some(peak)) => peak.writes,
                _ => entry.resident + room.growth_room,
            };
            entry.free = match reason {
                WaitReason::Memory => available,
                WaitReason::Disk => entry.disk(room).map_or(0, |disk| state.spare(disk)),
            };
        }
    }
}

pub(super) fn light(ceiling: u64) -> u64 {
    MAX_LIGHT.min(ceiling / 16)
}

pub(super) fn growth_room(ceiling: u64) -> u64 {
    MAX_GROWTH_ROOM.min(ceiling / 8)
}

/// A volume's free bytes against its floor.
#[derive(Clone, Copy)]
struct Disk {
    id: u64,
    free: u64,
    floor: u64,
}
impl Disk {
    fn above_floor(self) -> u64 {
        self.free.saturating_sub(self.floor)
    }
}

/// The volume holding `path`, with its floor.
fn disk_room(volume: &Volume, path: &Path) -> Option<Disk> {
    let space = volume(path)?;
    Some(Disk {
        id: space.id,
        free: space.free,
        floor: (space.total / 20).max(DISK_FLOOR),
    })
}

#[cfg(unix)]
pub(super) fn statvfs(path: &Path) -> Option<Space> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let id = std::fs::metadata(path).ok()?.dev();
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: the path is NUL-terminated and the buffer is one statvfs.
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statvfs succeeded and filled the buffer.
    let stat = unsafe { stat.assume_init() };
    // The field widths differ by platform (u64 on Linux, u32 on macOS).
    #[allow(clippy::unnecessary_cast)]
    let (block, available, blocks) = (
        stat.f_frsize as u64,
        stat.f_bavail as u64,
        stat.f_blocks as u64,
    );
    Some(Space {
        id,
        free: available.saturating_mul(block),
        total: blocks.saturating_mul(block),
    })
}

#[cfg(not(unix))]
pub(super) fn statvfs(_path: &Path) -> Option<Space> {
    None
}

/// Reads a process's argv and cwd. The key ignores numbers and hex runs
/// (temporary names, ports, ids) so reruns of one command share it.
fn recognise(pid: u32) -> Option<Seen> {
    let (argv, cwd) = arguments(pid)?;
    if argv.is_empty() {
        return None;
    }
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    for argument in &argv {
        hash.update(normalize(argument).as_bytes());
        hash.update([0]);
    }
    if let Some(cwd) = &cwd {
        hash.update(cwd.to_string_lossy().as_bytes());
    }
    let key = hash
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(Seen {
        pid,
        key,
        line: display(&argv),
        cwd,
    })
}

fn normalize(argument: &str) -> String {
    let mut normalized = String::with_capacity(argument.len());
    let mut run = String::new();
    for character in argument.chars().chain(std::iter::once(' ')) {
        if character.is_ascii_hexdigit() {
            run.push(character);
            continue;
        }
        if run.len() >= 6 || (!run.is_empty() && run.chars().all(|c| c.is_ascii_digit())) {
            normalized.push('#');
        } else {
            normalized.push_str(&run);
        }
        run.clear();
        normalized.push(character);
    }
    normalized.pop();
    normalized
}

/// The command a person recognises: a shell's `-c` script (the inner text of
/// an `eval '...'` wrapper), else the argv.
pub(super) fn display(argv: &[String]) -> String {
    let shell = argv.first().is_some_and(|program| {
        matches!(
            program
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .trim_start_matches('-'),
            "sh" | "bash" | "dash" | "zsh" | "ksh" | "fish"
        )
    });
    let script = shell
        .then(|| {
            argv.iter()
                .position(|argument| argument == "-c")
                .and_then(|at| argv[at + 1..].iter().find(|a| !a.starts_with('-')))
        })
        .flatten();
    let line = match script {
        Some(script) => script
            .split_once("eval '")
            .and_then(|(_, inner)| inner.split_once('\''))
            .map_or(script.as_str(), |(inner, _)| inner)
            .to_string(),
        None => argv.join(" "),
    };
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(160) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line,
    }
}

#[cfg(target_os = "linux")]
fn arguments(pid: u32) -> Option<(Vec<String>, Option<PathBuf>)> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv = raw
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();
    Some((argv, std::fs::read_link(format!("/proc/{pid}/cwd")).ok()))
}

#[cfg(target_os = "macos")]
fn arguments(pid: u32) -> Option<(Vec<String>, Option<PathBuf>)> {
    let output = crate::process_spawn::command("ps")
        .args(["-o", "args=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let argv = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let mut info = std::mem::MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    // SAFETY: the buffer is one proc_vnodepathinfo of the size passed.
    let read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    let cwd = (read == size).then(|| {
        // SAFETY: proc_pidinfo filled the whole buffer; the path is NUL-terminated.
        let info = unsafe { info.assume_init() };
        let path = unsafe { std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
        PathBuf::from(path.to_string_lossy().into_owned())
    });
    Some((argv, cwd))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn arguments(_pid: u32) -> Option<(Vec<String>, Option<PathBuf>)> {
    None
}

/// Bytes a process has written to storage.
#[cfg(target_os = "linux")]
pub(super) fn written(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/io"))
        .ok()
        .and_then(|io| {
            io.lines()
                .find_map(|line| line.strip_prefix("write_bytes:"))
                .and_then(|value| value.trim().parse().ok())
        })
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
pub(super) fn written(pid: u32) -> u64 {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: the buffer is one rusage_info_v2, the flavor asked for.
    let read = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast(),
        )
    };
    if read != 0 {
        return 0;
    }
    // SAFETY: proc_pid_rusage succeeded and filled the buffer.
    unsafe { info.assume_init() }.ri_diskio_byteswritten
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn written(_pid: u32) -> u64 {
    0
}

/// Expected peaks by command key, kept across kernel restarts.
struct Peaks {
    known: BTreeMap<String, Peak>,
    file: Option<PathBuf>,
}
impl Peaks {
    fn load(file: Option<PathBuf>) -> Self {
        let known = file
            .as_ref()
            .and_then(|file| std::fs::read(file).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { known, file }
    }

    fn get(&self, key: &str) -> Option<Peak> {
        self.known.get(key).copied()
    }

    /// Light runs of an unknown command are not worth remembering.
    fn record(&mut self, key: &str, peak: Peak, light: u64) {
        if peak.memory < light && peak.writes < light && !self.known.contains_key(key) {
            return;
        }
        if self.known.len() >= REMEMBERED && !self.known.contains_key(key) {
            let Some(first) = self.known.keys().next().cloned() else {
                return;
            };
            self.known.remove(&first);
        }
        self.known.insert(key.to_string(), peak);
        if let Some(file) = &self.file {
            let written = serde_json::to_vec(&self.known)
                .map_err(std::io::Error::other)
                .and_then(|bytes| {
                    let partial = file.with_extension("partial");
                    std::fs::write(&partial, bytes)?;
                    std::fs::rename(partial, file)
                });
            if let Err(error) = written {
                crate::logging::warn_with_fields(
                    "process_resources.admission_peaks",
                    "expected command peaks could not be saved; they are kept until the kernel stops",
                    serde_json::json!({"error": error.to_string()}),
                );
            }
        }
    }
}

#[cfg(test)]
mod ledger_tests;

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn the_line_takes_one_command_per_owner_in_turn() {
        let owner = |unit: &str, arrivals: &[u64]| {
            Arc::new(Mutex::new(Observation {
                unit: unit.into(),
                admission: arrivals
                    .iter()
                    .map(|arrived| {
                        (
                            *arrived as u32,
                            Entry {
                                arrived: *arrived,
                                wait: Some(Wait::Held(WaitReason::Memory)),
                                ..Entry::default()
                            },
                        )
                    })
                    .collect(),
                ..Observation::default()
            }))
        };
        let members = [owner("a", &[1, 2, 3]), owner("b", &[4]), owner("c", &[5])];
        let order = line(&members, WaitReason::Memory)
            .into_iter()
            .map(|(_, pid)| pid)
            .collect::<Vec<_>>();
        assert_eq!(order, [1, 4, 5, 2, 3]);
        assert!(line(&members, WaitReason::Disk).is_empty());
    }

    #[test]
    fn a_shell_wrapped_command_reads_as_the_command_and_reruns_share_a_key() {
        let wrapped = [
            "/bin/bash",
            "-c",
            "-l",
            "source /home/u/.snap/snapshot-bash-1730000000-a1b2c3.sh && eval 'cargo build --release' < /dev/null",
        ]
        .map(String::from);
        assert_eq!(display(&wrapped), "cargo build --release");
        assert_eq!(
            display(&["sh", "-c", "pnpm   test"].map(String::from)),
            "pnpm test"
        );
        assert_eq!(display(&["git", "status"].map(String::from)), "git status");
        assert_eq!(
            normalize("/tmp/claude-4f3a9c1d-cwd"),
            normalize("/tmp/claude-77aa01bc-cwd")
        );
        assert_ne!(normalize("cargo build"), normalize("cargo test"));
    }
}
