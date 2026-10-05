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
    owned: BTreeMap<u32, String>,
}

impl OwnedProcessSignals {
    pub(crate) fn for_child(child: &Child) -> io::Result<Self> {
        let rows = snapshot()?;
        let pid = child.id();
        if pid <= 1 || pid > i32::MAX as u32 {
            return Err(io::Error::other("invalid child PID"));
        }
        let identity = rows
            .iter()
            .find(|row| row.pid == pid)
            .ok_or_else(|| io::Error::other("child start identity unavailable"))?;
        let mut guard = Self {
            root: pid,
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
                                    && root.session == self.root
                                    && row.session == root.session
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
        self.group_with(&snapshot()?, |pid| send_signal(pid))
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
        .args(["-axo", "pid=,ppid=,pgid=,lstart="])
        .env("LC_ALL", "C")
        .output()?;
    if !result.status.success() {
        return Err(io::Error::other("process metadata unavailable"));
    }
    String::from_utf8(result.stdout)
        .map_err(io::Error::other)?
        .lines()
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 4 {
                return Err(io::Error::other("short process metadata"));
            }
            let parse = |field: &str| {
                field
                    .parse()
                    .map_err(|_| io::Error::other("invalid process ID"))
            };
            Ok(ProcessIdentity {
                pid: parse(fields[0])?,
                parent: parse(fields[1])?,
                group: parse(fields[2])?,
                session: 0,
                start: fields[3..].join(" "),
            })
        })
        .collect()
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
    fn mp11_signal_guard_refuses_reused_and_foreign_members() {
        let mut guard = OwnedProcessSignals {
            root: 41,
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
