//! Admission (OF-01..OF-05): a heavy command waits for room instead of growing
//! into a full machine. A command whose last run peaked above the light bar is
//! held before it grows when the pool has no room for that peak; an unknown
//! command growing into a tight pool is paused before anything is stopped.
//! Waiting commands start by themselves, in turn across owners, once their
//! room is free. Light commands never wait.
use super::*;
#[cfg(target_os = "macos")]
use crate::runtime::bounded_process::BoundedCommandExt as _;
use crate::runtime::kernel_access::process::{inspect, ProcessIdentity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

mod receiving_volume;
pub(super) use receiving_volume::ReceivingVolume;

/// Commands that stay under this never wait (OF-03); small pools scale it.
const MAX_LIGHT: u64 = 512 * MIB;
/// Room a paused command of unknown peak needs before it continues.
const MAX_GROWTH_ROOM: u64 = 1024 * MIB;
/// Disk floor: max(5 %, 10 GiB) of the volume (OF-05).
const DISK_FLOOR: u64 = 10 * 1024 * MIB;
/// Slower than this a command is not growing (or writing).
pub(super) const GROWING: u64 = MIB;
/// A start needs a moment to show in the samples.
const SETTLE: Duration = Duration::from_secs(1);
/// A paused command continues with room for this long at its pre-pause rate.
const AHEAD: u64 = 2;
/// A stopped filler needs a moment to give its space back before the next
/// one is judged.
const FILLER_SETTLE: Duration = Duration::from_secs(2);
const REMEMBERED: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    Memory,
    Disk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum Wait {
    /// Stopped before it grew: it waits for room for its expected peak.
    Held(WaitReason),
    /// Stopped while growing into a tight pool or a full disk.
    Paused(WaitReason),
}
impl Wait {
    pub(super) fn reason(self) -> WaitReason {
        match self {
            Wait::Held(reason) | Wait::Paused(reason) => reason,
        }
    }
}

/// The largest memory and the bytes written by one run of a command.
#[derive(Clone, Default, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct Peak {
    pub memory: u64,
    pub writes: u64,
    /// Where the run's output went: a path on the volume that received the
    /// most of its writes, and how much it received there. `None` while no
    /// run showed it, since the working directory says nothing about it (a
    /// checkout on one volume can build into another).
    #[serde(default)]
    pub volume: Option<ReceivingVolume>,
    #[serde(default)]
    pub volume_writes: u64,
    /// The exact command line (numbers kept) of the run that reached it: a
    /// peak decides alone only for that line, since `-j16` and `-j1` share
    /// the normalized key.
    #[serde(default)]
    pub exact: u64,
}

/// What the kernel recognises a command by: its normalized argv and cwd,
/// hashed. The line is what clients show.
#[derive(Clone)]
pub(super) struct Seen {
    pid: u32,
    pub(super) key: String,
    /// The same command line without normalizing its numbers.
    pub(super) exact: u64,
    pub line: String,
    pub cwd: Option<PathBuf>,
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
    /// Typical bytes per second before the pause; never sampled while stopped.
    paused_rate: u64,
    peak: Peak,
    resident: u64,
    writes: u64,
    pub position: u32,
    pub need: u64,
    pub free: u64,
    /// When it began to wait (Unix ms), for the row clients show.
    pub since_ms: u64,
    /// Its known peak is larger than the whole pool: it is ended, not held.
    refused: Option<u64>,
    /// It is the process filling a nearly full disk: it is ended, not held.
    filling: Option<Filling>,
}

/// What a disk-full stop tells the stopped command and its agent.
pub(super) struct Filling {
    pub free: u64,
    pub floor: u64,
    pub rate: u64,
    pub written: u64,
    pub deleted_open: u64,
}
impl Entry {
    /// It waits from now on, for the reason `wait` names.
    fn wait_for(&mut self, wait: Wait) {
        if self.wait.is_none() {
            self.since_ms = crate::session::unix_epoch_ms();
        }
        self.wait = Some(wait);
    }

    /// The job this entry followed is a command of its own from now on: it
    /// takes the whole accounting, and the shell starts over as a launcher.
    fn hand_job_over(&mut self, arrived: u64) -> Entry {
        std::mem::replace(
            self,
            Entry {
                arrived,
                holdable: self.holdable,
                ..Entry::default()
            },
        )
    }

    /// It runs again.
    fn stop_waiting(&mut self) {
        self.wait = None;
    }

    #[cfg(test)]
    pub(super) fn admitted_with_prediction(&self) -> (bool, bool) {
        (self.admitted, self.expected.is_some())
    }

    #[cfg(test)]
    pub(super) fn predicted(pid: u32, peak: Peak, admitted: bool, cwd: &Path) -> Self {
        Self {
            admitted,
            holdable: true,
            expected: Some(peak),
            seen: Some(Seen {
                pid,
                key: format!("test-{pid}"),
                exact: 0,
                line: "child".into(),
                cwd: Some(cwd.into()),
            }),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn holdable(&self) -> bool {
        self.holdable
    }

    /// The volume its last run wrote most to: the one it waits for. Where the
    /// output goes is unknown until a run showed it, and then it is not
    /// held: a destructive disk stop requires observed growth anyway.
    fn disk(&self, room: &Room) -> Option<Disk> {
        self.predicted_disk(room.volume)
    }

    pub(super) fn predicted_disk(&self, volume: &Volume) -> Option<Disk> {
        let expected = self.expected.as_ref()?;
        // Numeric output paths can share the memory key. Only the exact
        // command that wrote here tells us its receiving volume.
        if self.seen.as_ref()?.exact != expected.exact {
            return None;
        }
        expected.volume.as_ref()?.disk(volume)
    }

    /// Writes it still has ahead toward its expected peak on that volume;
    /// only a heavy writer counts.
    fn writes_ahead(&self, room: &Room) -> u64 {
        self.expected
            .as_ref()
            .filter(|peak| peak.volume.is_some() && peak.volume_writes >= room.light)
            .map_or(0, |peak| peak.volume_writes.saturating_sub(self.writes))
    }

    /// Memory it still has ahead toward its expected peak; only a heavy
    /// command counts.
    fn memory_ahead(&self, room: &Room) -> u64 {
        self.expected
            .as_ref()
            .filter(|peak| peak.memory >= room.light)
            .map_or(0, |peak| peak.memory.saturating_sub(self.resident))
    }

    /// Start now: the user admits it whatever the room.
    pub(super) fn force(&mut self) -> bool {
        if self.wait.is_none() {
            return false;
        }
        self.stop_waiting();
        self.forced = true;
        self.admitted = true;
        true
    }
}

/// A command the previous kernel program's admission had stopped (or was
/// resuming) when it handed its processes over, or that the person started
/// now: the next program keeps it waiting and controllable, resumes it when
/// room frees, and never makes a forced one wait again.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct HeldCommand {
    pub(super) identity: crate::runtime::kernel_access::process::ProcessIdentity,
    pub(super) wait: Option<Wait>,
    pub(super) admitted: bool,
    pub(super) forced: bool,
    pub(super) stopped: bool,
    pub(super) paused_rate: u64,
    pub(super) peak: Peak,
    pub(super) arrived: u64,
    pub(super) since_ms: u64,
}

impl HeldCommand {
    /// What a hot reload must carry for `pid`: a command admission has
    /// stopped, or has resumed but not yet continued, or that Start now
    /// admitted for good.
    pub(super) fn of(pid: u32, entry: &Entry) -> Option<Self> {
        if entry.wait.is_none() && !entry.stopped && !entry.forced {
            return None;
        }
        Some(Self {
            identity: inspect(pid).ok()?.0,
            wait: entry.wait,
            admitted: entry.admitted,
            forced: entry.forced,
            stopped: entry.stopped,
            paused_rate: entry.paused_rate,
            peak: entry.peak.clone(),
            arrived: entry.arrived,
            since_ms: entry.since_ms,
        })
    }

    /// Admission's view of the command in the program that adopts it: as
    /// stopped as it was, so that whatever resumes it also continues the
    /// process.
    fn into_entry(self) -> Entry {
        Entry {
            arrived: self.arrived,
            wait: self.wait,
            admitted: self.admitted,
            forced: self.forced,
            holdable: true,
            stopped: self.stopped,
            paused_rate: self.paused_rate,
            peak: self.peak,
            since_ms: self.since_ms,
            ..Entry::default()
        }
    }
}

/// What the previous program handed over, until the watch of the adopted
/// process registers the command it names.
static ADOPTED: std::sync::Mutex<BTreeMap<u32, HeldCommand>> =
    std::sync::Mutex::new(BTreeMap::new());

/// Remembers the previous program's waiting commands and Start now overrides.
/// Their watches start after this, for the processes the handover names.
pub(crate) fn adopt_admission(held: Vec<HeldCommand>) {
    ADOPTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(held.into_iter().map(|held| (held.identity.pid, held)));
}

/// The handed-over command at `pid`, unless the PID now names another process.
fn take_adopted(pid: u32) -> Option<HeldCommand> {
    let held = ADOPTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&pid)?;
    inspect(pid)
        .is_ok_and(|(current, _)| sampling::same_birth(&current, &held.identity))
        .then_some(held)
}

/// Whether the previous program handed over the command that is `identity`.
/// Its parent shell ran several jobs, whoever is left of them now.
pub(super) fn handed_over(identity: &ProcessIdentity) -> bool {
    ADOPTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&identity.pid)
        .is_some_and(|held| sampling::same_birth(identity, &held.identity))
}

/// The machine-wide part of admission, under the pool.
pub(super) struct Ledger {
    arrivals: u64,
    resumed_at: BTreeMap<u8, Instant>,
    /// When a command was last stopped for filling the disk.
    filler_at: Option<Instant>,
    /// The owner whose command started last for each reason: the next one in
    /// line is the following owner's.
    served: BTreeMap<u8, String>,
    pub(super) peaks: Peaks,
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
    /// The machine's free memory, not the pool's ceiling, set `memory`.
    pub host_bound: bool,
    /// Memory is short: pause the newest growing command.
    pub tight: bool,
    pub light: u64,
    pub growth_room: u64,
    /// The whole pool: what a command can never exceed, however long it waits.
    pub capacity: u64,
    pub volume: &'a Volume,
    /// Remaining binary predictions cover the whole domain. Descendant entries
    /// reserve only growth beyond them, independently for each resource.
    pub launches: &'a BTreeMap<String, super::launch::Reservation>,
    pub launch_memory: BTreeMap<String, u64>,
}

impl Ledger {
    pub(super) fn new(file: Option<PathBuf>) -> Self {
        Self {
            arrivals: 0,
            resumed_at: BTreeMap::new(),
            filler_at: None,
            served: BTreeMap::new(),
            peaks: Peaks::load(file),
        }
    }

    /// Growth beyond each domain's already-reserved binary prediction.
    pub(super) fn reserved_memory(
        &self,
        members: &[Arc<Mutex<Observation>>],
        light: u64,
        launch_memory: &BTreeMap<String, u64>,
    ) -> u64 {
        let mut units = BTreeMap::<String, u64>::new();
        for member in members {
            let member = member.lock().unwrap_or_else(|e| e.into_inner());
            let reserved = units.entry(member.unit.clone()).or_default();
            for entry in member
                .admission
                .values()
                .filter(|entry| entry.admitted && entry.wait.is_none())
            {
                *reserved = reserved.saturating_add(
                    entry
                        .expected
                        .as_ref()
                        .filter(|peak| peak.memory >= light)
                        .map_or(0, |peak| peak.memory.saturating_sub(entry.resident)),
                );
            }
        }
        units
            .iter()
            .map(|(unit, reserved)| {
                reserved.saturating_sub(launch_memory.get(unit).copied().unwrap_or(0))
            })
            .fold(0, u64::saturating_add)
    }

    /// Read consumed writes before the caller samples filesystem free space.
    pub(super) fn reserved_disk(
        &self,
        members: &[Arc<Mutex<Observation>>],
        light: u64,
        launches: &BTreeMap<String, super::launch::Reservation>,
        volume: &Volume,
    ) -> BTreeMap<u64, u64> {
        let mut units = BTreeMap::<(String, u64), u64>::new();
        for member in members {
            let member = member.lock().unwrap_or_else(|e| e.into_inner());
            for entry in member
                .admission
                .values()
                .filter(|entry| entry.admitted && entry.wait.is_none())
            {
                let writes = entry
                    .expected
                    .as_ref()
                    .filter(|peak| peak.volume_writes >= light)
                    .map_or(0, |peak| peak.volume_writes.saturating_sub(entry.writes));
                if writes == 0 {
                    continue;
                }
                if let Some(space) = entry.predicted_disk(volume) {
                    let total = units.entry((member.unit.clone(), space.id)).or_default();
                    *total = total.saturating_add(writes);
                }
            }
        }
        let mut reserved = BTreeMap::<u64, u64>::new();
        for ((unit, volume), writes) in units {
            let covered = launches
                .get(&unit)
                .filter(|launch| launch.volume == volume)
                .map_or(0, |launch| launch.writes);
            let total = reserved.entry(volume).or_default();
            *total = total.saturating_add(writes.saturating_sub(covered));
        }
        reserved
    }

    /// Records the current workload's commands: new ones arrive, ended ones
    /// leave their peak for the next run of the same command.
    pub(super) fn register(&mut self, current: &Arc<Mutex<Observation>>, light: u64) {
        let mut observation = current.lock().unwrap_or_else(|e| e.into_inner());
        let observation = &mut *observation;
        let submissions = &mut observation.submissions;
        let now = std::time::Instant::now();
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
                self.peaks.record(
                    &seen.key,
                    Peak {
                        exact: seen.exact,
                        ..peak
                    },
                    light,
                );
            }
        }
        // A shell that was one job's command and now runs several has handed
        // that job to a command of its own: the prediction and the admission
        // go with the job, or both would be reserved for the same peak.
        let handed = observation
            .commands
            .iter()
            .filter(|command| !command.root && !observation.admission.contains_key(&command.pid))
            .filter_map(|command| {
                let (shell, _) = observation.admission.iter().find(|(pid, entry)| {
                    **pid != command.pid
                        && entry
                            .seen
                            .as_ref()
                            .is_some_and(|seen| seen.pid == command.effective)
                })?;
                Some((*shell, command.pid))
            })
            .collect::<Vec<_>>();
        for (shell, job) in handed {
            self.arrivals += 1;
            if let Some(entry) = observation.admission.get_mut(&shell) {
                let handed = entry.hand_job_over(self.arrivals);
                observation.admission.insert(job, handed);
            }
        }
        for command in observation.commands.iter().filter(|command| !command.root) {
            let entry = match observation.admission.entry(command.pid) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(vacant) => {
                    if let Some(held) = take_adopted(command.pid) {
                        // Still waiting, or still forced, as in the previous program.
                        self.arrivals = self.arrivals.max(held.arrived);
                        let entry = vacant.insert(held.into_entry());
                        entry.seen = recognise(command.effective);
                        entry.expected = entry
                            .seen
                            .as_ref()
                            .and_then(|seen| self.peaks.get(&seen.key));
                        entry
                    } else {
                        // Who started it is not known yet: asked again next sample.
                        let Some(holdable) = submissions.claim(command.pid, command.group, now)
                        else {
                            continue;
                        };
                        self.arrivals += 1;
                        vacant.insert(Entry {
                            arrived: self.arrivals,
                            holdable,
                            ..Entry::default()
                        })
                    }
                }
            };
            entry.resident = command.resident;
            entry.writes = command.writes;
            entry.peak.memory = entry.peak.memory.max(command.resident);
            entry.peak.writes = entry.peak.writes.max(command.writes);
            if let Some(output) = observation
                .disk_writes
                .get(&command.pid)
                .and_then(|outputs| outputs.iter().max_by_key(|output| output.written))
                .filter(|output| output.written > entry.peak.volume_writes)
            {
                entry.peak.volume = ReceivingVolume::capture(output);
                entry.peak.volume_writes = output.written;
            }
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
        let mut changed = Vec::new();
        self.hold(current, room, &mut state);
        let mut may_stop = false;
        if room.tight {
            match self.pause(members, WaitReason::Memory, |entry, command| {
                (entry.resident >= room.light
                    && command.growth >= GROWING
                    && Some(entry.arrived) != state.oldest)
                    .then_some(command.steady_growth)
            }) {
                Some(owner) => changed.push(owner),
                None => may_stop = true,
            }
        }
        // Paused commands keep their memory, so a tight pool stays tight
        // until one of them finishes. When nothing heavy runs any more, the
        // next in line continues anyway: one command always makes progress.
        if !room.tight || !state.heavy_running {
            if let Some(owner) = self.resume(members, WaitReason::Memory, &mut state, room) {
                changed.push(owner);
            }
        }
        // A paused writer keeps its space, so nothing pauses for a full disk:
        // below half the floor the process filling it is stopped, with its
        // reason, and the others carry on.
        if let Some(owner) = self.stop_filler(members, room) {
            changed.push(owner);
        }
        if let Some(owner) = self.resume(members, WaitReason::Disk, &mut state, room) {
            changed.push(owner);
        }
        let state = State::read(members, room);
        number(members, &self.served, room, &state);
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

    /// Marks the command filling a nearly full volume: the agent's writer
    /// that wrote the most among those still writing there. A person's own
    /// command is never chosen.
    fn stop_filler(
        &mut self,
        members: &[Arc<Mutex<Observation>>],
        room: &Room,
    ) -> Option<Arc<Mutex<Observation>>> {
        if self
            .filler_at
            .is_some_and(|at| at.elapsed() < FILLER_SETTLE)
        {
            return None;
        }
        // A shared writable file can grow because of either holder. Neither
        // command supplies destructive evidence for that receiving volume.
        let mut holders = BTreeMap::<(u64, u64), usize>::new();
        for member in members {
            let observation = member.lock().unwrap_or_else(|e| e.into_inner());
            for command in &observation.commands {
                for output in observation
                    .disk_writes
                    .get(&command.pid)
                    .into_iter()
                    .flatten()
                {
                    for inode in &output.files {
                        *holders.entry((output.id, *inode)).or_default() += 1;
                    }
                }
            }
        }
        let mut largest: Option<(u64, u32, Filling, &Arc<Mutex<Observation>>)> = None;
        for member in members {
            let observation = member.lock().unwrap_or_else(|e| e.into_inner());
            for command in &observation.commands {
                let Some(entry) = observation.admission.get(&command.pid) else {
                    continue;
                };
                if !entry.holdable || command.write_growth < GROWING {
                    continue;
                }
                for output in observation
                    .disk_writes
                    .get(&command.pid)
                    .into_iter()
                    .flatten()
                {
                    if output.rate < GROWING
                        || output.files.is_empty()
                        || output.files.iter().any(|inode| {
                            holders
                                .get(&(output.id, *inode))
                                .is_none_or(|count| *count != 1)
                        })
                        || largest
                            .as_ref()
                            .is_some_and(|(written, ..)| output.written <= *written)
                    {
                        continue;
                    }
                    // A descriptor can close or be reused between samples.
                    let Some(disk) = disk_room(room.volume, &output.path)
                        .filter(|disk| disk.id == output.id && disk.free < disk.floor / 2)
                    else {
                        continue;
                    };
                    largest = Some((
                        output.written,
                        command.pid,
                        Filling {
                            free: disk.free,
                            floor: disk.floor,
                            rate: output.rate,
                            written: output.written,
                            deleted_open: command.deleted_open,
                        },
                        member,
                    ));
                }
            }
        }
        let (_, pid, filling, member) = largest?;
        let mut observation = member.lock().unwrap_or_else(|e| e.into_inner());
        observation.admission.get_mut(&pid)?.filling = Some(filling);
        self.filler_at = Some(Instant::now());
        Some(member.clone())
    }

    /// The commands of `current` marked as filling the disk; each is reported
    /// once.
    pub(super) fn take_fillers(
        &mut self,
        current: &Arc<Mutex<Observation>>,
    ) -> Vec<(u32, Filling)> {
        let mut observation = current.lock().unwrap_or_else(|e| e.into_inner());
        observation
            .admission
            .iter_mut()
            .filter_map(|(pid, entry)| Some((*pid, entry.filling.take()?)))
            .collect()
    }

    /// The commands of `current` that can never fit the pool, with the peak
    /// they are known to reach; each is reported once.
    pub(super) fn take_refused(&mut self, current: &Arc<Mutex<Observation>>) -> Vec<(u32, u64)> {
        let mut observation = current.lock().unwrap_or_else(|e| e.into_inner());
        observation
            .admission
            .iter_mut()
            .filter_map(|(pid, entry)| Some((*pid, entry.refused.take()?)))
            .collect()
    }

    /// New commands of `current` with a known heavy peak start only with room
    /// for it; otherwise they are held before they grow.
    fn hold(&mut self, current: &Arc<Mutex<Observation>>, room: &Room, state: &mut State) {
        let mut observation = current.lock().unwrap_or_else(|e| e.into_inner());
        let unit = observation.unit.clone();
        for entry in observation.admission.values_mut() {
            if entry.admitted || entry.forced || entry.wait.is_some() {
                continue;
            }
            let Some(expected) = entry.expected.clone() else {
                // Unknown: light until it grows.
                entry.admitted = entry.resident >= room.light;
                continue;
            };
            if entry.holdable && expected.memory > room.capacity {
                // Only the line that reached the peak is refused for it: another
                // line with the same normalized key (-j1 for -j16) starts as an
                // unknown command and is judged by what it uses.
                if entry
                    .seen
                    .as_ref()
                    .is_some_and(|seen| seen.exact == expected.exact)
                {
                    entry.refused = Some(expected.memory);
                    entry.admitted = true;
                } else {
                    entry.expected = None;
                }
                continue;
            }
            let need = entry.memory_ahead(room);
            let disk = entry.disk(room);
            let ahead = entry.writes_ahead(room);
            let held = if !entry.holdable {
                None
            } else if expected.memory >= room.light
                && need > state.memory_room(&unit, room)
                && state.heavy_running
            {
                Some(Wait::Held(WaitReason::Memory))
            } else if disk.is_some_and(|disk| ahead > state.spare(disk, &unit, room)) {
                Some(Wait::Held(WaitReason::Disk))
            } else {
                None
            };
            if let Some(held) = held {
                entry.wait_for(held);
            } else {
                entry.admitted = true;
                state.reserve_memory(&unit, need, room);
                if let Some(disk) = disk {
                    state.reserve_disk(disk, ahead, &unit, room);
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
            entry.wait_for(Wait::Paused(reason));
            entry.paused_rate = rate;
        }
        crate::logging::info_with_fields(
            "process_resources.admission_pause",
            "paused a growing command instead of stopping one",
            serde_json::json!({"pid": pid, "reason": reason, "rate_bytes_per_second": rate}),
        );
        Some(member.clone())
    }

    /// Starts the first eligible command in the owners' fair line. A full
    /// volume must not block commands whose receiving volume has room.
    fn resume(
        &mut self,
        members: &[Arc<Mutex<Observation>>],
        reason: WaitReason,
        state: &mut State,
        room: &Room,
    ) -> Option<Arc<Mutex<Observation>>> {
        if self
            .resumed_at
            .get(&(reason as u8))
            .is_some_and(|at| at.elapsed() < SETTLE)
        {
            return None;
        }
        for (member, pid) in line(members, reason, self.served.get(&(reason as u8))) {
            let mut observation = member.lock().unwrap_or_else(|e| e.into_inner());
            let unit = observation.unit.clone();
            let entry = observation.admission.get_mut(&pid)?;
            if state.blocked_by(entry, &unit, reason, room).is_some() {
                continue;
            }
            let volume = entry.disk(room);
            state.reserve_memory(&unit, entry.memory_ahead(room), room);
            if let Some(volume) = volume {
                state.reserve_disk(volume, entry.writes_ahead(room), &unit, room);
            }
            entry.stop_waiting();
            entry.admitted = true;
            state.running(entry, room);
            let unit = observation.unit.clone();
            drop(observation);
            self.served.insert(reason as u8, unit);
            self.resumed_at.insert(reason as u8, Instant::now());
            return Some(member);
        }
        None
    }
}

/// A command waiting on memory that already started: it holds memory and
/// makes no progress.
pub(super) struct PausedCommand {
    pub pid: u32,
    pub resident: u64,
    pub control: std::sync::mpsc::Sender<WatchCommand>,
}

/// What kept a paused command from continuing: the resource that had too
/// little room, the room left for it and what it needs to go on.
pub(super) enum Blocked {
    Memory { left: u64, need: u64 },
    Disk { spare: u64, need: u64 },
}

/// The newest command paused on memory. When the pool is full and nothing
/// else can pause, it yields before a running command does: it has made the
/// least progress, and the oldest running command can then finish.
pub(super) fn youngest_paused(members: &[Arc<Mutex<Observation>>]) -> Option<PausedCommand> {
    newest_waiting(
        members,
        0,
        |wait| matches!(wait, Wait::Paused(WaitReason::Memory)),
        |_, _| Some(()),
    )
    .map(|(command, ())| command)
}

/// The newest command waiting on memory (paused, or held before it started)
/// for at least `waited_ms` that cannot continue in `room` (a command that can
/// is left to the next resume), with the resource and figures that kept it.
pub(super) fn blocked_waiting(
    members: &[Arc<Mutex<Observation>>],
    waited_ms: u64,
    room: &Room,
) -> Option<(PausedCommand, Blocked)> {
    let state = State::read(members, room);
    newest_waiting(
        members,
        waited_ms,
        |wait| wait.reason() == WaitReason::Memory,
        |entry, unit| state.blocked_by(entry, unit, WaitReason::Memory, room),
    )
}

fn newest_waiting<T>(
    members: &[Arc<Mutex<Observation>>],
    waited_ms: u64,
    waits: impl Fn(&Wait) -> bool,
    verdict: impl Fn(&Entry, &str) -> Option<T>,
) -> Option<(PausedCommand, T)> {
    let now = crate::session::unix_epoch_ms();
    let mut newest: Option<(u64, PausedCommand, T)> = None;
    for member in members {
        let observation = member.lock().unwrap_or_else(|e| e.into_inner());
        let Some(control) = observation.control.clone() else {
            continue;
        };
        for command in &observation.commands {
            let Some(entry) = observation.admission.get(&command.pid) else {
                continue;
            };
            if !entry.wait.as_ref().is_some_and(&waits)
                || now.saturating_sub(entry.since_ms) < waited_ms
                || newest
                    .as_ref()
                    .is_some_and(|(arrived, ..)| entry.arrived <= *arrived)
            {
                continue;
            }
            if let Some(found) = verdict(entry, &observation.unit) {
                newest = Some((
                    entry.arrived,
                    PausedCommand {
                        pid: command.pid,
                        resident: command.resident,
                        control: control.clone(),
                    },
                    found,
                ));
            }
        }
    }
    newest.map(|(_, command, found)| (command, found))
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
    memory_by_unit: BTreeMap<String, u64>,
    writes_by_unit: BTreeMap<(String, u64), u64>,
}
impl State {
    fn memory_credit(&self, unit: &str, room: &Room) -> u64 {
        room.launch_memory
            .get(unit)
            .copied()
            .unwrap_or(0)
            .saturating_sub(self.memory_by_unit.get(unit).copied().unwrap_or(0))
    }

    /// What keeps a waiting command from continuing now, `None` when it can.
    /// Both resources must have room: what it waited for, and the other one
    /// for the peak it is known to reach. Memory is named first when both
    /// lack it.
    fn blocked_by(
        &self,
        entry: &Entry,
        unit: &str,
        reason: WaitReason,
        room: &Room,
    ) -> Option<Blocked> {
        let (memory, disk) = match reason {
            WaitReason::Memory => (need(entry, reason, room), entry.writes_ahead(room)),
            WaitReason::Disk => (entry.memory_ahead(room), need(entry, reason, room)),
        };
        let left = self.memory_room(unit, room);
        if memory > left && self.heavy_running {
            return Some(Blocked::Memory { left, need: memory });
        }
        let spare = self.spare(entry.disk(room)?, unit, room);
        (disk > spare).then_some(Blocked::Disk { spare, need: disk })
    }

    fn memory_room(&self, unit: &str, room: &Room) -> u64 {
        room.memory
            .saturating_sub(self.reserved.saturating_sub(self.memory_credit(unit, room)))
    }

    fn disk_credit(&self, volume: u64, unit: &str, room: &Room) -> u64 {
        room.launches
            .get(unit)
            .filter(|launch| launch.volume == volume)
            .map_or(0, |launch| launch.writes)
            .saturating_sub(
                self.writes_by_unit
                    .get(&(unit.to_string(), volume))
                    .copied()
                    .unwrap_or(0),
            )
    }

    /// Available space includes the requesting domain's unused reservation.
    fn spare(&self, disk: Disk, unit: &str, room: &Room) -> u64 {
        disk.above_floor().saturating_sub(
            self.disk
                .get(&disk.id)
                .copied()
                .unwrap_or(0)
                .saturating_sub(self.disk_credit(disk.id, unit, room)),
        )
    }

    fn reserve_memory(&mut self, unit: &str, bytes: u64, room: &Room) {
        let extra = bytes.saturating_sub(self.memory_credit(unit, room));
        self.reserved = self.reserved.saturating_add(extra);
        let total = self.memory_by_unit.entry(unit.to_string()).or_default();
        *total = total.saturating_add(bytes);
    }

    fn reserve_disk(&mut self, disk: Disk, bytes: u64, unit: &str, room: &Room) {
        let extra = bytes.saturating_sub(self.disk_credit(disk.id, unit, room));
        let total = self.disk.entry(disk.id).or_default();
        *total = total.saturating_add(extra);
        let total = self
            .writes_by_unit
            .entry((unit.to_string(), disk.id))
            .or_default();
        *total = total.saturating_add(bytes);
    }

    /// Notes a command that runs now: the rest of the pass must see it as the
    /// heavy work it is, however it came to run.
    fn running(&mut self, entry: &Entry, room: &Room) {
        let heavy = entry.resident >= room.light
            || entry.admitted
                && entry
                    .expected
                    .as_ref()
                    .is_some_and(|peak| peak.memory >= room.light);
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
            reserved: room
                .launch_memory
                .values()
                .copied()
                .fold(0, u64::saturating_add),
            oldest: None,
            heavy_running: room
                .launches
                .values()
                .any(|launch| launch.memory >= room.light),
            disk: BTreeMap::new(),
            memory_by_unit: BTreeMap::new(),
            writes_by_unit: BTreeMap::new(),
        };
        for launch in room.launches.values() {
            let total = state.disk.entry(launch.volume).or_default();
            *total = total.saturating_add(launch.writes);
        }
        for member in members {
            let observation = member.lock().unwrap_or_else(|e| e.into_inner());
            for entry in observation.admission.values() {
                if entry.wait.is_some() {
                    continue;
                }
                if entry.admitted {
                    state.reserve_memory(&observation.unit, entry.memory_ahead(room), room);
                    let ahead = entry.writes_ahead(room);
                    if let Some(disk) = entry.disk(room).filter(|_| ahead > 0) {
                        state.reserve_disk(disk, ahead, &observation.unit, room);
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
    let expected = entry.expected.as_ref().map(|peak| match reason {
        WaitReason::Memory => peak.memory.saturating_sub(entry.resident),
        WaitReason::Disk => peak.volume_writes.saturating_sub(entry.writes),
    });
    match entry.wait {
        Some(Wait::Held(_)) => expected.unwrap_or(room.growth_room),
        // It continues only with room for its typical growth before the
        // pause, so a burst does not strand it and a stopped sample cannot
        // make a fast grower continue and pause again at once.
        _ => expected
            .filter(|need| *need > 0)
            .unwrap_or(room.growth_room)
            .max(entry.paused_rate.saturating_mul(AHEAD)),
    }
}

/// The waiting commands for `reason` in the order they start: an owner's by
/// arrival, one per owner in turn, so one owner's ten builds cannot starve the
/// others. The owners take turns in name order, the one after `served` first
/// (the one waiting longest when none has started yet).
fn line(
    members: &[Arc<Mutex<Observation>>],
    reason: WaitReason,
    served: Option<&String>,
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
    let mut owners = owners.into_iter().collect::<Vec<_>>();
    for (_, waiting) in &mut owners {
        waiting.sort_by_key(|(arrived, ..)| *arrived);
    }
    let first = match served {
        Some(served) => owners.iter().position(|(unit, _)| unit > served),
        None => (0..owners.len()).min_by_key(|owner| owners[*owner].1[0].0),
    };
    owners.rotate_left(first.unwrap_or(0));
    let mut line = Vec::new();
    for turn in 0..owners
        .iter()
        .map(|(_, waiting)| waiting.len())
        .max()
        .unwrap_or(0)
    {
        for (_, waiting) in &owners {
            if let Some((_, member, pid)) = waiting.get(turn) {
                line.push((member.clone(), *pid));
            }
        }
    }
    line
}

/// Writes each waiting command's place in line, need and free room, for
/// clients.
fn number(
    members: &[Arc<Mutex<Observation>>],
    served: &BTreeMap<u8, String>,
    room: &Room,
    state: &State,
) {
    for reason in [WaitReason::Memory, WaitReason::Disk] {
        let line = line(members, reason, served.get(&(reason as u8)));
        for (position, (member, pid)) in line.into_iter().enumerate() {
            let mut observation = member.lock().unwrap_or_else(|e| e.into_inner());
            let unit = observation.unit.clone();
            let Some(entry) = observation.admission.get_mut(&pid) else {
                continue;
            };
            entry.position = position as u32 + 1;
            entry.need = match (reason, entry.expected.as_ref()) {
                (WaitReason::Memory, Some(peak)) => peak.memory,
                (WaitReason::Disk, Some(peak)) => peak.volume_writes,
                _ => entry.resident + room.growth_room,
            };
            entry.free = match reason {
                WaitReason::Memory => state.memory_room(&unit, room),
                WaitReason::Disk => entry
                    .disk(room)
                    .map_or(0, |disk| state.spare(disk, &unit, room)),
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
pub(super) struct Disk {
    pub(super) id: u64,
    free: u64,
    floor: u64,
}
impl Disk {
    pub(super) fn above_floor(self) -> u64 {
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

/// The volume holding `path`: its id, free bytes and floor.
pub(crate) fn volume_floor(path: &Path) -> Option<(u64, u64, u64)> {
    let disk = disk_room(&statvfs, path)?;
    Some((disk.id, disk.free, disk.floor))
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
    Some(seen_argv(pid, &argv, cwd))
}

pub(super) fn seen_argv(pid: u32, argv: &[String], cwd: Option<PathBuf>) -> Seen {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    let mut exact = sha2::Sha256::new();
    for argument in argv {
        hash.update(normalize(argument).as_bytes());
        hash.update([0]);
        exact.update(argument.as_bytes());
        exact.update([0]);
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
    let exact = exact.finalize();
    Seen {
        pid,
        key,
        exact: u64::from_le_bytes(exact[..8].try_into().expect("a digest has eight bytes")),
        line: display(argv),
        cwd,
    }
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
        .bounded_output()
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

/// Storage I/O, shared by admission and the session process feed. Unreadable
/// counters stay unknown in the feed rather than looking like zero usage.
#[cfg(target_os = "linux")]
pub(super) fn disk_io(pid: u32) -> Option<(u64, u64)> {
    let io = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    let counter = |name: &str| {
        io.lines()
            .find_map(|line| line.strip_prefix(name))?
            .trim()
            .parse()
            .ok()
    };
    Some((counter("read_bytes:")?, counter("write_bytes:")?))
}

#[cfg(target_os = "macos")]
pub(super) fn disk_io(pid: u32) -> Option<(u64, u64)> {
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
        return None;
    }
    // SAFETY: proc_pid_rusage succeeded and filled the buffer.
    let info = unsafe { info.assume_init() };
    Some((info.ri_diskio_bytesread, info.ri_diskio_byteswritten))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn disk_io(_pid: u32) -> Option<(u64, u64)> {
    None
}

pub(super) fn written(pid: u32) -> u64 {
    disk_io(pid).map_or(0, |(_, written)| written)
}

/// Expected peaks by command key, kept across kernel restarts.
pub(super) struct Peaks {
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

    pub(super) fn get(&self, key: &str) -> Option<Peak> {
        self.known.get(key).cloned()
    }

    /// Light runs of an unknown command are not worth remembering.
    pub(super) fn remember(&mut self, key: &str, peak: Peak, light: u64) -> bool {
        if peak.memory < light && peak.writes < light && !self.known.contains_key(key) {
            return false;
        }
        if self.known.len() >= REMEMBERED && !self.known.contains_key(key) {
            let Some(first) = self.known.keys().next().cloned() else {
                return false;
            };
            self.known.remove(&first);
        }
        self.known.insert(key.to_string(), peak);
        true
    }

    pub(super) fn record(&mut self, key: &str, peak: Peak, light: u64) {
        if !self.remember(key, peak, light) {
            return;
        }
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
        let order = line(&members, WaitReason::Memory, None)
            .into_iter()
            .map(|(_, pid)| pid)
            .collect::<Vec<_>>();
        assert_eq!(order, [1, 4, 5, 2, 3]);
        assert!(line(&members, WaitReason::Disk, None).is_empty());
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
