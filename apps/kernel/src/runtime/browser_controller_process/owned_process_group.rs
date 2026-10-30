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
            self.end_members(&owned);
        }
    }

    fn end_members(&self, owned: &[(u32, String)]) {
        for (pid, started) in owned.iter().rev() {
            kill_member(self.root, *pid, started);
        }
    }

    /// The group members proven to descend from our child, and how many are not.
    fn members(&self, processes: &BTreeMap<u32, Process>) -> (Vec<(u32, String)>, usize) {
        let mut owned = Vec::new();
        let mut unproven = 0;
        for (&pid, process) in processes {
            if process.group != self.root {
                continue;
            }
            if self.owns(pid, processes) {
                owned.push((pid, process.started.clone()));
            } else {
                unproven += 1;
            }
        }
        (owned, unproven)
    }
}

/// Ends `pid` only while it still has the start identity recorded for it and
/// still belongs to `root`'s group: the snapshot it came from is no authority
/// once the member may have exited and its PID been reused. On Linux a pidfd
/// pins the process before the identity is read, so a later reuse of the PID
/// cannot receive the signal.
fn kill_member(root: u32, pid: u32, started: &str) {
    let Ok(raw) = i32::try_from(pid) else { return };
    if raw <= 1 {
        return;
    }
    #[cfg(target_os = "linux")]
    match pidfd_open(raw) {
        Ok(pinned) => {
            if still_member(root, pid, started) {
                use std::os::fd::AsRawFd;
                unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        pinned.as_raw_fd(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    );
                }
            }
            return;
        }
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return,
        // A kernel without pidfd: check, then signal by number.
        Err(_) => {}
    }
    if still_member(root, pid, started) {
        unsafe {
            libc::kill(raw, libc::SIGKILL);
        }
    }
}

#[cfg(target_os = "linux")]
fn pidfd_open(pid: i32) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) })
}

fn still_member(root: u32, pid: u32, started: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| linux_identity(&String::from_utf8_lossy(&stat)))
            .is_some_and(|process| process.group == root && process.started == started)
    }
    #[cfg(not(target_os = "linux"))]
    {
        snapshot(root)
            .and_then(|processes| {
                processes
                    .get(&pid)
                    .map(|process| process.started == started)
            })
            .unwrap_or(false)
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
    let current = proc_pids()?;
    // Enumerate every PID, but read identity only for new PIDs and recorded
    // group members. Never cache a signal decision: signal() still scans ALL
    // current identities, rejecting foreign/reused members before any kill.
    let processes = linux_members(
        root,
        current
            .iter()
            .copied()
            .filter(|pid| *pid == root || !seen.contains(pid) || known.contains_key(pid)),
        read_proc_stat,
    )?;
    *seen = current;
    Some(processes)
}

#[cfg(target_os = "linux")]
fn proc_pids() -> Option<BTreeSet<u32>> {
    let mut pids = BTreeSet::new();
    for entry in std::fs::read_dir("/proc").ok()? {
        if let Some(pid) = entry
            .ok()?
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        {
            pids.insert(pid);
        }
    }
    Some(pids)
}

#[cfg(target_os = "linux")]
fn read_proc_stat(pid: u32) -> std::io::Result<String> {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
}

/// Members of group `root` among `pids`. A process that exits while it is read
/// is gone, not a reason to discard the whole scan and leave the group
/// unsignaled: it reports ENOENT, ESRCH when the open or read was already in
/// flight, or state X with parent 0 and group -1 once past exit.
#[cfg(target_os = "linux")]
fn linux_members(
    root: u32,
    pids: impl Iterator<Item = u32>,
    read_stat: impl Fn(u32) -> std::io::Result<String>,
) -> Option<BTreeMap<u32, Process>> {
    let mut processes = BTreeMap::new();
    for pid in pids {
        let stat = match read_stat(pid) {
            Ok(stat) => stat,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(libc::ESRCH) =>
            {
                continue
            }
            Err(_) => return None,
        };
        if exited(&stat) {
            continue;
        }
        let process = linux_identity(&stat)?;
        if process.group == root {
            processes.insert(pid, process);
        }
    }
    Some(processes)
}

#[cfg(target_os = "linux")]
fn exited(stat: &str) -> bool {
    stat.rsplit_once(')')
        .is_some_and(|(_, tail)| tail.trim_start().starts_with(['X', 'x']))
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
        linux_members(root, proc_pids()?.into_iter(), read_proc_stat)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Numeric IDs and start time only; never command lines or environment.
        let output = crate::process_spawn::command("ps")
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
        group.known.lock().unwrap().insert(42, "root".to_string());
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
        assert_eq!(
            group.members(&processes),
            (vec![(42, "root".into()), (43, "child".into())], 1)
        );
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn a_replaced_member_is_not_signalled_after_enumeration() {
        use std::os::unix::process::CommandExt;
        let mut command = std::process::Command::new("sleep");
        command.arg("30");
        unsafe {
            command.pre_exec(|| match libc::setsid() {
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(()),
            });
        }
        let mut child = command.spawn().unwrap();
        let root = child.id();
        let group = OwnedProcessGroup::new(root);
        let started = snapshot(root).unwrap()[&root].started.clone();
        // The PID now belongs to a process other than the one enumerated.
        group.end_members(&[(root, format!("{started}-replaced"))]);
        std::thread::sleep(Duration::from_millis(200));
        let survived = child.try_wait().unwrap().is_none();
        if survived {
            group.end_members(&[(root, started)]);
        }
        let ended = child.wait().is_ok();
        assert!(
            survived,
            "a member with another start identity was signalled"
        );
        assert!(ended);
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn md4_scan_skips_a_process_that_exits_while_it_is_read() {
        let stat =
            |group: u32| format!("1 (x) S 1 {group} {group} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 7 0");
        // A task past exit: state X, parent 0, group -1 (not a valid group id).
        let exiting = "1 (x y)) X 0 -1 -1 0 -1 4227084 83 0 0 0 0 0 0 0 20 0 0 0 7 0";
        let members = linux_members(42, [42, 43].into_iter(), |pid| match pid {
            43 => Ok(exiting.to_string()),
            _ => Ok(stat(42)),
        });
        assert_eq!(
            members.map(|members| members.into_keys().collect::<Vec<_>>()),
            Some(vec![42]),
            "a process past exit must not abort the scan"
        );
        for vanished in [libc::ENOENT, libc::ESRCH] {
            let members = linux_members(42, [42, 43, 44].into_iter(), |pid| match pid {
                43 => Err(std::io::Error::from_raw_os_error(vanished)),
                44 => Ok(stat(99)),
                _ => Ok(stat(42)),
            });
            assert_eq!(
                members.map(|members| members.into_keys().collect::<Vec<_>>()),
                Some(vec![42]),
                "a process reaped mid-read (errno {vanished}) must not abort the scan"
            );
        }
        assert!(
            linux_members(42, [43].into_iter(), |_| Err(
                std::io::Error::from_raw_os_error(libc::EACCES)
            ))
            .is_none(),
            "an unreadable member still fails closed"
        );
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
        crate::process_spawn::command("true").status().unwrap();
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
