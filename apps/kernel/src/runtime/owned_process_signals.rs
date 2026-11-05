//! Signals require a retained PID/start witness, including every group member.
use std::{collections::BTreeMap, io, process::Child};

#[derive(Clone, Debug)]
struct ProcessIdentity {
    pid: u32,
    parent: u32,
    group: u32,
    session: u32,
    start: String,
}

pub(crate) struct OwnedProcessSignals {
    root: u32,
    exclusive_session: Option<u32>,
    owned: BTreeMap<u32, String>,
}

impl OwnedProcessSignals {
    pub(crate) fn for_child(child: &Child) -> io::Result<Self> {
        Self::capture(child, false)
    }

    /// Only for a child launched with a successful setsid pre_exec hook.
    /// Retain that launch witness even after macOS clears the exited leader's session flag.
    pub(crate) fn for_session_child(child: &Child) -> io::Result<Self> {
        Self::capture(child, true)
    }

    /// A process this kernel did not just spawn (an orphan found by scanning).
    /// Its identity is only as good as the moment of this snapshot.
    pub(crate) fn for_pid(pid: u32) -> io::Result<Self> {
        Self::capture_pid(pid, false)
    }

    fn capture(child: &Child, launched_exclusive_session: bool) -> io::Result<Self> {
        Self::capture_pid(child.id(), launched_exclusive_session)
    }

    /// The caller launched this async child with a successful setsid hook.
    pub(crate) fn for_tokio_session_child(child: &tokio::process::Child) -> io::Result<Self> {
        Self::capture_pid(
            child
                .id()
                .ok_or_else(|| io::Error::other("child already reaped"))?,
            true,
        )
    }

    fn capture_pid(pid: u32, launched_exclusive_session: bool) -> io::Result<Self> {
        let rows = snapshot()?;
        if pid <= 1 || pid > i32::MAX as u32 {
            return Err(io::Error::other("invalid child PID"));
        }
        let identity = rows
            .iter()
            .find(|row| row.pid == pid)
            .ok_or_else(|| io::Error::other("child start identity unavailable"))?;
        if identity.start.is_empty() {
            return Err(io::Error::other("child birth witness unavailable"));
        }
        if launched_exclusive_session && identity.group != pid {
            return Err(io::Error::other(
                "child no longer has its launched process group",
            ));
        }
        let mut guard = Self {
            root: pid,
            exclusive_session: (launched_exclusive_session || identity.session == pid)
                .then_some(pid),
            owned: BTreeMap::from([(pid, identity.start.clone())]),
        };
        guard.refresh_from(&rows);
        Ok(guard)
    }

    fn matches(&self, row: &ProcessIdentity) -> bool {
        self.owned.get(&row.pid) == Some(&row.start)
    }

    fn refresh_from(&mut self, rows: &[ProcessIdentity]) {
        loop {
            let added: Vec<_> = rows
                .iter()
                .filter(|row| {
                    row.pid > 1
                        && !row.start.is_empty()
                        && !self.owned.contains_key(&row.pid)
                        && (rows
                            .iter()
                            .any(|parent| parent.pid == row.parent && self.matches(parent))
                            || rows.iter().any(|root| {
                                root.pid == self.root
                                    && self.exclusive_session == Some(row.session)
                                    && row.session != 0
                                    && self.matches(root)
                            }))
                })
                .map(|row| (row.pid, row.start.clone()))
                .collect();
            if added.is_empty() {
                break;
            }
            self.owned.extend(added);
        }
    }

    pub(crate) fn refresh(&mut self) -> io::Result<()> {
        self.refresh_from(&snapshot()?);
        Ok(())
    }

    fn group_with(
        &mut self,
        rows: &[ProcessIdentity],
        send: impl FnOnce(i32, Signal) -> io::Result<()>,
    ) -> io::Result<bool> {
        self.refresh_from(rows);
        let members: Vec<_> = rows.iter().filter(|row| row.group == self.root).collect();
        if members.is_empty() {
            return Ok(false);
        }
        if self.root <= 1
            || self.root > i32::MAX as u32
            || members.iter().any(|row| !self.matches(row))
        {
            return Err(io::Error::other("unowned or reused process group member"));
        }
        send(-(self.root as i32), Signal::Kill)?;
        Ok(true)
    }

    pub(crate) fn kill_group(&mut self) -> io::Result<bool> {
        self.group_with(&snapshot()?, send_signal)
    }

    pub(crate) fn kill_owned_processes(&mut self) -> io::Result<()> {
        self.kill_owned_with(snapshot, send_signal)
    }

    /// Freezes the whole tree before ending any member: a waiting parent whose child is killed
    /// first would otherwise resume and run its next command before its own signal arrives.
    fn kill_owned_with(
        &mut self,
        mut inspect: impl FnMut() -> io::Result<Vec<ProcessIdentity>>,
        mut send: impl FnMut(i32, Signal) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut stopped = std::collections::BTreeSet::new();
        loop {
            let rows = inspect()?;
            self.refresh_from(&rows);
            // A member forked before its parent froze appears in the next snapshot.
            let pending: Vec<_> = self
                .parents_first(&rows)
                .into_iter()
                .filter(|pid| !stopped.contains(pid))
                .collect();
            if pending.is_empty() {
                break;
            }
            for pid in pending {
                self.signal_current(pid, Signal::Stop, &mut inspect, &mut send)?;
                stopped.insert(pid);
            }
        }
        let rows = inspect()?;
        for pid in self.parents_first(&rows) {
            self.signal_current(pid, Signal::Kill, &mut inspect, &mut send)?;
        }
        Ok(())
    }

    /// Owned members still alive, every parent before its descendants.
    fn parents_first(&self, rows: &[ProcessIdentity]) -> Vec<u32> {
        let alive: BTreeMap<u32, u32> = rows
            .iter()
            .filter(|row| self.matches(row))
            .map(|row| (row.pid, row.parent))
            .collect();
        let depth = |mut pid: u32| {
            let mut depth = 0;
            while let Some(parent) = alive.get(&pid).copied().filter(|p| alive.contains_key(p)) {
                if depth > alive.len() {
                    break;
                }
                depth += 1;
                pid = parent;
            }
            depth
        };
        let mut order: Vec<_> = alive.keys().map(|pid| (depth(*pid), *pid)).collect();
        order.sort_unstable();
        order.into_iter().map(|(_, pid)| pid).collect()
    }

    /// Rechecks the birth immediately before its individual signal.
    fn signal_current(
        &self,
        pid: u32,
        signal: Signal,
        inspect: &mut impl FnMut() -> io::Result<Vec<ProcessIdentity>>,
        send: &mut impl FnMut(i32, Signal) -> io::Result<()>,
    ) -> io::Result<()> {
        if inspect()?
            .iter()
            .any(|row| row.pid == pid && self.matches(row))
        {
            send(pid as i32, signal)?;
        }
        Ok(())
    }

    /// The launched leader's PID and birth, for a record that outlives this kernel.
    pub(crate) fn root_birth(&self) -> (u32, String) {
        (
            self.root,
            self.owned.get(&self.root).cloned().unwrap_or_default(),
        )
    }

    /// Ends what remains of a tree an earlier kernel launched in its own session with `claim`
    /// in its environment. Ok(true): nothing of it is left. Ok(false): its members were just
    /// signalled; ask again. An error means some member could not be inspected, so it may live.
    pub(crate) fn end_claimed_tree(claim: &str, leader: Option<(u32, &str)>) -> io::Result<bool> {
        Self::end_claimed_tree_with(claim, leader, snapshot, read_environment, send_signal)
    }

    fn end_claimed_tree_with(
        claim: &str,
        leader: Option<(u32, &str)>,
        mut inspect: impl FnMut() -> io::Result<Vec<ProcessIdentity>>,
        environment: impl Fn(u32) -> io::Result<Option<Vec<u8>>>,
        send: impl FnMut(i32, Signal) -> io::Result<()>,
    ) -> io::Result<bool> {
        if claim.is_empty() {
            return Err(io::Error::other("the launch claim is missing"));
        }
        let marker = format!("{CLAIM_VARIABLE}={claim}").into_bytes();
        let rows = inspect()?;
        let mut owned = BTreeMap::new();
        for row in rows.iter().filter(|row| row.pid > 1) {
            let named_by_launch = leader.is_some_and(|(pid, birth)| {
                (row.pid == pid && row.start == birth) || (row.session == pid && pid > 1)
            });
            let marked = match environment(row.pid)? {
                Some(bytes) => bytes.split(|byte| *byte == 0).any(|entry| entry == marker),
                None if named_by_launch => {
                    return Err(io::Error::other("an earlier Setup process is unreadable"))
                }
                None => false,
            };
            if !marked {
                continue;
            }
            if row.start.is_empty() {
                return Err(io::Error::other("an earlier Setup process has no birth"));
            }
            owned.insert(row.pid, row.start.clone());
        }
        let Some(root) = owned.keys().next().copied() else {
            return Ok(true);
        };
        let mut guard = Self {
            root,
            exclusive_session: None,
            owned,
        };
        guard.kill_owned_with(inspect, send)?;
        Ok(false)
    }

    pub(crate) fn kill_child(&mut self) -> io::Result<bool> {
        let rows = snapshot()?;
        let Some(row) = rows.iter().find(|row| row.pid == self.root) else {
            return Ok(false);
        };
        if !self.matches(row) {
            return Err(io::Error::other("child PID was reused"));
        }
        send_signal(self.root as i32, Signal::Kill)?;
        Ok(true)
    }
}

/// Names the run a process belongs to; every descendant inherits it.
pub(crate) const CLAIM_VARIABLE: &str = "CHARIOX_SETUP_CLAIM";

/// Ok(None): the process cannot be read (another user's or hidden), so it proves nothing.
#[cfg(target_os = "linux")]
fn read_environment(pid: u32) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(format!("/proc/{pid}/environ")) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => {
            Ok(Some(Vec::new()))
        }
        Err(error) if matches!(error.raw_os_error(), Some(libc::EACCES | libc::EPERM)) => Ok(None),
        Err(error) => Err(error),
    }
}
/// No environment is readable here: only the launch witnesses can show the tree is gone.
#[cfg(not(target_os = "linux"))]
fn read_environment(_: u32) -> io::Result<Option<Vec<u8>>> {
    Ok(None)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Signal {
    Stop,
    Kill,
}

#[cfg(unix)]
fn send_signal(pid: i32, signal: Signal) -> io::Result<()> {
    // MP-11: reject system-wide targets even if a future caller loses its witness.
    if pid == i32::MIN || (-1..=1).contains(&pid) {
        return Err(io::Error::other("invalid process signal target"));
    }
    // Identity and complete group membership are checked immediately before this sole seam.
    if matches!(pid, -1..=1) {
        return Err(io::Error::other("reserved signal target"));
    }
    let signal = match signal {
        Signal::Stop => libc::SIGSTOP,
        Signal::Kill => libc::SIGKILL,
    };
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}
#[cfg(not(unix))]
fn send_signal(_: i32, _: Signal) -> io::Result<()> {
    Err(io::Error::other("owned signals unsupported"))
}

#[cfg(target_os = "linux")]
fn snapshot() -> io::Result<Vec<ProcessIdentity>> {
    let mut rows = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if let Some(row) = linux_row(pid, std::fs::read(entry.path().join("stat")))? {
            rows.push(row);
        }
    }
    Ok(rows)
}

#[cfg(target_os = "linux")]
fn linux_row(pid: u32, stat: io::Result<Vec<u8>>) -> io::Result<Option<ProcessIdentity>> {
    let bytes = match stat {
        Ok(bytes) => bytes,
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => {
            return Ok(None)
        }
        // hidepid or another user's process: its group and session stay
        // visible, its start identity does not, so it is never ours.
        Err(error) if matches!(error.raw_os_error(), Some(libc::EACCES | libc::EPERM)) => {
            return Ok(unreadable_row(pid))
        }
        Err(error) => return Err(error),
    };
    // The process name is arbitrary bytes; the numbers follow its last ')'.
    let Some(end) = bytes.iter().rposition(|byte| *byte == b')') else {
        return Err(io::Error::other("invalid process identity"));
    };
    let tail = std::str::from_utf8(&bytes[end + 1..]).map_err(io::Error::other)?;
    let fields: Vec<_> = tail.split_whitespace().collect();
    if fields.len() < 20 {
        return Err(io::Error::other("short process identity"));
    }
    // A row whose ids are not numbers (a process that is exiting, or one in another PID
    // namespace) is not one of ours, like a row that has vanished: it must not fail the
    // snapshot of every process on the host.
    let (Ok(parent), Ok(group), Ok(session)) =
        (fields[1].parse(), fields[2].parse(), fields[3].parse())
    else {
        return Ok(None);
    };
    Ok(Some(ProcessIdentity {
        pid,
        parent,
        group,
        session,
        start: fields[19].into(),
    }))
}

#[cfg(target_os = "linux")]
fn unreadable_row(pid: u32) -> Option<ProcessIdentity> {
    let id = i32::try_from(pid).ok()?;
    let (group, session) = unsafe { (libc::getpgid(id), libc::getsid(id)) };
    (group > 0 && session > 0).then(|| ProcessIdentity {
        pid,
        parent: 0,
        group: group as u32,
        session: session as u32,
        start: String::new(),
    })
}
#[cfg(target_os = "macos")]
fn snapshot() -> io::Result<Vec<ProcessIdentity>> {
    let result = crate::process_spawn::command("/bin/ps")
        .args(["-axo", "pid=,ppid=,pgid="])
        .env("LC_ALL", "C")
        .output()?;
    if !result.status.success() {
        return Err(io::Error::other("process metadata unavailable"));
    }
    let mut rows = Vec::new();
    for line in String::from_utf8(result.stdout)
        .map_err(io::Error::other)?
        .lines()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 3 {
            return Err(io::Error::other("invalid process metadata"));
        }
        let parse = |field: &str| {
            field
                .parse::<u32>()
                .map_err(|_| io::Error::other("invalid process ID"))
        };
        let pid = parse(fields[0])?;
        if pid == 0 || pid > i32::MAX as u32 {
            continue;
        }
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        // arg=1 includes zombie metadata: an unreaped leader still pins its PID/birth.
        let read = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                1,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if read != size {
            let error = io::Error::last_os_error();
            if read == 0 && error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            if read == 0 && error.raw_os_error() == Some(libc::EPERM) {
                // Preserve visible foreign group membership for rejection, never admission.
                rows.push(ProcessIdentity {
                    pid,
                    parent: parse(fields[1])?,
                    group: parse(fields[2])?,
                    session: 0,
                    start: String::new(),
                });
                continue;
            }
            return Err(io::Error::other("incomplete native process metadata"));
        }
        if info.pbi_pid != pid || info.pbi_start_tvsec == 0 {
            return Err(io::Error::other("invalid native process birth"));
        }
        let session = macos_session_id(pid, info.pbi_flags, |pid| {
            let session = unsafe { libc::getsid(pid as i32) };
            if session >= 0 {
                return Ok(session as u32);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(0)
            } else {
                Err(error)
            }
        })?;
        rows.push(ProcessIdentity {
            pid,
            parent: info.pbi_ppid,
            group: info.pbi_pgid,
            session,
            start: format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec),
        });
    }
    Ok(rows)
}

#[cfg(any(target_os = "macos", test))]
fn macos_session_id(
    pid: u32,
    flags: u32,
    session_of: impl Fn(u32) -> io::Result<u32>,
) -> io::Result<u32> {
    // Public Darwin proc_info.h PROC_FLAG_SLEADER. Exited leaders lose this flag;
    // their exclusive-session launch witness is retained separately by the guard.
    const PROC_FLAG_SLEADER: u32 = 0x20;
    if flags & PROC_FLAG_SLEADER != 0 {
        Ok(pid)
    } else {
        session_of(pid)
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn snapshot() -> io::Result<Vec<ProcessIdentity>> {
    Err(io::Error::other("owned process inspection unsupported"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(pid: u32, parent: u32, group: u32, start: &str) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            parent,
            group,
            session: 0,
            start: start.into(),
        }
    }
    /// A process can only rename itself, so a shell does it and then waits.
    #[cfg(target_os = "linux")]
    fn spawn_shell_named_invalidly() -> std::process::Child {
        use std::process::Stdio;
        let child = std::process::Command::new("sh")
            .args([
                "-c",
                r#"printf '\377\376' > /proc/$$/comm; read line; true"#,
            ])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let stat = format!("/proc/{}/stat", child.id());
        while !std::fs::read(&stat).unwrap().contains(&0xff) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_foreign_process_with_a_non_utf8_name_does_not_block_a_witness() {
        let mut foreign = spawn_shell_named_invalidly();
        let mut own = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let witness = OwnedProcessSignals::for_child(&own);
        let seen = snapshot();
        let _ = foreign.kill();
        let _ = foreign.wait();
        let _ = own.kill();
        let _ = own.wait();
        assert!(witness.is_ok(), "witness failed: {:?}", witness.err());
        assert!(seen.unwrap().iter().any(|row| row.pid == foreign.id()));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_row_with_non_numeric_ids_is_skipped_not_an_error() {
        let stat =
            |ids: &str| format!("77 (x) S {ids} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 5000 0").into_bytes();
        assert!(linux_row(77, Ok(stat("1 1 1"))).unwrap().is_some());
        for ids in ["1 -1 1", "1 1 -1", "-1 1 1"] {
            assert!(
                linux_row(77, Ok(stat(ids))).unwrap().is_none(),
                "a row with ids `{ids}` failed the snapshot"
            );
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn an_unreadable_process_still_counts_as_a_group_member_but_is_never_owned() {
        let denied = || Err(io::Error::from_raw_os_error(libc::EACCES));
        let own = std::process::id();
        let hidden = linux_row(own, denied())
            .unwrap()
            .expect("row for a live process");
        assert!(hidden.start.is_empty());
        assert_eq!(hidden.group, unsafe { libc::getpgrp() } as u32);
        let mut guard = OwnedProcessSignals {
            root: hidden.group,
            exclusive_session: None,
            owned: BTreeMap::from([(hidden.group, "root".into())]),
        };
        let leader = row(hidden.group, 1, hidden.group, "root");
        assert!(guard
            .group_with(&[leader, hidden], |_, _| panic!(
                "unreadable member signalled"
            ))
            .is_err());
    }

    #[test]
    fn mp11_review_macos_session_witness_admits_fast_reparented_child() {
        assert_eq!(
            macos_session_id(41, 0x20, |_| panic!("leader does not need live getsid")).unwrap(),
            41
        );
        let mut leader = row(41, 10, 41, "precise-leader-birth");
        leader.session = macos_session_id(41, 0, |_| Ok(0)).unwrap(); // zombie getsid = ESRCH
        let mut orphan = row(42, 1, 41, "precise-child-birth");
        orphan.session = macos_session_id(42, 0, |_| Ok(41)).unwrap();
        let rows = [leader, orphan];
        let mut guard = OwnedProcessSignals {
            root: 41,
            exclusive_session: Some(41), // retained setsid launch witness
            owned: BTreeMap::from([(41, rows[0].start.clone())]),
        };
        assert!(guard
            .group_with(&rows, |pid, _| {
                assert_eq!(pid, -41);
                Ok(())
            })
            .unwrap());
        let mut foreign = row(43, 1, 41, "foreign-birth");
        foreign.session = 99;
        assert!(guard
            .group_with(&[rows[0].clone(), foreign], |_, _| panic!(
                "foreign session signalled"
            ))
            .is_err());
        let mut lost = OwnedProcessSignals {
            root: 41,
            exclusive_session: Some(41),
            owned: BTreeMap::from([(41, rows[0].start.clone())]),
        };
        assert!(lost
            .group_with(&rows[1..], |_, _| panic!(
                "missing leader witness signalled"
            ))
            .is_err());
        let mut replacement = rows[0].clone();
        replacement.start = "replacement-leader".into();
        assert!(lost
            .group_with(&[replacement, rows[1].clone()], |_, _| panic!(
                "reused leader admitted children"
            ))
            .is_err());
    }

    #[test]
    fn mp11_review_failed_group_cleanup_signals_only_current_owned_births() {
        let mut guard = OwnedProcessSignals {
            root: 41,
            exclusive_session: None,
            owned: BTreeMap::from([
                (41, "root".into()),
                (43, "child".into()),
                (44, "original".into()),
            ]),
        };
        let rows = vec![
            row(41, 1, 41, "root"),
            row(42, 1, 41, "foreign"),
            row(43, 1, 41, "child"),
            row(44, 1, 41, "reused"),
        ];
        assert!(guard
            .group_with(&rows, |_, _| panic!("mixed group signalled"))
            .is_err());
        let mut signalled = Vec::new();
        guard
            .kill_owned_with(
                || Ok(rows.clone()),
                |pid, signal| {
                    signalled.push((pid, signal));
                    Ok(())
                },
            )
            .unwrap();
        use Signal::{Kill, Stop};
        assert_eq!(signalled, [(41, Stop), (43, Stop), (41, Kill), (43, Kill)]);
    }

    #[test]
    fn an_earlier_kernels_claimed_tree_is_frozen_then_ended_and_unreadable_members_keep_it_alive() {
        let mut leader = row(41, 1, 41, "leader");
        leader.session = 41;
        let mut child = row(42, 41, 41, "child");
        child.session = 41;
        let mut reused = row(43, 1, 43, "reused");
        reused.session = 43;
        let rows = vec![leader, child, reused];
        let environment = |pid: u32| {
            Ok(Some(match pid {
                41 | 42 => b"PATH=/bin\0CHARIOX_SETUP_CLAIM=abc\0".to_vec(),
                _ => b"CHARIOX_SETUP_CLAIM=abcd\0".to_vec(),
            }))
        };
        let mut sent = Vec::new();
        let ended = OwnedProcessSignals::end_claimed_tree_with(
            "abc",
            Some((41, "leader")),
            || Ok(rows.clone()),
            environment,
            |pid, signal| {
                sent.push((pid, signal));
                Ok(())
            },
        )
        .unwrap();
        use Signal::{Kill, Stop};
        assert!(!ended);
        assert_eq!(sent, [(41, Stop), (42, Stop), (41, Kill), (42, Kill)]);
        // Nothing carries the claim and the saved leader birth names another process.
        assert!(OwnedProcessSignals::end_claimed_tree_with(
            "abc",
            Some((43, "earlier")),
            || Ok(vec![rows[2].clone()]),
            environment,
            |_, _| panic!("a reused PID was signalled"),
        )
        .unwrap());
        // A member of the launched session that cannot be read may still be the script.
        assert!(OwnedProcessSignals::end_claimed_tree_with(
            "abc",
            Some((41, "leader")),
            || Ok(rows.clone()),
            |pid| Ok((pid != 42).then(Vec::new)),
            |_, _| panic!("an unproven tree was signalled"),
        )
        .is_err());
    }

    #[test]
    fn mp11_signal_guard_rejects_reserved_and_overflowed_groups_before_io() {
        for root in [0, 1, u32::MAX] {
            let rows = [row(root, 9, root, "birth")];
            let mut guard = OwnedProcessSignals {
                root,
                exclusive_session: None,
                owned: BTreeMap::from([(root, "birth".into())]),
            };
            assert!(guard
                .group_with(&rows, |_, _| panic!("invalid group signalled"))
                .is_err());
        }
    }

    #[test]
    fn mp11_cleanup_retains_detached_descendants_after_a_group_signal() {
        let rows = vec![
            row(41, 9, 41, "root"),
            row(45, 41, 45, "detached"),
            row(99, 1, 99, "foreign"),
        ];
        let mut guard = OwnedProcessSignals {
            root: 41,
            exclusive_session: None,
            owned: BTreeMap::from([(41, "root".into())]),
        };
        let mut sent = Vec::new();
        guard
            .group_with(&rows, |pid, signal| {
                sent.push((pid, signal));
                Ok(())
            })
            .unwrap();
        guard
            .kill_owned_with(
                || Ok(rows.clone()),
                |pid, signal| {
                    sent.push((pid, signal));
                    Ok(())
                },
            )
            .unwrap();
        use Signal::{Kill, Stop};
        assert_eq!(
            sent,
            [(-41, Kill), (41, Stop), (45, Stop), (41, Kill), (45, Kill)]
        );
    }

    #[test]
    fn mp11_signal_guard_refuses_reused_and_foreign_members() {
        let mut guard = OwnedProcessSignals {
            root: 41,
            exclusive_session: None,
            owned: BTreeMap::from([(41, "original".into())]),
        };
        let mut sends = 0;
        assert!(guard
            .group_with(&[row(41, 1, 41, "reused")], |_, _| {
                sends += 1;
                Ok(())
            })
            .is_err());
        assert!(guard
            .group_with(
                &[row(41, 1, 41, "original"), row(42, 1, 41, "foreign")],
                |_, _| {
                    sends += 1;
                    Ok(())
                }
            )
            .is_err());
        assert_eq!(sends, 0);
        assert!(guard
            .group_with(
                &[row(41, 1, 41, "original"), row(43, 41, 41, "child")],
                |pid, _| {
                    assert_eq!(pid, -41);
                    sends += 1;
                    Ok(())
                }
            )
            .unwrap());
        assert_eq!(sends, 1);
        // Once reparented, only the recorded child birth is authorized.
        assert!(guard
            .group_with(&[row(43, 1, 41, "child")], |_, _| Ok(()))
            .unwrap());
        assert!(guard
            .group_with(&[row(43, 1, 41, "replacement")], |_, _| panic!(
                "reused child signalled"
            ))
            .is_err());
    }
}

#[cfg(all(test, unix))]
#[test]
fn mp11_signal_seam_rejects_system_wide_targets_without_signalling() {
    for pid in [i32::MIN, -1, 0, 1] {
        assert_eq!(
            send_signal(pid, Signal::Kill).unwrap_err().to_string(),
            "invalid process signal target"
        );
    }
}
