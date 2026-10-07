//! MD-2: verify membership before signaling a controller's private Unix group.
use std::collections::{BTreeMap, BTreeSet};

struct Process {
    parent: u32,
    group: u32,
    started: String,
}

pub(super) struct OwnedProcessGroup {
    root: u32,
    known: BTreeMap<u32, String>,
    #[cfg(target_os = "linux")]
    seen: BTreeSet<u32>,
}

impl OwnedProcessGroup {
    pub(super) fn new(root: u32) -> Self {
        let mut group = Self {
            root,
            known: BTreeMap::new(),
            #[cfg(target_os = "linux")]
            seen: BTreeSet::new(),
        };
        group.refresh();
        group
    }

    pub(super) fn refresh(&mut self) {
        #[cfg(target_os = "linux")]
        let processes = linux_refresh(self.root, &self.known, &mut self.seen);
        #[cfg(not(target_os = "linux"))]
        let processes = snapshot(self.root);
        if let Some(processes) = processes {
            self.remember(&processes);
        }
    }

    fn remember(&mut self, processes: &BTreeMap<u32, Process>) {
        if self.root <= 1 || self.root > i32::MAX as u32 {
            return;
        }
        for (&pid, process) in processes {
            if process.group == self.root && self.owns(pid, processes) {
                self.known.insert(pid, process.started.clone());
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
            if (pid == self.root && !self.known.contains_key(&pid))
                || self.known.get(&pid) == Some(&process.started)
            {
                return true;
            }
            pid = process.parent;
        }
        false
    }

    pub(super) fn signal(&mut self) {
        if self.root <= 1 || self.root > i32::MAX as u32 {
            return;
        }
        let Some(processes) = snapshot(self.root) else {
            return;
        };
        let members: Vec<_> = processes
            .iter()
            .filter(|(_, process)| process.group == self.root)
            .collect();
        // Reject a missing group or any member not proven to descend from our child.
        if members.is_empty() || members.iter().any(|(pid, _)| !self.owns(**pid, &processes)) {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.root as i32), libc::SIGKILL);
        }
    }
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
            known: BTreeMap::new(),
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
    #[cfg(target_os = "linux")]
    fn md_display_proc_identity_handles_commands_and_checks_start_time() {
        let stat = "42 (command ) with spaces) S 2 42 42 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 12345 0";
        let process = linux_identity(stat).unwrap();
        assert_eq!(process.parent, 2);
        assert_eq!(process.group, 42);
        assert_eq!(process.started, "12345");
        assert!(linux_identity("42 (broken) S 2").is_none());
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
