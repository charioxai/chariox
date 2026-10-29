//! MD-2: verify membership before signaling a controller's private Unix group.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;
use std::time::Duration;

struct Process {
    parent: u32,
    group: u32,
    started: String,
}

pub(super) struct OwnedProcessGroup {
    root: u32,
    known: Arc<Mutex<BTreeMap<u32, String>>>,
    tracker: Option<(Arc<AtomicBool>, JoinHandle<()>)>,
    #[cfg(target_os = "linux")]
    seen: BTreeSet<u32>,
}

impl OwnedProcessGroup {
    /// `root` must be a child that is still unreaped, so its PID cannot belong
    /// to anyone else yet; its start identity is the group's anchor.
    pub(super) fn new(root: u32) -> Self {
        let mut group = Self {
            root,
            known: Arc::new(Mutex::new(BTreeMap::new())),
            tracker: None,
            #[cfg(target_os = "linux")]
            seen: BTreeSet::new(),
        };
        if let Some(processes) = group.scan() {
            if let Some(leader) = processes.get(&root).filter(|process| process.group == root) {
                group
                    .known
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(root, leader.started.clone());
            }
            group.remember(&processes);
        }
        group
    }

    fn scan(&mut self) -> Option<BTreeMap<u32, Process>> {
        #[cfg(target_os = "linux")]
        {
            let known = self
                .known
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            linux_refresh(self.root, &known, &mut self.seen)
        }
        #[cfg(not(target_os = "linux"))]
        snapshot(self.root)
    }

    pub(super) fn refresh(&mut self) {
        if let Some(processes) = self.scan() {
            self.remember(&processes);
        }
    }

    fn remember(&mut self, processes: &BTreeMap<u32, Process>) {
        if self.root <= 1 || self.root > i32::MAX as u32 {
            return;
        }
        for (&pid, process) in processes {
            if process.group == self.root && self.owns(pid, processes) {
                self.known
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(pid, process.started.clone());
            }
        }
    }

    fn owns(&self, mut pid: u32, processes: &BTreeMap<u32, Process>) -> bool {
        let mut visited = BTreeSet::new();
        while pid > 1 && visited.insert(pid) {
            let Some(process) = processes.get(&pid) else {
                return false;
            };
            if process.group != self.root {
                return false;
            }
            let known = self.known.lock().unwrap_or_else(|error| error.into_inner());
            if known.get(&pid) == Some(&process.started) {
                return true;
            }
            pid = process.parent;
        }
        false
    }

    // MD-DISPLAY-04: record descendant start identities independently of the
    // frame/input RPC path. Signaling still verifies EVERY live group member.
    pub(super) fn witness(&self) -> Self {
        Self {
            root: self.root,
            known: self.known.clone(),
            tracker: None,
            #[cfg(target_os = "linux")]
            seen: BTreeSet::new(),
        }
    }
    pub(super) fn start_tracking(&mut self) -> std::io::Result<()> {
        if self.tracker.is_some() {
            return Ok(());
        }
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let root = self.root;
        let known = self.known.clone();
        let worker = std::thread::Builder::new()
            .name("chariox-browser-owned-identities".into())
            .spawn(move || {
                let mut observer = OwnedProcessGroup {
                    root,
                    known,
                    tracker: None,
                    #[cfg(target_os = "linux")]
                    seen: BTreeSet::new(),
                };
                #[cfg(target_os = "linux")]
                let mut allocated = None;
                while !stop.load(Ordering::Acquire) {
                    // MP-08/MP-11: a new descendant needs a newly allocated PID.
                    // Skip the full /proc walk only when none was allocated
                    // since the value read before the previous walk.
                    #[cfg(target_os = "linux")]
                    {
                        let current = last_allocated_pid();
                        if current.is_none() || current != allocated {
                            observer.refresh();
                            allocated = current;
                        }
                    }
                    #[cfg(not(target_os = "linux"))]
                    observer.refresh();
                    std::thread::park_timeout(Duration::from_millis(
                        if cfg!(target_os = "linux") { 25 } else { 250 },
                    ));
                }
            })?;
        self.tracker = Some((stopped, worker));
        Ok(())
    }
    fn stop_tracking(&mut self) {
        if let Some((stop, worker)) = self.tracker.take() {
            stop.store(true, Ordering::Release);
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
    pub(super) fn signal(&mut self) {
        self.stop_tracking();
        if self.root <= 1 || self.root > i32::MAX as u32 {
            return;
        }
        let Some(processes) = snapshot(self.root) else {
            return;
        };
        let (owned, unproven) = self.members(&processes);
        if owned.is_empty() && unproven == 0 {
            return;
        }
        #[cfg(unix)]
        if unproven == 0 {
            unsafe {
                libc::kill(-(self.root as i32), libc::SIGKILL);
            }
        } else {
            // A member we cannot prove is ours may be a foreign process in a
            // reused group, so no group signal; end every proven member
            // by itself instead of leaking the whole tree.
            crate::logging::warn_with_fields(
                "browser_controller.group_kill",
                "browser process group holds members not proven ours; ending only the proven ones",
                serde_json::json!({ "proven": owned.len(), "unproven": unproven }),
            );
            for pid in owned.into_iter().rev() {
                if let Ok(pid) = i32::try_from(pid) {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
            }
        }
    }

    /// The group members proven to descend from our child, and how many are not.
    fn members(&self, processes: &BTreeMap<u32, Process>) -> (Vec<u32>, usize) {
        let mut owned = Vec::new();
        let mut unproven = 0;
        for (&pid, process) in processes {
            if process.group != self.root {
                continue;
            }
            if self.owns(pid, processes) {
                owned.push(pid);
            } else {
                unproven += 1;
            }
        }
        (owned, unproven)
    }
}

impl Drop for OwnedProcessGroup {
    fn drop(&mut self) {
        self.stop_tracking()
    }
}

#[cfg(target_os = "linux")]
fn last_allocated_pid() -> Option<String> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .nth(4)
        .map(str::to_owned)
}

#[cfg(target_os = "linux")]
fn linux_refresh(
    root: u32,
    known: &BTreeMap<u32, String>,
    seen: &mut BTreeSet<u32>,
) -> Option<BTreeMap<u32, Process>> {
    if root <= 1 || root > i32::MAX as u32 {
        return None;
    }
    let current = std::fs::read_dir("/proc")
        .ok()?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .collect::<BTreeSet<_>>();
    let mut processes = BTreeMap::new();
    // Enumerate every PID, but read identity only for new PIDs and recorded
    // group members. Never cache a signal decision: signal() still scans ALL
    // current identities, rejecting foreign/reused members before any kill.
    for &pid in current
        .iter()
        .filter(|pid| **pid == root || !seen.contains(pid) || known.contains_key(pid))
    {
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        let process = linux_identity(&stat)?;
        if process.group == root {
            processes.insert(pid, process);
        }
    }
    *seen = current;
    Some(processes)
}

fn snapshot(root: u32) -> Option<BTreeMap<u32, Process>> {
    if root <= 1 || root > i32::MAX as u32 {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        // MD-DISPLAY-02/04: no ps subprocess on every controller RPC. Read only
        // public numeric identity fields; signaling still takes a fresh, full
        // membership snapshot and rejects foreign/reused group members.
        let mut processes = BTreeMap::new();
        for entry in std::fs::read_dir("/proc").ok()? {
            let entry = entry.ok()?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let stat = match std::fs::read_to_string(entry.path().join("stat")) {
                Ok(stat) => stat,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return None,
            };
            let process = linux_identity(&stat)?;
            if process.group == root {
                processes.insert(pid, process);
            }
        }
        Some(processes)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Numeric IDs and start time only; never command lines or environment.
        let output = std::process::Command::new("ps")
            .args(["-axo", "pid=,ppid=,pgid=,lstart="])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout)
            .ok()?
            .lines()
            .map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?.parse().ok()?;
                let parent = fields.next()?.parse().ok()?;
                let group = fields.next()?.parse().ok()?;
                if group != root {
                    return Some((
                        pid,
                        Process {
                            parent,
                            group,
                            started: String::new(),
                        },
                    ));
                }
                let started = fields.collect::<Vec<_>>().join(" ");
                if started.is_empty() {
                    return None;
                }
                Some((
                    pid,
                    Process {
                        parent,
                        group,
                        started,
                    },
                ))
            })
            .collect()
    }
}

#[cfg(target_os = "linux")]
fn linux_identity(stat: &str) -> Option<Process> {
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    Some(Process {
        parent: fields.get(1)?.parse().ok()?,
        group: fields.get(2)?.parse().ok()?,
        started: fields.get(19)?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn md2_group_membership_rejects_foreign_reused_and_unsafe_ids() {
        let mut group = OwnedProcessGroup {
            root: 42,
            known: Arc::new(Mutex::new(BTreeMap::new())),
            tracker: None,
            #[cfg(target_os = "linux")]
            seen: BTreeSet::new(),
        };
        let mut processes = BTreeMap::from([
            (
                42,
                Process {
                    parent: 2,
                    group: 42,
                    started: "root".into(),
                },
            ),
            (
                43,
                Process {
                    parent: 42,
                    group: 42,
                    started: "child".into(),
                },
            ),
            (
                44,
                Process {
                    parent: 1,
                    group: 42,
                    started: "foreign".into(),
                },
            ),
        ]);
        group
            .known
            .lock()
            .unwrap()
            .insert(42, "root".to_string());
        group.remember(&processes);
        assert!(group.owns(43, &processes));
        assert!(!group.owns(44, &processes));
        processes.remove(&42);
        processes.get_mut(&43).unwrap().parent = 1;
        assert!(group.owns(43, &processes), "recorded orphan remains owned");
        processes.get_mut(&43).unwrap().started = "reused".into();
        assert!(!group.owns(43, &processes));
        group.root = 1;
        group.remember(&processes);
        assert!(!group.owns(1, &processes));
    }
    #[test]
    fn an_unrecorded_root_pid_is_not_owned() {
        // After the controller is reaped its PID can belong to a foreign group
        // leader; only a start identity recorded while it was our child counts.
        let group = OwnedProcessGroup {
            root: 42,
            known: Arc::new(Mutex::new(BTreeMap::new())),
            tracker: None,
            #[cfg(target_os = "linux")]
            seen: BTreeSet::new(),
        };
        let processes = BTreeMap::from([(
            42,
            Process {
                parent: 7,
                group: 42,
                started: "foreign".into(),
            },
        )]);
        assert!(!group.owns(42, &processes));
        assert_eq!(group.members(&processes), (Vec::new(), 1));
    }
    #[test]
    fn an_unproven_member_does_not_hide_the_proven_ones() {
        let group = OwnedProcessGroup {
            root: 42,
            known: Arc::new(Mutex::new(BTreeMap::from([
                (42, "root".to_string()),
                (43, "child".to_string()),
            ]))),
            tracker: None,
            #[cfg(target_os = "linux")]
            seen: BTreeSet::new(),
        };
        let member = |parent, started: &str| Process {
            parent,
            group: 42,
            started: started.into(),
        };
        let processes = BTreeMap::from([
            (42, member(2, "root")),
            (43, member(42, "child")),
            (44, member(1, "reparented-before-seen")),
        ]);
        assert_eq!(group.members(&processes), (vec![42, 43], 1));
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn md_display_proc_identity_handles_commands_and_checks_start_time() {
        let stat = "42 (command ) with spaces) S 2 42 42 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 12345 0";
        let process = linux_identity(stat).unwrap();
        assert_eq!(process.parent, 2);
        assert_eq!(process.group, 42);
        assert_eq!(process.started, "12345");
        assert!(linux_identity("42 (broken) S 2").is_none());
        let before = last_allocated_pid().unwrap();
        std::process::Command::new("true").status().unwrap();
        assert_ne!(
            last_allocated_pid().unwrap(),
            before,
            "a new process allocates a PID"
        );
        for root in [0, 1, u32::MAX] {
            assert!(snapshot(root).is_none());
        }
        let root = std::process::id();
        let processes = snapshot(root);
        if let Some(processes) = processes {
            assert!(processes.values().all(|process| process.group == root));
        }
    }
}
