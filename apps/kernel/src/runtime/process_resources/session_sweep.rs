//! Background jobs of a terminal's shell or a provider's tools live in their own
//! process groups, so stopping the session leader and its foreground group
//! leaves them behind. Everything still in the leader's session ends with it,
//! by exact PID.

const DETACH_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// Who belongs to a terminal's shell, captured while the shell still runs.
#[derive(Clone, Copy)]
pub(crate) struct SessionSweep {
    leader: i32,
}

impl SessionSweep {
    pub(crate) fn capture(session_leader: i32) -> Self {
        Self {
            leader: session_leader,
        }
    }

    pub(crate) fn stop(&self) {
        for pid in self.live_members() {
            // SAFETY: a positive PID selected moments ago; never a group.
            let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }

    /// Ends the session and the leader's live process tree, which may hold
    /// commands in sessions of their own (a provider's tool commands). A job
    /// whose parent already exited has left both and keeps running.
    pub(crate) fn stop_with_descendants(&self) {
        let mut members = descendants(self.leader);
        members.extend(self.live_members());
        for pid in members {
            // SAFETY: a positive PID selected moments ago; never a group.
            let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }

    /// Ends the session a moment from now, so a job that is detaching
    /// (`setsid job &`) as its shell or command returns can leave it first.
    pub(crate) fn stop_soon(self) {
        std::thread::spawn(move || {
            std::thread::sleep(DETACH_GRACE);
            self.stop();
        });
    }

    /// Whether anything but the shell itself still runs in the session: a
    /// background job keeps running after the prompt returns.
    pub(crate) fn has_jobs(&self) -> bool {
        self.live_members().iter().any(|pid| *pid != self.leader)
    }

    fn live_members(&self) -> Vec<i32> {
        #[cfg(target_os = "linux")]
        return proc_session_members(self.leader);
        #[cfg(not(target_os = "linux"))]
        return ps_session_members(self.leader);
    }
}

/// The leader's running descendants, from one reading of parent links so
/// nothing reparents mid-walk.
fn descendants(leader: i32) -> Vec<i32> {
    if leader <= 1 {
        return Vec::new();
    }
    let parents = parent_links();
    let mut found = std::collections::BTreeSet::from([leader]);
    loop {
        let before = found.len();
        for (pid, parent) in &parents {
            if found.contains(parent) {
                found.insert(*pid);
            }
        }
        if found.len() == before {
            break;
        }
    }
    found.remove(&leader);
    found.into_iter().filter(|pid| *pid > 1).collect()
}

#[cfg(target_os = "linux")]
fn parent_links() -> Vec<(i32, i32)> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter_map(|pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let mut fields = stat[stat.rfind(')')? + 1..].split_whitespace();
            (!fields.next()?.starts_with('Z')).then_some(())?;
            Some((pid, fields.next()?.parse().ok()?))
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn parent_links() -> Vec<(i32, i32)> {
    let Ok(output) = crate::process_spawn::command("ps")
        .args(["-A", "-o", "pid=,ppid="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            Some((columns.next()?.parse().ok()?, columns.next()?.parse().ok()?))
        })
        .collect()
}

/// The running processes of the session led by `session_leader`.
#[cfg(target_os = "linux")]
fn proc_session_members(session_leader: i32) -> Vec<i32> {
    if session_leader <= 1 {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
                .filter(|pid| *pid > 1)
        })
        .filter(|pid| session_of(*pid) == Some(session_leader))
        .collect()
}

// A session ID outlives its controlling terminal and leader. A tty selector
// would miss disowned jobs after the shell exits and detaches that terminal.
#[cfg(any(not(target_os = "linux"), test))]
fn ps_session_members(session_leader: i32) -> Vec<i32> {
    if session_leader <= 1 {
        return Vec::new();
    }
    let Ok(output) = crate::process_spawn::command("ps")
        .args(["-A", "-o", "pid=,stat="])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    session_members(
        &String::from_utf8_lossy(&output.stdout),
        session_leader,
        |pid| {
            // SAFETY: query a positive PID; this does not signal it.
            let session = unsafe { libc::getsid(pid) };
            (session > 1).then_some(session)
        },
    )
}

#[cfg(any(not(target_os = "linux"), test))]
fn session_members(
    processes: &str,
    leader: i32,
    session_of: impl Fn(i32) -> Option<i32>,
) -> Vec<i32> {
    if leader <= 1 {
        return Vec::new();
    }
    processes
        .lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let pid = columns.next()?.parse::<i32>().ok()?;
            let zombie = columns.next().is_some_and(|stat| stat.starts_with('Z'));
            (!zombie).then_some(pid)
        })
        .filter(|pid| *pid > 1 && session_of(*pid) == Some(leader))
        .collect()
}

/// Field 6 of `/proc/<pid>/stat` (none for a zombie); the command name
/// (field 2) may hold spaces and parentheses, so fields are counted after its
/// closing parenthesis.
#[cfg(target_os = "linux")]
fn session_of(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_command = &stat[stat.rfind(')')? + 1..];
    let mut fields = after_command.split_whitespace();
    // A zombie runs nothing; it only waits to be reaped.
    if fields.next()?.starts_with('Z') {
        return None;
    }
    fields.nth(2)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_sweep_stops_a_job_after_its_session_leader_exits() {
        use std::os::unix::process::CommandExt;
        use std::time::{Duration, Instant};
        let root =
            std::env::temp_dir().join(format!("terminal-detached-job-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("job.pid");
        let mut command = crate::process_spawn::command("/bin/bash");
        command
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "nohup sleep 8884 >/dev/null 2>&1 & echo $! > \"$1\"",
                "bash",
            ])
            .arg(&file);
        // SAFETY: setsid is async-signal-safe and touches only this child.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut shell = command.spawn().unwrap();
        let leader = i32::try_from(shell.id()).unwrap();
        assert!(shell.wait().unwrap().success());
        let pid = std::fs::read_to_string(&file)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        assert!(pid > 1 && leader > 1);
        let alive = || {
            crate::process_spawn::command("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .is_ok_and(|output| {
                    let status = String::from_utf8_lossy(&output.stdout);
                    !status.trim().is_empty() && !status.trim().starts_with('Z')
                })
        };
        assert!(alive());
        for pid in ps_session_members(leader) {
            let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let survived = alive();
        // Exact-PID cleanup also runs if selection was broken (the RED case).
        if survived && pid > 1 {
            let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            !survived,
            "the detached job survived its terminal session sweep"
        );
    }

    #[test]
    fn detached_jobs_are_selected_by_session_without_the_leader_or_a_tty() {
        let listed = " 0 S\n1 Ss\n-1 S\nnot-a-pid S\n42 S\n43 S\n44 S\n45 Z\n";
        // Leader 40 has already exited; job 42 has no controlling tty.
        let lookup = |pid| match pid {
            42 => Some(40),
            43 => Some(43),
            45 => Some(40),
            _ => None,
        };
        assert_eq!(session_members(listed, 40, lookup), vec![42]);
        for invalid in [-1, 0, 1] {
            assert!(session_members(listed, invalid, lookup).is_empty());
        }
    }
}
