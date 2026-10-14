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

    fn capture(child: &Child, launched_exclusive_session: bool) -> io::Result<Self> {
        let rows = snapshot()?;
        let pid = child.id();
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
        send: impl FnOnce(i32) -> io::Result<()>,
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
        send(-(self.root as i32))?;
        Ok(true)
    }

    pub(crate) fn kill_group(&mut self) -> io::Result<bool> {
        self.group_with(&snapshot()?, send_signal)
    }

    pub(crate) fn kill_owned_processes(&mut self) -> io::Result<()> {
        self.kill_owned_with(snapshot, send_signal)
    }

    fn kill_owned_with(
        &mut self,
        mut inspect: impl FnMut() -> io::Result<Vec<ProcessIdentity>>,
        mut send: impl FnMut(i32) -> io::Result<()>,
    ) -> io::Result<()> {
        self.refresh_from(&inspect()?);
        let pids: Vec<_> = self.owned.keys().copied().collect();
        for pid in pids.into_iter().rev() {
            // Recheck each birth immediately before its individual signal.
            let rows = inspect()?;
            if rows.iter().any(|row| row.pid == pid && self.matches(row)) {
                send(pid as i32)?;
            }
        }
        Ok(())
    }

    pub(crate) fn kill_child(&mut self) -> io::Result<bool> {
        let rows = snapshot()?;
        let Some(row) = rows.iter().find(|row| row.pid == self.root) else {
            return Ok(false);
        };
        if !self.matches(row) {
            return Err(io::Error::other("child PID was reused"));
        }
        send_signal(self.root as i32)?;
        Ok(true)
    }
}

#[cfg(unix)]
fn send_signal(pid: i32) -> io::Result<()> {
    // MP-11: reject system-wide targets even if a future caller loses its witness.
    if pid == i32::MIN || (-1..=1).contains(&pid) {
        return Err(io::Error::other("invalid process signal target"));
    }
    // Identity and complete group membership are checked immediately before this sole seam.
    if unsafe { libc::kill(pid, libc::SIGKILL) } == 0 {
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
fn send_signal(_: i32) -> io::Result<()> {
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
        let text = match std::fs::read_to_string(entry.path().join("stat")) {
            Ok(text) => text,
            Err(error) if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => {
                continue
            }
            Err(error) => return Err(error),
        };
        let Some((_, tail)) = text.rsplit_once(") ") else {
            return Err(io::Error::other("invalid process identity"));
        };
        let fields: Vec<_> = tail.split_whitespace().collect();
        if fields.len() < 20 {
            return Err(io::Error::other("short process identity"));
        }
        let parse = |field: &str| {
            field
                .parse()
                .map_err(|_| io::Error::other("invalid process ID"))
        };
        rows.push(ProcessIdentity {
            pid,
            parent: parse(fields[1])?,
            group: parse(fields[2])?,
            session: parse(fields[3])?,
            start: fields[19].into(),
        });
    }
    Ok(rows)
}
#[cfg(target_os = "macos")]
fn snapshot() -> io::Result<Vec<ProcessIdentity>> {
    let result = std::process::Command::new("/bin/ps")
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
            .group_with(&rows, |pid| {
                assert_eq!(pid, -41);
                Ok(())
            })
            .unwrap());
        let mut foreign = row(43, 1, 41, "foreign-birth");
        foreign.session = 99;
        assert!(guard
            .group_with(&[rows[0].clone(), foreign], |_| panic!(
                "foreign session signalled"
            ))
            .is_err());
        let mut lost = OwnedProcessSignals {
            root: 41,
            exclusive_session: Some(41),
            owned: BTreeMap::from([(41, rows[0].start.clone())]),
        };
        assert!(lost
            .group_with(&rows[1..], |_| panic!("missing leader witness signalled"))
            .is_err());
        let mut replacement = rows[0].clone();
        replacement.start = "replacement-leader".into();
        assert!(lost
            .group_with(&[replacement, rows[1].clone()], |_| panic!(
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
            .group_with(&rows, |_| panic!("mixed group signalled"))
            .is_err());
        let mut signalled = Vec::new();
        guard
            .kill_owned_with(
                || Ok(rows.clone()),
                |pid| {
                    signalled.push(pid);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(signalled, [43, 41]);
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
            .group_with(&[row(41, 1, 41, "reused")], |_| {
                sends += 1;
                Ok(())
            })
            .is_err());
        assert!(guard
            .group_with(
                &[row(41, 1, 41, "original"), row(42, 1, 41, "foreign")],
                |_| {
                    sends += 1;
                    Ok(())
                }
            )
            .is_err());
        assert_eq!(sends, 0);
        assert!(guard
            .group_with(
                &[row(41, 1, 41, "original"), row(43, 41, 41, "child")],
                |pid| {
                    assert_eq!(pid, -41);
                    sends += 1;
                    Ok(())
                }
            )
            .unwrap());
        assert_eq!(sends, 1);
        // Once reparented, only the recorded child birth is authorized.
        assert!(guard
            .group_with(&[row(43, 1, 41, "child")], |_| Ok(()))
            .unwrap());
        assert!(guard
            .group_with(&[row(43, 1, 41, "replacement")], |_| panic!(
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
            send_signal(pid).unwrap_err().to_string(),
            "invalid process signal target"
        );
    }
}
