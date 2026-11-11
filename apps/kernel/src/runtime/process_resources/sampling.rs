use crate::runtime::bounded_process::BoundedCommandExt as _;
use crate::runtime::kernel_access::process::{inspect, ProcessIdentity};
use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ProcessSample {
    pub process_id: String,
    pub pid: u32,
    pub command: String,
    pub resident_set_bytes: u64,
    /// CPU usage in thousandths of one core.
    pub cpu_millis: u64,
    /// Cumulative bytes read from and written to storage; null when unavailable.
    pub disk_io_bytes: Option<u64>,
    /// Bytes this process wrote to storage; null when unavailable.
    pub disk_written_bytes: Option<u64>,
    /// Bytes of deleted files it still holds open: they take space until it
    /// lets go. Counted only for a process that wrote a lot.
    pub deleted_open_bytes: u64,
    /// It kept running after the command that started it ended.
    pub outlived: bool,
    pub sampled_at_ms: u64,
    pub age_ms: u64,
}

/// A process still executing its program is unreadable for a moment (its
/// executable is missing), which looks like an exit. Ownership of a process
/// ends only after this many non-resource failures in a row that could not read it.
const UNREADABLE_SAMPLES: u8 = 8;

/// The processes a watch owns, as a hot reload hands them to the next program:
/// each with the start identity that keeps a recycled PID from being adopted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Owned {
    root: u32,
    processes: Vec<OwnedProcess>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OwnedProcess {
    process_id: String,
    identity: ProcessIdentity,
}

fn fresh_process_id() -> String {
    format!("process-{:032x}", rand::random::<u128>())
}

impl Owned {
    /// Adds processes the last sample did not list, such as a child its
    /// parent forked just before the sample and ended right after.
    pub fn including(&mut self, pids: impl IntoIterator<Item = u32>) {
        for pid in pids {
            if pid > 1 && !self.processes.iter().any(|owned| owned.identity.pid == pid) {
                if let Ok((identity, _)) = inspect(pid) {
                    self.processes.push(OwnedProcess {
                        process_id: fresh_process_id(),
                        identity,
                    });
                }
            }
        }
    }

    /// The owned processes that still run.
    pub fn alive(&self) -> impl Iterator<Item = u32> + '_ {
        self.processes
            .iter()
            .filter(|owned| same_process(owned.identity.pid, &owned.identity))
            .map(|owned| owned.identity.pid)
    }
}
/// A child started this soon after its parent is part of its launcher
/// (process ages have one-second resolution).
const LAUNCH_WINDOW_MS: u64 = 3000;

pub(super) struct ProcessTree {
    root: u32,
    owned: BTreeMap<u32, ProcessIdentity>,
    ids: BTreeMap<u32, String>,
    /// Parent of each owned process at the last sample.
    parents: BTreeMap<u32, u32>,
    /// Failed reads so far of a root that is not owned yet.
    root_unread: Option<u8>,
    /// Consecutive failed reads of each owned process.
    unreadable: BTreeMap<u32, u8>,
    /// The root and the launcher chain that started with it (a wrapper
    /// running the provider's server): the harness. Commands are what it
    /// starts later.
    harness: std::collections::BTreeSet<u32>,
    /// Shells the harness started that ran several jobs at once (a tool call
    /// that starts a holder and a grower), by birth: each job is a command of
    /// its own, so the newest growing one can wait without the shell's other
    /// jobs. A shell stays one while it lives, so a job whose siblings ended
    /// keeps its command, its wait and its stop.
    launchers: BTreeMap<u32, String>,
    #[cfg(target_os = "linux")]
    cgroup: Option<std::sync::Arc<super::cgroup::Domain>>,
    /// Without a cgroup: the environment entry the workload's processes
    /// inherit, however far they detach from it.
    #[cfg(target_os = "linux")]
    marker: Option<Vec<u8>>,
    /// Processes already read without the marker, by the birth they had then.
    #[cfg(target_os = "linux")]
    unmarked: BTreeMap<u32, String>,
    /// Since when the workload owned nothing, while it is still waited on.
    #[cfg(target_os = "linux")]
    empty_since: Option<std::time::Instant>,
}

/// A job that is mid-exec cannot be told from a stranger yet; the workload
/// is over once nothing of it showed up for this long.
#[cfg(target_os = "linux")]
const NOTHING_LEFT_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(target_os = "linux")]
const MARKER_NAME: &str = "CHARIOX_RESOURCE_DOMAIN";

#[cfg(target_os = "linux")]
pub(super) fn new_marker() -> String {
    format!("{:032x}", rand::random::<u128>())
}

#[cfg(target_os = "linux")]
pub(super) fn marker_entry(marker: &str) -> String {
    format!("{MARKER_NAME}={marker}")
}

impl ProcessTree {
    pub fn new(root: u32) -> Self {
        let mut tree = Self {
            root,
            owned: BTreeMap::new(),
            ids: BTreeMap::new(),
            parents: BTreeMap::new(),
            root_unread: Some(0),
            unreadable: BTreeMap::new(),
            harness: std::collections::BTreeSet::from([root]),
            launchers: BTreeMap::new(),
            #[cfg(target_os = "linux")]
            cgroup: None,
            #[cfg(target_os = "linux")]
            marker: None,
            #[cfg(target_os = "linux")]
            unmarked: BTreeMap::new(),
            #[cfg(target_os = "linux")]
            empty_since: None,
        };
        // A first read under resource pressure leaves identification for the next sample.
        let _ = tree.identify_root();
        tree
    }

    /// The root is identified at its first readable sample. Only a child of
    /// the kernel is adopted, and a root that never became readable is given up.
    fn identify_root(&mut self) -> Result<(), SamplingError> {
        let Some(failed) = self.root_unread else {
            return Ok(());
        };
        self.root_unread = match inspect(self.root) {
            Ok((identity, parent)) => {
                if parent == std::process::id() {
                    self.adopt(self.root, identity);
                }
                None
            }
            Err(error) => match shortage(&error) {
                Some(errno) => return Err(SamplingError::Short(errno)),
                None if failed < UNREADABLE_SAMPLES => Some(failed + 1),
                None => None,
            },
        };
        Ok(())
    }

    fn adopt(&mut self, pid: u32, identity: ProcessIdentity) {
        self.owned.insert(pid, identity);
        self.ids.insert(pid, fresh_process_id());
    }

    #[cfg(target_os = "linux")]
    pub fn set_marker(&mut self, marker: &str) {
        self.marker = Some(marker_entry(marker).into_bytes());
    }

    /// The tree of a previous program's watch: only processes that are still
    /// the ones it owned, under the ids Health listed them by.
    pub fn restored(owned: Owned) -> Self {
        let mut tree = Self {
            root: owned.root,
            owned: BTreeMap::new(),
            ids: BTreeMap::new(),
            parents: BTreeMap::new(),
            root_unread: None,
            unreadable: BTreeMap::new(),
            harness: std::collections::BTreeSet::from([owned.root]),
            launchers: BTreeMap::new(),
            #[cfg(target_os = "linux")]
            cgroup: None,
            #[cfg(target_os = "linux")]
            marker: None,
            #[cfg(target_os = "linux")]
            unmarked: BTreeMap::new(),
            #[cfg(target_os = "linux")]
            empty_since: None,
        };
        for process in owned.processes {
            let pid = process.identity.pid;
            if same_process(pid, &process.identity) {
                tree.owned.insert(pid, process.identity);
                tree.ids.insert(pid, process.process_id);
            }
        }
        tree
    }

    pub fn owned(&self) -> Owned {
        Owned {
            root: self.root,
            processes: self
                .owned
                .iter()
                .filter_map(|(pid, identity)| {
                    Some(OwnedProcess {
                        process_id: self.ids.get(pid)?.clone(),
                        identity: identity.clone(),
                    })
                })
                .collect(),
        }
    }

    pub fn root(&self) -> u32 {
        self.root
    }

    /// Any owned process, which tells whether the tree has anything left.
    pub fn alive_pid(&self) -> Option<u32> {
        self.owned.keys().next().copied()
    }

    #[cfg(target_os = "linux")]
    pub fn set_cgroup(&mut self, cgroup: std::sync::Arc<super::cgroup::Domain>) {
        self.cgroup = Some(cgroup);
    }

    pub fn sample(&mut self) -> Result<Vec<ProcessSample>, SamplingError> {
        let rows = process_rows(self.root)?;
        self.observe(&rows)?;
        self.parents = rows
            .iter()
            .filter(|row| self.owned.contains_key(&row.pid))
            .map(|row| (row.pid, row.parent))
            .collect();
        self.harness = std::collections::BTreeSet::from([self.root]);
        let mut current = rows.iter().find(|row| row.pid == self.root);
        while let Some(launcher) = current {
            let mut children = rows
                .iter()
                .filter(|row| row.parent == launcher.pid && self.owned.contains_key(&row.pid));
            // A launched server is a program; a shell, or what a shell starts
            // at once, is a command.
            current = match (children.next(), children.next()) {
                (Some(child), None)
                    if !shell(&launcher.command)
                        && !shell(&child.command)
                        && launcher.age_ms.abs_diff(child.age_ms) <= LAUNCH_WINDOW_MS =>
                {
                    self.harness.insert(child.pid);
                    Some(child)
                }
                _ => None,
            };
        }
        self.launchers.retain(|pid, birth| {
            rows.iter()
                .any(|row| row.pid == *pid && row.birth == *birth)
        });
        // A shell whose jobs were commands in the previous program stays their
        // launcher whatever is left of them, so a job that waits is still found.
        // Those jobs are the authority: once the workload's root has ended, the
        // shell that survived it is no longer under the harness.
        let owned = &self.owned;
        let jobs = |shell: u32| {
            rows.iter()
                .filter(move |child| child.parent == shell && owned.contains_key(&child.pid))
        };
        for row in rows.iter().filter(|row| {
            self.owned.contains_key(&row.pid)
                && shell(&row.command)
                && !self.harness.contains(&row.pid)
                && (jobs(row.pid).any(|job| super::admission::handed_over(&owned[&job.pid]))
                    || (self.harness.contains(&row.parent) && jobs(row.pid).nth(1).is_some()))
        }) {
            self.launchers.insert(row.pid, row.birth.clone());
        }
        // Once the original root is gone, only previously observed descendants
        // can remain owned. Never adopt a replacement process with its PID.
        Ok(rows
            .iter()
            .filter(|row| self.owned.contains_key(&row.pid))
            .map(|row| {
                let io = super::admission::disk_io(row.pid);
                let written = io.map(|(_, written)| written);
                ProcessSample {
                    process_id: self.ids[&row.pid].clone(),
                    pid: row.pid,
                    command: row.command.clone(),
                    resident_set_bytes: row.rss,
                    cpu_millis: row.cpu,
                    disk_io_bytes: io.and_then(|(read, written)| read.checked_add(written)),
                    disk_written_bytes: written,
                    deleted_open_bytes: if written
                        .is_some_and(|w| w >= super::disk_use::LOOK_AT_WRITTEN)
                    {
                        super::disk_use::deleted_open_bytes(row.pid)
                    } else {
                        0
                    },
                    // Its parent is not part of the workload any more.
                    outlived: row.pid != self.root && !self.owned.contains_key(&row.parent),
                    age_ms: row.age_ms,
                    sampled_at_ms: row.sampled_at_ms,
                }
            })
            .collect())
    }

    fn observe(&mut self, rows: &[Row]) -> Result<(), SamplingError> {
        self.identify_root()?;
        // A recorded start identity survives exec but never PID recycling.
        let unreadable = &mut self.unreadable;
        let mut short_errno = None;
        self.owned.retain(|pid, identity| match inspect(*pid) {
            Ok((current, _)) => {
                unreadable.remove(pid);
                same_birth(&current, identity)
            }
            Err(error) => match shortage(&error) {
                Some(errno) => {
                    short_errno = Some(errno);
                    true
                }
                None => {
                    let failed = unreadable.entry(*pid).or_default();
                    *failed += 1;
                    *failed <= UNREADABLE_SAMPLES
                }
            },
        });
        if let Some(errno) = short_errno {
            return Err(SamplingError::Short(errno));
        }
        self.unreadable
            .retain(|pid, _| self.owned.contains_key(pid));
        self.ids.retain(|pid, _| self.owned.contains_key(pid));
        #[cfg(target_os = "linux")]
        if let Some(group) = self.cgroup.clone() {
            for pid in group.pids().map_err(|error| match shortage(&error) {
                Some(errno) => SamplingError::Short(errno),
                None => SamplingError::Unusable(format!(
                    "owned process cgroup membership unavailable: {error}"
                )),
            })? {
                if pid > 1 && !self.owned.contains_key(&pid) && group.owns(pid) {
                    if let Ok((identity, _)) = inspect(pid) {
                        if group.owns(pid) && same_process(pid, &identity) {
                            self.adopt(pid, identity);
                        }
                    }
                }
            }
        }
        #[cfg(target_os = "linux")]
        self.adopt_marked(rows);
        let mut changed = true;
        while changed {
            changed = false;
            for row in rows {
                if !self.owned.contains_key(&row.pid) && self.owned.contains_key(&row.parent) {
                    if let Ok((identity, parent)) = inspect(row.pid) {
                        if parent == row.parent {
                            self.adopt(row.pid, identity);
                            changed = true;
                        }
                    }
                }
            }
        }
        #[cfg(target_os = "linux")]
        if !self.owned.is_empty() {
            self.empty_since = None;
        }
        Ok(())
    }

    /// A job that detached (`setsid job &`) and lost its parent is no
    /// descendant any more. Without a cgroup it is still the workload's:
    /// it inherited the marker from the process that started it.
    #[cfg(target_os = "linux")]
    fn adopt_marked(&mut self, rows: &[Row]) {
        let Some(marker) = self.marker.clone() else {
            return;
        };
        let present = rows
            .iter()
            .map(|row| row.pid)
            .collect::<std::collections::BTreeSet<_>>();
        self.unmarked.retain(|pid, _| present.contains(pid));
        for row in rows {
            // Another birth at this PID is another process, even when no
            // sample saw the PID free in between.
            let known = self
                .unmarked
                .get(&row.pid)
                .is_some_and(|birth| *birth == row.birth);
            if known || self.owned.contains_key(&row.pid) {
                continue;
            }
            match std::fs::read(format!("/proc/{}/environ", row.pid)) {
                Ok(environment)
                    if environment
                        .split(|byte| *byte == 0)
                        .any(|entry| entry == marker) =>
                {
                    if let Ok((identity, _)) = inspect(row.pid) {
                        self.adopt(row.pid, identity);
                    }
                }
                // Empty while the process is mid-exec or a zombie: not an answer yet.
                Ok(environment) if environment.is_empty() => {}
                Ok(_) => {
                    self.unmarked.insert(row.pid, row.birth.clone());
                }
                // Another user's process: it never becomes readable.
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                    self.unmarked.insert(row.pid, row.birth.clone());
                }
                // Exited, or not readable yet: ask again at the next sample.
                Err(_) => {}
            }
        }
    }

    /// Only successful process observations separated by the grace interval
    /// establish emptiness. Re-reading a retained table cannot end a watch.
    #[cfg(target_os = "linux")]
    pub fn left_nothing_running(&mut self) -> bool {
        if self.marker.is_none() {
            return true;
        }
        let snapshot = match process_observation(self.root) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                self.empty_since = None;
                return false;
            }
        };
        if self.observe(&snapshot.rows).is_err() || !self.owned.is_empty() {
            self.empty_since = None;
            return false;
        }
        let since = self.empty_since.get_or_insert(snapshot.started_at);
        snapshot.started_at.saturating_duration_since(*since) >= NOTHING_LEFT_GRACE
    }

    /// Stops the target and everything it owns, as the process table reads now:
    /// the cached table can predate a fork. Membership is read again after each
    /// signal round, and the stop stays unresolved when a fresh read fails.
    pub fn stop(
        &mut self,
        process_id: Option<&str>,
        before_root_stop: impl FnOnce(),
        force_domain: impl FnOnce(),
    ) -> Result<Vec<String>, String> {
        let mut rows = fresh_rows(self.root)?;
        self.observe(&rows)?;
        let target = match process_id {
            None => self.root,
            Some(id) => self
                .ids
                .iter()
                .find(|(pid, value)| value.as_str() == id && self.owned.contains_key(pid))
                .map(|(pid, _)| *pid)
                .ok_or_else(|| {
                    "the process already ended or changed ownership; refresh Health".to_string()
                })?,
        };
        // Once the root ended, the whole tree is what it left running.
        if self.owned.is_empty() || (target != self.root && !self.owned.contains_key(&target)) {
            return Err("the process already ended; refresh Health".into());
        }
        let mut selected: std::collections::BTreeSet<u32> = if target == self.root {
            self.owned.keys().copied().collect()
        } else {
            std::collections::BTreeSet::from([target])
        };
        if target == self.root {
            before_root_stop();
        }
        let mut identities = BTreeMap::<u32, ProcessIdentity>::new();
        let mut force_domain = Some(force_domain);
        for signal in [libc::SIGTERM, libc::SIGKILL] {
            if signal == libc::SIGKILL && target == self.root {
                if let Some(force_domain) = force_domain.take() {
                    force_domain();
                }
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
            let mut signalled = std::collections::BTreeSet::new();
            loop {
                self.select_descendants(&rows, &mut selected);
                let fresh = self.signal_unsignalled(signal, &selected, &mut signalled);
                identities.extend(fresh);
                let running = identities.iter().any(|(pid, identity)| {
                    same_process(*pid, identity) || (*pid == self.root && !root_waitable(*pid))
                });
                // A member signalled this round may have forked before it
                // died; only a fresh read shows that child.
                if std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
                rows = fresh_rows(self.root)?;
                self.observe(&rows)?;
                if !running {
                    let mut grown = selected.clone();
                    self.select_descendants(&rows, &mut grown);
                    if grown == selected {
                        break;
                    }
                }
            }
        }
        rows = fresh_rows(self.root)?;
        self.observe(&rows)?;
        self.select_descendants(&rows, &mut selected);
        let survivors = selected.iter().any(|pid| {
            self.owned
                .get(pid)
                .is_some_and(|identity| same_process(*pid, identity))
        }) || identities.iter().any(|(pid, identity)| {
            same_process(*pid, identity) || (*pid == self.root && !root_waitable(*pid))
        });
        if survivors {
            return Err(
                "some owned processes did not exit after TERM and KILL; refresh Health and retry"
                    .into(),
            );
        }
        Ok(selected
            .iter()
            .filter_map(|pid| self.ids.get(pid).cloned())
            .collect())
    }

    /// Adds every owned process whose parent is already selected.
    fn select_descendants(&self, rows: &[Row], selected: &mut std::collections::BTreeSet<u32>) {
        loop {
            let before = selected.len();
            for row in rows {
                if selected.contains(&row.parent) && self.owned.contains_key(&row.pid) {
                    selected.insert(row.pid);
                }
            }
            if selected.len() == before {
                break;
            }
        }
    }

    /// Signals each selected process once per phase, only while it is still the
    /// process this tree recorded. Returns the identities it signalled.
    fn signal_unsignalled(
        &self,
        signal: i32,
        selected: &std::collections::BTreeSet<u32>,
        signalled: &mut std::collections::BTreeSet<u32>,
    ) -> Vec<(u32, ProcessIdentity)> {
        let mut sent = Vec::new();
        for pid in selected {
            let Some(identity) = self.owned.get(pid) else {
                continue;
            };
            if !signalled.insert(*pid) {
                continue;
            }
            #[cfg(unix)]
            if *pid > 1 && *pid <= i32::MAX as u32 && same_process(*pid, identity) {
                let _ = unsafe { libc::kill(*pid as i32, signal) };
            }
            sent.push((*pid, identity.clone()));
        }
        sent
    }

    /// Resident memory and processes per command: the root's own process, and
    /// each subtree under it (after the root ended, each job it left running).
    pub fn commands(&self, samples: &[ProcessSample]) -> BTreeMap<u32, (u64, Vec<u32>)> {
        let mut commands = BTreeMap::<u32, (u64, Vec<u32>)>::new();
        for sample in samples {
            let command = commands.entry(self.top(sample.pid)).or_default();
            command.0 += sample.resident_set_bytes;
            command.1.push(sample.pid);
        }
        commands
    }

    /// The command a process belongs to.
    fn top(&self, pid: u32) -> u32 {
        let mut top = pid;
        for _ in 0..self.parents.len() {
            match self.parents.get(&top) {
                Some(parent)
                    if !self.harness.contains(&top)
                        && !self.harness.contains(parent)
                        && !self.launchers.contains_key(parent)
                        && self.owned.contains_key(parent) =>
                {
                    top = *parent
                }
                _ => break,
            }
        }
        top
    }

    /// The command of the process `process_id` names.
    pub fn command_of(&self, process_id: &str) -> Option<u32> {
        let pid = self
            .ids
            .iter()
            .find(|(pid, id)| id.as_str() == process_id && self.owned.contains_key(pid))
            .map(|(pid, _)| *pid)?;
        (!self.harness.contains(&pid)).then(|| self.top(pid))
    }

    /// What a command runs: past shells that only run one child (`sh -c`,
    /// a harness's tool shell) to that child. A launcher stays itself: its
    /// jobs are commands of their own, whoever is left.
    pub fn effective(&self, command: u32, samples: &[ProcessSample]) -> u32 {
        let mut pid = command;
        for _ in 0..8 {
            let shell = samples
                .iter()
                .find(|sample| sample.pid == pid)
                .is_some_and(|sample| shell(&sample.command))
                && !self.launchers.contains_key(&pid);
            let mut children = self.parents.iter().filter(|(_, parent)| **parent == pid);
            match (shell, children.next(), children.next()) {
                (true, Some((child, _)), None) => pid = *child,
                _ => break,
            }
        }
        pid
    }

    /// Holds (`SIGSTOP`) or continues (`SIGCONT`) one command and everything
    /// it started; only verified processes of this tree, never the root.
    #[cfg(unix)]
    pub fn signal(&self, command: u32, signal: libc::c_int) {
        if self.harness.contains(&command) {
            return;
        }
        for pid in self.subtree(command) {
            if let Some(identity) = self.owned.get(&pid) {
                if pid > 1 && pid <= i32::MAX as u32 && same_process(pid, identity) {
                    let _ = unsafe { libc::kill(pid as i32, signal) };
                }
            }
        }
    }

    fn subtree(&self, command: u32) -> std::collections::BTreeSet<u32> {
        let mut selected = std::collections::BTreeSet::from([command]);
        loop {
            let before = selected.len();
            for (pid, parent) in &self.parents {
                if selected.contains(parent) && self.owned.contains_key(pid) {
                    selected.insert(*pid);
                }
            }
            if selected.len() == before {
                break;
            }
        }
        selected
    }

    /// Ends one command and what it started at once: a runaway would keep
    /// growing through a grace period. Returns the ended process ids.
    pub fn kill(&self, command: u32) -> Vec<String> {
        if command == self.root {
            self.terminate();
            return self.ids.values().cloned().collect();
        }
        let selected = self.subtree(command);
        #[cfg(unix)]
        for pid in &selected {
            if let Some(identity) = self.owned.get(pid) {
                if *pid > 1 && *pid <= i32::MAX as u32 && same_process(*pid, identity) {
                    let _ = unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
                }
            }
        }
        selected
            .iter()
            .filter_map(|pid| self.ids.get(pid).cloned())
            .collect()
    }

    pub fn is_root(&self, pid: u32) -> bool {
        pid == self.root
    }

    /// Part of the harness: never held or paused.
    pub fn in_harness(&self, pid: u32) -> bool {
        self.harness.contains(&pid)
    }

    pub fn terminate(&self) {
        #[cfg(unix)]
        for (pid, identity) in self
            .owned
            .iter()
            .filter(|(pid, _)| **pid != self.root)
            .chain(self.owned.get_key_value(&self.root))
        {
            if *pid > 1 && *pid <= i32::MAX as u32 && same_process(*pid, identity) {
                let _ = unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
            }
        }
    }
}

fn shell(command: &str) -> bool {
    matches!(
        command
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .trim_start_matches('-'),
        "sh" | "bash" | "dash" | "zsh" | "ksh" | "fish"
    )
}

#[cfg(unix)]
fn root_waitable(pid: u32) -> bool {
    if pid <= 1 {
        return false;
    }
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            libc::id_t::from(pid),
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == 0 {
        return unsafe { info.assume_init().si_pid() } != 0;
    }
    // A handed-over process may have another parent; its identity check still
    // decides whether it is alive. Waiting must never reap another owner's child.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD)
}

#[cfg(not(unix))]
fn root_waitable(_: u32) -> bool {
    true
}

fn same_process(pid: u32, identity: &ProcessIdentity) -> bool {
    inspect(pid).is_ok_and(|(current, _)| same_birth(&current, identity))
}

// Linux start ticks and Darwin's unique process ID survive exec. Darwin's
// idversion and executable describe an exec image, so they are deliberately
// retained for kernel-access admission but excluded from resource ownership.
pub(super) fn same_birth(current: &ProcessIdentity, identity: &ProcessIdentity) -> bool {
    current.pid == identity.pid && current.start == identity.start && current.uid == identity.uid
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[test]
    fn resource_ownership_survives_exec_without_accepting_pid_reuse() {
        let launched = ProcessIdentity {
            pid: 42,
            uid: 501,
            start: 123,
            version: 1,
            executable: "/bin/sh".into(),
        };
        let executed = ProcessIdentity {
            version: 2,
            executable: "/bin/sleep".into(),
            ..launched.clone()
        };
        assert!(
            same_birth(&executed, &launched),
            "Darwin exec changes idversion, not process birth"
        );
        assert!(!same_birth(
            &ProcessIdentity {
                start: 124,
                ..executed.clone()
            },
            &launched
        ));
        assert!(!same_birth(
            &ProcessIdentity {
                uid: 502,
                ..executed.clone()
            },
            &launched
        ));
        assert!(!same_birth(
            &ProcessIdentity {
                pid: 43,
                ..executed
            },
            &launched
        ));
    }
}
#[derive(Clone)]
pub(super) struct Row {
    pid: u32,
    parent: u32,
    rss: u64,
    cpu: u64,
    age_ms: u64,
    /// When the process started, as `ps` prints it (whole seconds): unlike the
    /// elapsed time it is the same at every sample of one process.
    birth: String,
    sampled_at_ms: u64,
    command: String,
}

/// Why the process table could not be read.
#[derive(Debug, Clone)]
pub(super) enum SamplingError {
    /// The kernel lacks resources to start `ps` or read a process or cgroup.
    /// This says nothing about the tree; the next tick may read it.
    Short(i32),
    /// A signal ended `ps` before it answered, as a restart of the kernel does
    /// to the children it leaves behind. The tree is as it was; the next tick
    /// may read it.
    Ended,
    /// Nothing will change by trying again: `ps` is missing, refused or
    /// answers with columns the monitor cannot read.
    Unusable(String),
}
impl std::fmt::Display for SamplingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Short(errno) => write!(
                f,
                "process resource monitor cannot sample for now: {}",
                std::io::Error::from_raw_os_error(*errno)
            ),
            Self::Ended => f.write_str("process resource monitor's ps was ended by a signal"),
            Self::Unusable(message) => f.write_str(message),
        }
    }
}
impl From<SamplingError> for String {
    fn from(error: SamplingError) -> Self {
        error.to_string()
    }
}

/// Sampling can retry when the kernel gets descriptors, files or memory back.
fn shortage(error: &std::io::Error) -> Option<i32> {
    #[cfg(unix)]
    return error.raw_os_error().filter(|errno| {
        matches!(
            *errno,
            libc::EMFILE | libc::ENFILE | libc::EAGAIN | libc::ENOMEM
        )
    });
    #[cfg(not(unix))]
    {
        let _ = error;
        None
    }
}

// The sampler owns ps and publishes a new immutable table. Readers never wait
// for process creation, pipe I/O or a previous reader's refresh.
#[derive(Clone)]
struct RowObservation {
    rows: std::sync::Arc<Vec<Row>>,
    #[cfg(target_os = "linux")]
    started_at: std::time::Instant,
}

struct RowSnapshot {
    rows: Option<RowObservation>,
    error: Option<SamplingError>,
}

struct Sampler {
    snapshot: std::sync::Arc<crate::runtime::checked_lock::CheckedMutex<RowSnapshot>>,
    stop: std::sync::mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Sampler {
    fn new(mut read: impl FnMut() -> Result<Vec<Row>, SamplingError> + Send + 'static) -> Self {
        let snapshot = std::sync::Arc::new(crate::runtime::checked_lock::CheckedMutex::new(
            "process_rows_snapshot",
            RowSnapshot {
                rows: None,
                error: None,
            },
        ));
        let published = std::sync::Arc::clone(&snapshot);
        let (stop, stopping) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new().name("chariox-sampler".into()).spawn(move || {
            let mut failed = false;
            loop {
                // Start time prevents a slow pre-fork read from looking fresh
                // when it finally publishes after the parent has exited.
                #[cfg(target_os = "linux")]
                let started_at = std::time::Instant::now();
                let result = read();
                let mut state = published.lock().unwrap_or_else(|e| e.into_inner());
                match result {
                    Ok(rows) => {
                        state.rows = Some(RowObservation {
                            rows: std::sync::Arc::new(rows),
                            #[cfg(target_os = "linux")]
                            started_at,
                        });
                        state.error = None;
                        failed = false;
                    }
                    Err(error) => {
                        state.error = Some(error.clone());
                        drop(state);
                        if !failed {
                            crate::logging::warn_with_fields("process_resources.sampler", "process sample failed; retaining the last good snapshot", serde_json::json!({"operation": "ps", "error": error.to_string()}));
                            failed = true;
                        }
                        if !matches!(stopping.recv_timeout(std::time::Duration::from_millis(200)), Err(std::sync::mpsc::RecvTimeoutError::Timeout)) { break; }
                        continue;
                    }
                }
                drop(state);
                if !matches!(stopping.recv_timeout(std::time::Duration::from_millis(200)), Err(std::sync::mpsc::RecvTimeoutError::Timeout)) { break; }
            }
        }).expect("process sampler thread must start");
        Self {
            snapshot,
            stop,
            worker: Some(worker),
        }
    }

    fn rows(&self) -> Result<std::sync::Arc<Vec<Row>>, SamplingError> {
        let state = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        state
            .rows
            .as_ref()
            .map(|snapshot| snapshot.rows.clone())
            .ok_or_else(|| {
                state
                    .error
                    .clone()
                    .unwrap_or(SamplingError::Short(libc::EAGAIN))
            })
    }

    #[cfg(target_os = "linux")]
    fn successful_observation(&self) -> Result<RowObservation, SamplingError> {
        let state = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        state.rows.clone().ok_or(SamplingError::Short(libc::EAGAIN))
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn sampler() -> &'static Sampler {
    static SAMPLER: std::sync::OnceLock<Sampler> = std::sync::OnceLock::new();
    SAMPLER.get_or_init(|| Sampler::new(read_process_rows))
}

#[cfg(test)]
thread_local! {
    static TEST_SAMPLER: std::cell::RefCell<Option<Sampler>> = const { std::cell::RefCell::new(None) };
}

/// A process table read now, within `ps`'s own deadline, never the cached one.
fn fresh_rows(root: u32) -> Result<Vec<Row>, String> {
    if let Some(errno) = ps_failure::injected(root) {
        return Err(format!(
            "the current processes could not be read ({}); the process may still be running, retry",
            std::io::Error::from_raw_os_error(errno)
        ));
    }
    read_process_rows().map_err(|error| {
        format!(
            "the current processes could not be read ({error}); the process may still be running, retry"
        )
    })
}

pub(super) fn process_rows(root: u32) -> Result<std::sync::Arc<Vec<Row>>, SamplingError> {
    if let Some(errno) = ps_failure::injected(root) {
        return Err(SamplingError::Short(errno));
    }
    #[cfg(test)]
    if let Some(rows) = TEST_SAMPLER.with(|sampler| sampler.borrow().as_ref().map(Sampler::rows)) {
        return rows;
    }
    sampler().rows()
}

#[cfg(target_os = "linux")]
fn process_observation(root: u32) -> Result<RowObservation, SamplingError> {
    if let Some(errno) = ps_failure::injected(root) {
        return Err(SamplingError::Short(errno));
    }
    #[cfg(test)]
    if let Some(observation) = TEST_SAMPLER.with(|sampler| {
        sampler
            .borrow()
            .as_ref()
            .map(Sampler::successful_observation)
    }) {
        return observation;
    }
    sampler().successful_observation()
}

/// Submitter changes need current ancestry without waiting for a ps child.
pub(super) fn live_groups(root: u32) -> Vec<u32> {
    super::terminal_groups::read(root).unwrap_or_else(|error| {
        crate::logging::error_with_fields(
            "process_resources.monitor",
            "the processes of a terminal could not be read when its submitter changed",
            serde_json::json!({"pid": root, "error": error.to_string()}),
        );
        Vec::new()
    })
}

/// Startup may wait once for the sampler, outside every registry/owner lock.
/// Ordinary launches and readers always keep their nonblocking snapshot path.
pub(super) fn warm_up() -> Result<(), SamplingError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match sampler().rows() {
            Ok(_) => return Ok(()),
            Err(SamplingError::Short(libc::EAGAIN)) if std::time::Instant::now() < deadline => {}
            Err(error) => return Err(error),
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub(super) fn ensure_available(root: u32) -> std::io::Result<()> {
    if let Some(errno) = ps_failure::injected(root) {
        return Err(std::io::Error::from_raw_os_error(errno));
    }
    let _ = sampler();
    super::validate_executable("ps", None, std::env::var_os("PATH").as_deref())
}

fn run_ps() -> std::io::Result<std::process::Output> {
    crate::process_spawn::command("ps")
        .env("LC_ALL", "C")
        .args(["-axo", "pid=,ppid=,rss=,pcpu=,etime=,lstart=,comm="])
        .output_with_deadline(std::time::Duration::from_secs(1))
}

/// Makes starting `ps` fail with an errno for the tree of one root process,
/// as a kernel at its descriptor limit does. Other trees, and the tests that
/// watch them, are unaffected.
#[cfg(not(test))]
pub(super) mod ps_failure {
    pub(super) fn injected(_root: u32) -> Option<i32> {
        None
    }
}

#[cfg(test)]
pub(super) mod ps_failure {
    static ERRNO: std::sync::Mutex<std::collections::BTreeMap<u32, i32>> =
        std::sync::Mutex::new(std::collections::BTreeMap::new());

    pub(super) fn injected(root: u32) -> Option<i32> {
        ERRNO
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&root)
            .copied()
    }

    pub(in crate::runtime::process_resources) fn set(root: u32, errno: Option<i32>) {
        let mut faults = ERRNO.lock().unwrap_or_else(|e| e.into_inner());
        match errno {
            Some(errno) => faults.insert(root, errno),
            None => faults.remove(&root),
        };
    }
}

/// Why `ps` did not answer: a signal ends it for now, an exit status means it
/// cannot give the columns.
pub(super) fn ps_refusal(status: std::process::ExitStatus) -> SamplingError {
    if status.code().is_none() {
        return SamplingError::Ended;
    }
    SamplingError::Unusable(
        "process resource monitor failed: ps must support pid, ppid, rss, pcpu, etime and comm"
            .into(),
    )
}

fn read_process_rows() -> Result<Vec<Row>, SamplingError> {
    let sampled_at_ms = crate::session::unix_epoch_ms();
    let output = run_ps().map_err(|error| match shortage(&error) {
        Some(errno) => SamplingError::Short(errno),
        None => SamplingError::Unusable(format!(
            "process resource monitor cannot start ps: {error}; install ps and check the kernel PATH"
        )),
    })?;
    if !output.status.success() {
        return Err(ps_refusal(output.status));
    }
    let rows = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            let rss = fields.next()?.parse::<u64>().ok()?.checked_mul(1024)?;
            let cpu = (fields.next()?.parse::<f64>().ok()? * 10.0) as u64;
            let age_ms = elapsed_ms(fields.next()?)?;
            // Weekday, month, day, time and year.
            let birth = fields.by_ref().take(5).collect::<Vec<_>>();
            if birth.len() < 5 {
                return None;
            }
            let birth = birth.join(" ");
            let command = fields.collect::<Vec<_>>().join(" ");
            Some(Row {
                pid,
                parent,
                rss,
                cpu,
                age_ms,
                birth,
                sampled_at_ms,
                command,
            })
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Err(SamplingError::Unusable(
            "process resource monitor received an empty process table; check ps availability"
                .into(),
        ));
    }
    Ok(rows)
}

fn elapsed_ms(text: &str) -> Option<u64> {
    let (days, clock) = text
        .split_once('-')
        .map_or(Some((0, text)), |(days, clock)| {
            Some((days.parse::<u64>().ok()?, clock))
        })?;
    let seconds = clock.split(':').try_fold(0u64, |total, part| {
        total
            .checked_mul(60)?
            .checked_add(part.parse::<u64>().ok()?)
    })?;
    days.checked_mul(86400)?
        .checked_add(seconds)?
        .checked_mul(1000)
}

#[cfg(all(test, target_os = "linux"))]
mod marker_cache_tests {
    use super::*;

    fn row(pid: u32, birth: &str) -> Row {
        Row {
            pid,
            parent: 1,
            rss: 0,
            cpu: 0,
            age_ms: 0,
            birth: birth.into(),
            sampled_at_ms: 0,
            command: "sleep".into(),
        }
    }

    #[test]
    fn a_process_born_at_a_cached_pid_is_read_again() {
        let marker = new_marker();
        let mut job = std::process::Command::new("sleep")
            .arg("30")
            .env(MARKER_NAME, &marker)
            .spawn()
            .unwrap();
        let pid = job.id();
        let mut tree = ProcessTree::new(std::process::id());
        // Stands for the unmarked process that held this PID first.
        tree.set_marker("another-workload");
        tree.adopt_marked(&[row(pid, "born-first")]);
        tree.set_marker(&marker);
        tree.adopt_marked(&[row(pid, "born-first")]);
        let kept_skipping = !tree.owned.contains_key(&pid);
        // The PID is reused with no sample in between that lacks it.
        tree.adopt_marked(&[row(pid, "born-second")]);
        let adopted = tree.owned.contains_key(&pid);
        let _ = job.kill();
        let _ = job.wait();
        assert!(kept_skipping, "a process already read was read again");
        assert!(adopted, "a marked process on a reused PID was skipped");
    }
}

#[cfg(all(test, unix))]
mod snapshot_stall_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_pre_fork_snapshot_cannot_retire_a_detached_workload() {
        use crate::runtime::bounded_process::BoundedChildExt as _;
        use std::io::BufRead;
        use std::process::Stdio;
        let root = crate::test_support::TestWorktree::new("stale-detached-watch");
        let go = root.path().join("go");
        let marker = new_marker();
        let script = "import os,sys,time,subprocess; print('ready',flush=True)\nwhile not os.path.exists(sys.argv[1]): time.sleep(.01)\nchild=subprocess.Popen(['/bin/sleep','120'],start_new_session=True); print(child.pid,flush=True)";
        let mut parent = crate::process_spawn::command("python3")
            .args(["-c", script])
            .arg(&go)
            .env(MARKER_NAME, &marker)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(parent.stdout.take().unwrap());
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "ready");
        let mut tree = ProcessTree::new(parent.id());
        tree.set_marker(&marker);
        let mut first = Some(read_process_rows().unwrap());
        let (release, resumed) = std::sync::mpsc::channel::<()>();
        let sampler = Sampler::new(move || {
            if let Some(rows) = first.take() {
                return Ok(rows);
            }
            let _ = resumed.recv_timeout(Duration::from_secs(5));
            Err(SamplingError::Ended)
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while sampler.rows().is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        TEST_SAMPLER.with(|slot| *slot.borrow_mut() = Some(sampler));
        std::fs::write(go, "go").unwrap();
        line.clear();
        output.read_line(&mut line).unwrap();
        let child: u32 = line.trim().parse().unwrap();
        let identity = inspect(child).unwrap().0;
        parent.bounded_wait().unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        let mut retired = false;
        while Instant::now() < until {
            tree.sample().unwrap();
            retired |= tree.left_nothing_running();
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(release);
        // Once refresh recovers, this same watch must discover the detached
        // child, then retire only after fresh observations of its real exit.
        TEST_SAMPLER.with(|slot| *slot.borrow_mut() = Some(Sampler::new(read_process_rows)));
        let until = Instant::now() + Duration::from_secs(5);
        let mut adopted = false;
        while Instant::now() < until {
            if tree.sample().is_ok() && tree.owned.contains_key(&child) {
                adopted = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let alive = same_process(child, &identity);
        if child > 1 && child <= i32::MAX as u32 && alive {
            unsafe {
                libc::kill(child as i32, libc::SIGKILL);
            }
        }
        let until = Instant::now() + Duration::from_secs(5);
        let mut ended_after_exit = false;
        while Instant::now() < until {
            let _ = tree.sample();
            if tree.left_nothing_running() {
                ended_after_exit = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        TEST_SAMPLER.with(|slot| slot.borrow_mut().take());
        assert!(alive, "the detached marked workload must still be running");
        assert!(
            !retired,
            "stale observations retired a live detached workload"
        );
        assert!(adopted, "successful refresh must recover ownership");
        assert!(
            ended_after_exit,
            "fresh empty observations must retire the watch"
        );
    }

    #[test]
    fn stalled_ps_keeps_last_snapshot_readable_and_is_reaped() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("stuck-ps");
        let pid_file = root.path().join("ps.pid");
        let target = pid_file.clone();
        let mut calls = 0;
        let sampler = Sampler::new(move || {
            calls += 1;
            if calls == 1 {
                return Ok(vec![Row {
                    pid: 42,
                    parent: 2,
                    rss: 1,
                    cpu: 0,
                    age_ms: 1,
                    birth: "fixture".into(),
                    sampled_at_ms: 1,
                    command: "fixture".into(),
                }]);
            }
            if calls > 2 {
                return Err(SamplingError::Ended);
            }
            let script = format!("import os,signal,time,pathlib; signal.signal(signal.SIGTERM,signal.SIG_IGN); pathlib.Path({:?}).write_text(str(os.getpid())); time.sleep(600)", target.to_string_lossy());
            let result = crate::process_spawn::command("python3")
                .args(["-c", &script])
                .output_with_deadline(Duration::from_millis(500));
            assert!(result.is_err(), "the ps stand-in must hit its deadline");
            Err(SamplingError::Ended)
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while !pid_file.exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        let start = Instant::now();
        for _ in 0..100 {
            assert_eq!(sampler.rows().unwrap()[0].pid, 42);
        }
        assert!(start.elapsed() < Duration::from_millis(100));
        while sampler.snapshot.lock().unwrap().error.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let timed_out = sampler.snapshot.lock().unwrap().error.is_some();
        // Keep the fail-first form safe if the owner regresses to an unbounded wait.
        if !timed_out && pid > 1 && pid <= i32::MAX as u32 {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
        assert_eq!(sampler.rows().unwrap()[0].pid, 42);
        drop(sampler);
        assert!(!crate::runtime::process_health::process_running(pid));
        assert!(timed_out, "stalled ps did not hit its process deadline");
    }
}

#[cfg(all(test, unix))]
mod stop_tests {
    use super::*;
    use crate::runtime::bounded_process::BoundedChildExt as _;
    use std::io::{BufRead, Write};
    use std::time::{Duration, Instant};

    /// A shell that forks `sleep` only after it is told to, so a process table
    /// read before that cannot contain the child.
    fn spawn_forking_shell() -> (
        std::process::Child,
        std::process::ChildStdin,
        std::io::BufReader<std::process::ChildStdout>,
    ) {
        let mut shell = std::process::Command::new("/bin/sh")
            .args(["-c", "read go; sleep 300 & echo $!; wait"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let input = shell.stdin.take().unwrap();
        let output = std::io::BufReader::new(shell.stdout.take().unwrap());
        (shell, input, output)
    }

    fn fork_child(input: &mut std::process::ChildStdin, output: &mut impl BufRead) -> u32 {
        input.write_all(b"go\n").unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        line.trim().parse().unwrap()
    }

    fn kill_if_still_there(pid: u32, identity: &ProcessIdentity) {
        if pid > 1 && pid <= i32::MAX as u32 && same_process(pid, identity) {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }

    #[test]
    fn stop_ends_a_child_forked_after_the_cached_table() {
        let (mut shell, mut input, mut output) = spawn_forking_shell();
        let mut tree = ProcessTree::new(shell.id());
        // The sampler's retained table predates the fork and stays that way.
        let stale = read_process_rows().unwrap();
        let mut stale = Some(stale);
        let sampler = Sampler::new(move || match stale.take() {
            Some(rows) => Ok(rows),
            None => Err(SamplingError::Short(libc::EAGAIN)),
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while sampler.rows().is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        TEST_SAMPLER.with(|slot| *slot.borrow_mut() = Some(sampler));
        let child = fork_child(&mut input, &mut output);
        let identity = inspect(child).unwrap().0;
        assert!(
            !process_rows(shell.id())
                .unwrap()
                .iter()
                .any(|row| row.pid == child),
            "the cached table must not know the child"
        );
        let stopped = tree.stop(None, || {}, || {});
        let survived = same_process(child, &identity);
        kill_if_still_there(child, &identity);
        TEST_SAMPLER.with(|slot| slot.borrow_mut().take());
        let _ = shell.kill();
        let _ = shell.bounded_wait();
        assert!(stopped.is_ok(), "{stopped:?}");
        assert!(!survived, "Stop must end the child the cached table missed");
    }

    #[test]
    fn stop_stays_unresolved_when_the_current_processes_cannot_be_read() {
        let (mut shell, mut input, mut output) = spawn_forking_shell();
        let mut tree = ProcessTree::new(shell.id());
        let child = fork_child(&mut input, &mut output);
        let identity = inspect(child).unwrap().0;
        tree.observe(&read_process_rows().unwrap()).unwrap();
        ps_failure::set(shell.id(), Some(libc::EMFILE));
        let stopped = tree.stop(None, || {}, || {});
        ps_failure::set(shell.id(), None);
        let still_running = same_process(child, &identity) && shell.try_wait().unwrap().is_none();
        kill_if_still_there(child, &identity);
        let _ = shell.kill();
        let _ = shell.bounded_wait();
        let error = stopped.expect_err("an unobservable tree is not stopped");
        assert!(error.contains("may still be running"), "{error}");
        assert!(
            still_running,
            "nothing is signalled without a current table"
        );
    }
}
