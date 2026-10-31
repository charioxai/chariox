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

pub(super) struct ProcessTree {
    root: u32,
    owned: BTreeMap<u32, ProcessIdentity>,
    ids: BTreeMap<u32, String>,
    /// Parent of each owned process at the last sample.
    parents: BTreeMap<u32, u32>,
    #[cfg(target_os = "linux")]
    cgroup: Option<std::sync::Arc<super::cgroup::Domain>>,
}
impl ProcessTree {
    pub fn new(root: u32) -> Self {
        let mut owned = BTreeMap::new();
        if let Ok((identity, _)) = inspect(root) {
            owned.insert(root, identity);
        }
        let ids = owned
            .keys()
            .map(|pid| (*pid, format!("process-{:032x}", rand::random::<u128>())))
            .collect();
        Self {
            root,
            owned,
            ids,
            parents: BTreeMap::new(),
            #[cfg(target_os = "linux")]
            cgroup: None,
        }
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
        // A recorded start identity survives exec but never PID recycling.
        self.owned
            .retain(|pid, identity| same_process(*pid, identity));
        self.ids.retain(|pid, _| self.owned.contains_key(pid));
        #[cfg(target_os = "linux")]
        if let Some(group) = &self.cgroup {
            for pid in group
                .pids()
                .map_err(|error| format!("owned process cgroup membership unavailable: {error}"))?
            {
                if pid > 1 && !self.owned.contains_key(&pid) && group.owns(pid) {
                    if let Ok((identity, _)) = inspect(pid) {
                        if group.owns(pid) && same_process(pid, &identity) {
                            self.owned.insert(pid, identity);
                            self.ids
                                .insert(pid, format!("process-{:032x}", rand::random::<u128>()));
                        }
                    }
                }
            }
        }
        let mut changed = true;
        while changed {
            changed = false;
            for row in rows {
                if !self.owned.contains_key(&row.pid) && self.owned.contains_key(&row.parent) {
                    if let Ok((identity, parent)) = inspect(row.pid) {
                        if parent == row.parent {
                            self.owned.insert(row.pid, identity);
                            self.ids.insert(
                                row.pid,
                                format!("process-{:032x}", rand::random::<u128>()),
                            );
                            changed = true;
                        }
                    }
                }
            }
        }
        Ok(())
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
                    if top != self.root
                        && *parent != self.root
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
        (pid != self.root).then(|| self.top(pid))
    }

    /// What a command runs: past shells that only run one child (`sh -c`,
    /// a harness's tool shell) to that child.
    pub fn effective(&self, command: u32, samples: &[ProcessSample]) -> u32 {
        let mut pid = command;
        for _ in 0..8 {
            let shell = samples
                .iter()
                .find(|sample| sample.pid == pid)
                .is_some_and(|sample| {
                    matches!(
                        sample
                            .command
                            .rsplit('/')
                            .next()
                            .unwrap_or_default()
                            .trim_start_matches('-'),
                        "sh" | "bash" | "dash" | "zsh" | "ksh" | "fish"
                    )
                });
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
        if command == self.root {
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
    let output = crate::process_spawn::command("ps").args(["-axo", "pid=,ppid=,rss=,pcpu=,etime=,comm="]).output()
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
            let command = fields.collect::<Vec<_>>().join(" ");
            Some(Row {
                pid,
                parent,
                rss,
                cpu,
                age_ms,
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
