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
}

impl OwnedProcessGroup {
    pub(super) fn new(root: u32) -> Self {
        let mut group = Self {
            root,
            known: BTreeMap::new(),
        };
        group.refresh();
        group
    }

    pub(super) fn refresh(&mut self) {
        if let Some(processes) = snapshot(self.root) {
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

fn snapshot(root: u32) -> Option<BTreeMap<u32, Process>> {
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
            #[cfg(not(target_os = "linux"))]
            let started = fields.collect::<Vec<_>>().join(" ");
            #[cfg(target_os = "linux")]
            let started = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()?
                .rsplit_once(')')?
                .1
                .split_whitespace()
                .nth(19)?
                .to_string();
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn md2_group_membership_rejects_foreign_reused_and_unsafe_ids() {
        let mut group = OwnedProcessGroup {
            root: 42,
            known: BTreeMap::new(),
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
}
