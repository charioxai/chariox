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
    pub sampled_at_ms: u64,
    pub age_ms: u64,
}

/// A process still executing its program is unreadable for a moment (its
/// executable is missing), which looks like an exit. Ownership of a process
/// ends only after this many samples in a row that could not read it.
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
            #[cfg(target_os = "linux")]
            cgroup: None,
            #[cfg(target_os = "linux")]
            marker: None,
            #[cfg(target_os = "linux")]
            unmarked: BTreeMap::new(),
            #[cfg(target_os = "linux")]
            empty_since: None,
        };
        tree.identify_root();
        tree
    }

    /// The root is identified at its first readable sample. Only a child of
    /// the kernel is adopted, and a root that never became readable is given up.
    fn identify_root(&mut self) {
        let Some(failed) = self.root_unread else {
            return;
        };
        self.root_unread = match inspect(self.root) {
            Ok((identity, parent)) => {
                if parent == std::process::id() {
                    self.adopt(self.root, identity);
                }
                None
            }
            Err(_) if failed < UNREADABLE_SAMPLES => Some(failed + 1),
            Err(_) => None,
        };
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

    pub fn sample(&mut self) -> Result<Vec<ProcessSample>, String> {
        let rows = process_rows()?;
        self.observe(&rows)?;
        self.parents = rows
            .iter()
            .filter(|row| self.owned.contains_key(&row.pid))
            .map(|row| (row.pid, row.parent))
            .collect();
        // Once the original root is gone, only previously observed descendants
        // can remain owned. Never adopt a replacement process with its PID.
        Ok(rows
            .into_iter()
            .filter(|row| self.owned.contains_key(&row.pid))
            .map(|row| ProcessSample {
                process_id: self.ids[&row.pid].clone(),
                pid: row.pid,
                command: row.command,
                resident_set_bytes: row.rss,
                cpu_millis: row.cpu,
                age_ms: row.age_ms,
                sampled_at_ms: row.sampled_at_ms,
            })
            .collect())
    }

    fn observe(&mut self, rows: &[Row]) -> Result<(), String> {
        self.identify_root();
        // A recorded start identity survives exec but never PID recycling.
        let unreadable = &mut self.unreadable;
        self.owned.retain(|pid, identity| match inspect(*pid) {
            Ok((current, _)) => {
                unreadable.remove(pid);
                same_birth(&current, identity)
            }
            Err(_) => {
                let failed = unreadable.entry(*pid).or_default();
                *failed += 1;
                *failed <= UNREADABLE_SAMPLES
            }
        });
        self.unreadable
            .retain(|pid, _| self.owned.contains_key(pid));
        self.ids.retain(|pid, _| self.owned.contains_key(pid));
        #[cfg(target_os = "linux")]
        if let Some(group) = self.cgroup.clone() {
            for pid in group
                .pids()
                .map_err(|error| format!("owned process cgroup membership unavailable: {error}"))?
            {
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
        if self.owned.is_empty() {
            self.empty_since.get_or_insert_with(std::time::Instant::now);
        } else {
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

    /// Whether the workload left nothing running. A shared sample may predate
    /// a job that detached as the root ended, so a marked workload reads a
    /// fresh one, and only a lasting emptiness counts.
    #[cfg(target_os = "linux")]
    pub fn left_nothing_running(&mut self) -> bool {
        if self.marker.is_none() {
            return true;
        }
        if !read_process_rows().is_ok_and(|rows| self.observe(&rows).is_ok()) {
            return false;
        }
        self.empty_since
            .is_some_and(|since| since.elapsed() >= NOTHING_LEFT_GRACE)
    }

    pub fn stop(
        &mut self,
        process_id: Option<&str>,
        before_root_stop: impl FnOnce(),
        force_domain: impl FnOnce(),
    ) -> Result<Vec<String>, String> {
        let rows = read_process_rows()?;
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
        let mut selected = if target == self.root {
            self.owned.keys().copied().collect()
        } else {
            std::collections::BTreeSet::from([target])
        };
        loop {
            let before = selected.len();
            for row in &rows {
                if selected.contains(&row.parent) && self.owned.contains_key(&row.pid) {
                    selected.insert(row.pid);
                }
            }
            if selected.len() == before {
                break;
            }
        }
        if target == self.root {
            before_root_stop();
        }
        let identities = selected
            .iter()
            .filter_map(|pid| self.owned.get(pid).map(|identity| (*pid, identity.clone())))
            .collect::<Vec<_>>();
        #[cfg(unix)]
        for (pid, identity) in &identities {
            if *pid > 1 && *pid <= i32::MAX as u32 && same_process(*pid, identity) {
                let _ = unsafe { libc::kill(*pid as i32, libc::SIGTERM) };
            }
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline
            && identities
                .iter()
                .any(|(pid, identity)| same_process(*pid, identity))
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if target == self.root {
            force_domain();
        }
        #[cfg(unix)]
        for (pid, identity) in &identities {
            if *pid > 1 && *pid <= i32::MAX as u32 && same_process(*pid, identity) {
                let _ = unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
            }
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline
            && identities
                .iter()
                .any(|(pid, identity)| same_process(*pid, identity))
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if identities
            .iter()
            .any(|(pid, identity)| same_process(*pid, identity))
        {
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

    /// Resident memory per command: the root's own process, and each subtree
    /// under it (after the root ended, each job it left running).
    pub fn commands(&self, samples: &[ProcessSample]) -> BTreeMap<u32, u64> {
        let mut commands = BTreeMap::<u32, u64>::new();
        for sample in samples {
            let mut top = sample.pid;
            for _ in 0..samples.len() {
                match self.parents.get(&top) {
                    Some(parent)
                        if top != self.root
                            && *parent != self.root
                            && self.owned.contains_key(parent) =>
                    {
                        top = *parent
                    }
                    _ => break,
                }
            }
            *commands.entry(top).or_default() += sample.resident_set_bytes;
        }
        commands
    }

    /// Ends one command and what it started at once: a runaway would keep
    /// growing through a grace period. Returns the ended process ids.
    pub fn kill(&self, command: u32) -> Vec<String> {
        if command == self.root {
            self.terminate();
            return self.ids.values().cloned().collect();
        }
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

fn same_process(pid: u32, identity: &ProcessIdentity) -> bool {
    inspect(pid).is_ok_and(|(current, _)| same_birth(&current, identity))
}

// Linux start ticks and Darwin's unique process ID survive exec. Darwin's
// idversion and executable describe an exec image, so they are deliberately
// retained for kernel-access admission but excluded from resource ownership.
fn same_birth(current: &ProcessIdentity, identity: &ProcessIdentity) -> bool {
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
struct Row {
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

fn process_rows() -> Result<Vec<Row>, String> {
    // One shared sample avoids a ps process for every watchdog tick.
    type CachedRows = (std::time::Instant, Result<Vec<Row>, String>);
    static ROWS: std::sync::OnceLock<std::sync::Mutex<CachedRows>> = std::sync::OnceLock::new();
    let mut cache = ROWS
        .get_or_init(|| {
            std::sync::Mutex::new((
                std::time::Instant::now() - std::time::Duration::from_secs(1),
                Err("process sample unavailable".into()),
            ))
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if cache.0.elapsed() >= std::time::Duration::from_millis(200) {
        cache.1 = read_process_rows();
        cache.0 = std::time::Instant::now();
    }
    cache.1.clone()
}

pub(super) fn ensure_available() -> std::io::Result<()> {
    process_rows().map(|_| ()).map_err(std::io::Error::other)
}

fn read_process_rows() -> Result<Vec<Row>, String> {
    let sampled_at_ms = crate::session::unix_epoch_ms();
    let output = crate::process_spawn::command("ps").env("LC_ALL", "C").args(["-axo", "pid=,ppid=,rss=,pcpu=,etime=,lstart=,comm="]).output()
        .map_err(|error| format!("process resource monitor cannot start ps: {error}; install ps and check the kernel PATH"))?;
    if !output.status.success() {
        return Err(
            "process resource monitor failed: ps must support pid, ppid, rss, pcpu, etime and comm"
                .into(),
        );
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
        return Err(
            "process resource monitor received an empty process table; check ps availability"
                .into(),
        );
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
